//! Translate the frozen wire operations into typed engine operations.
use super::{engine_error, error, ApiResult};
use crate::{
    api::*,
    daemon::Daemon,
    engine::{
        self,
        ops::{EngineOp, EngineResult, HashSelection},
    },
};
use serde_json::json;
use std::{
    collections::HashSet, fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path,
};

fn execute(daemon: &Daemon, op: EngineOp) -> ApiResult<EngineResult> {
    let hash = match &op {
        EngineOp::Get { hash }
        | EngineOp::Files { hash }
        | EngineOp::OpenTarget { hash }
        | EngineOp::SetFiles { hash, .. } => Some(hash.clone()),
        _ => None,
    };
    daemon.execute(op).map_err(|value| {
        let mut failure = engine_error(value);
        if failure.code == ErrorCode::NotFound {
            if let Some(hash) = hash {
                failure.details = Some(serde_json::Map::from_iter([("hash".into(), json!(hash))]));
            }
        }
        failure
    })
}
fn hashes(values: &Hashes) -> HashSelection {
    HashSelection::Hashes(values.as_slice().iter().map(ToString::to_string).collect())
}
pub fn torrent(view: engine::view::TorrentView) -> ApiResult<Torrent> {
    let legacy: serde_json::Value =
        serde_json::from_str(&view.to_legacy_json().map_err(engine_error)?).unwrap();
    let mut value = serde_json::to_value(view).unwrap();
    if value["upload_limit_bps"] == 0 {
        value.as_object_mut().unwrap().remove("upload_limit_bps");
    }
    if let Some(origin) = legacy["tags"].as_str().and_then(|tags| {
        tags.split(',')
            .find_map(|tag| tag.strip_prefix("fbt-origin:"))
    }) {
        value["origin"] = json!(origin);
    }
    serde_json::from_value(value)
        .map_err(|_| error(ErrorCode::Internal, "invalid normalized torrent", None))
}
pub fn list(daemon: &Daemon) -> ApiResult<Vec<Torrent>> {
    let EngineResult::Torrents(rows) = execute(daemon, EngineOp::Info)? else {
        unreachable!("typed info result")
    };
    rows.into_iter().map(torrent).collect()
}
fn files(daemon: &Daemon, hash: &Hash) -> ApiResult<Vec<File>> {
    let EngineResult::Files(rows) = execute(
        daemon,
        EngineOp::Files {
            hash: hash.to_string(),
        },
    )?
    else {
        unreachable!("typed file result")
    };
    Ok(rows
        .into_iter()
        .map(|file| File {
            index: file.index as u64,
            path: file.name.clone(),
            size: file.size.max(0) as u64,
            progress: file.progress,
            included: file.priority != 0,
            sort_key: sort_key(&file.name),
            ready: None,
        })
        .collect())
}
pub fn sort_key(path: &str) -> String {
    let mut result = String::new();
    let lowered = path.to_lowercase();
    let mut chars = lowered.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            let mut digits = String::from(ch);
            while chars.peek().is_some_and(char::is_ascii_digit) {
                digits.push(chars.next().unwrap());
            }
            for _ in digits.len()..10 {
                result.push('0');
            }
            result.push_str(&digits);
        } else {
            result.push(ch);
        }
    }
    result
}
fn read_torrent(path: &str) -> ApiResult<(String, Vec<u8>)> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| {
            error(
                ErrorCode::InvalidArgs,
                "torrent path must be an accessible regular file without symlinks",
                Some("source.path"),
            )
        })?;
    let metadata = file.metadata().map_err(|_| {
        error(
            ErrorCode::InvalidArgs,
            "cannot inspect torrent path",
            Some("source.path"),
        )
    })?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(error(
            ErrorCode::InvalidArgs,
            "torrent must be a regular file at most 16 MiB",
            Some("source.path"),
        ));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            error(
                ErrorCode::InvalidArgs,
                "cannot read torrent file",
                Some("source.path"),
            )
        })?;
    if bytes.len() > 16 * 1024 * 1024
        || bytes.first() != Some(&b'd')
        || librqbit::torrent_from_bytes(&bytes).is_err()
    {
        return Err(error(
            ErrorCode::InvalidArgs,
            "torrent must be valid bencoded metainfo at most 16 MiB",
            Some("source.path"),
        ));
    }
    Ok((
        Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        bytes,
    ))
}

pub fn dispatch(daemon: &Daemon, call: &Call) -> ApiResult<Reply> {
    use ErrorCode::*;
    Ok(match call {
        Call::TorrentsList { args } => {
            let mut rows = list(daemon)?;
            rows.retain(|row| {
                args.states
                    .as_ref()
                    .is_none_or(|states| states.contains(&row.state))
                    && args.origin.as_ref().is_none_or(|origins| {
                        row.origin
                            .as_ref()
                            .is_some_and(|origin| origins.contains(origin))
                    })
            });
            Reply::TorrentsList(ListResult { torrents: rows })
        }
        Call::TorrentsGet { args } => {
            let EngineResult::Detail(detail) = execute(
                daemon,
                EngineOp::Get {
                    hash: args.hash.to_string(),
                },
            )?
            else {
                unreachable!("typed detail result")
            };
            Reply::TorrentsGet(GetResult {
                torrent: TorrentDetail {
                    torrent: torrent(detail.torrent)?,
                    private: detail.private,
                    file_count: detail.file_count,
                    piece_count: detail.piece_count,
                    piece_size: detail.piece_size,
                    tracker_count: detail.tracker_count,
                    comment: detail.comment,
                    created_by: detail.created_by,
                    undo_until: None,
                },
            })
        }
        Call::TorrentsFiles { args } => Reply::TorrentsFiles(FilesResult {
            files: files(daemon, &args.hash)?,
        }),
        Call::TorrentsAdd { args } => {
            if args.stage.is_some() {
                return Err(error(
                    InvalidArgs,
                    "staging is not implemented",
                    Some("stage"),
                ));
            }
            let origin = args.origin.as_deref().unwrap_or("manual");
            if !matches!(origin, "manual" | "handler" | "agent") {
                return Err(error(InvalidArgs, "unsupported origin", Some("origin")));
            }
            let seed = args.seed.unwrap_or(
                crate::config::Config::load()
                    .map_err(engine_error)?
                    .seed_by_default,
            );
            let mut add = engine::ops::AddRequest {
                sources: Vec::new(),
                torrent: None,
                save_path: args.save_path.clone(),
                upload_limit: if seed { 0 } else { 32768 },
                ratio_limit: if seed { -2.0 } else { 0.0 },
                stopped: args.stopped.unwrap_or(false),
                category: args.category.clone().unwrap_or_default(),
                tags: format!("fbt-origin:{origin}"),
            };
            let magnet = match &args.source {
                AddSource::Restore(_) => {
                    return Err(error(
                        InvalidArgs,
                        "restore is not implemented",
                        Some("source.restore"),
                    ))
                }
                AddSource::Path(path) => {
                    add.torrent = Some(read_torrent(path)?);
                    false
                }
                AddSource::Url(url) => {
                    let parsed = url::Url::parse(url)
                        .map_err(|_| error(InvalidArgs, "invalid URL", Some("source.url")))?;
                    if !matches!(parsed.scheme(), "http" | "https") {
                        return Err(error(
                            InvalidArgs,
                            "expected http or https",
                            Some("source.url"),
                        ));
                    }
                    add.sources.push(url.clone());
                    false
                }
                AddSource::Magnet(source) => {
                    let mut source = url::Url::parse(source)
                        .map_err(|_| error(InvalidArgs, "invalid magnet", Some("source.magnet")))?;
                    if let Some(files) = &args.files {
                        source.query_pairs_mut().append_pair(
                            "so",
                            &files
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                    let parsed = librqbit::Magnet::parse(source.as_str()).map_err(|_| {
                        error(InvalidArgs, "invalid v1 magnet", Some("source.magnet"))
                    })?;
                    if parsed.as_id20().is_none() {
                        return Err(error(
                            InvalidArgs,
                            "a v1 magnet hash is required",
                            Some("source.magnet"),
                        ));
                    }
                    add.sources.push(source.to_string());
                    true
                }
            };
            if let (Some(indices), Some((_, bytes))) = (&args.files, &add.torrent) {
                let metadata = librqbit::torrent_from_bytes(bytes)
                    .map_err(|value| engine_error(value.into()))?;
                let count = metadata
                    .info
                    .data
                    .validate()
                    .map_err(|value| engine_error(value.into()))?
                    .iter_file_details()
                    .count();
                if indices.iter().any(|index| *index >= count as u64) {
                    return Err(error(
                        InvalidArgs,
                        "file index is out of range",
                        Some("files"),
                    ));
                }
            }
            let EngineResult::Added(mut added) = execute(
                daemon,
                if magnet {
                    EngineOp::AddMagnet(add)
                } else if let Some(files) = &args.files {
                    EngineOp::AddSelected {
                        options: add,
                        files: files.iter().map(|index| *index as usize).collect(),
                    }
                } else {
                    EngineOp::Add(add)
                },
            )?
            else {
                unreachable!("typed add result")
            };
            let added = added
                .pop()
                .ok_or_else(|| error(Internal, "engine returned no add result", None))?;
            Reply::TorrentsAdd(AddResult {
                status: MutationResult::Done,
                hash: Some(Hash::new(added.hash).unwrap()),
                name: Some(added.name),
                duplicate: Some(added.duplicate),
                deferred: None,
            })
        }
        Call::TorrentsStart { args } => {
            if args.force.is_some() {
                return Err(error(
                    InvalidArgs,
                    "force is not implemented",
                    Some("force"),
                ));
            }
            // A read of already-durable run intent is a no-op. Do not claim
            // that another start changed it; failed restores still retry.
            let rows = list(daemon)?;
            let mut seen = HashSet::new();
            let requested: Vec<_> = args
                .hashes
                .as_slice()
                .iter()
                .filter(|hash| seen.insert(hash.to_string()))
                .collect();
            for hash in &requested {
                if !rows.iter().any(|row| &row.hash == *hash) {
                    let mut failure = error(NotFound, "torrent record not found", None);
                    failure.details =
                        Some(serde_json::Map::from_iter([("hash".into(), json!(hash))]));
                    return Err(failure);
                }
            }
            let mut started = Vec::new();
            let mut skipped = Vec::new();
            let mut wanted = Vec::new();
            for hash in &requested {
                let row = rows.iter().find(|row| &row.hash == *hash).unwrap();
                if !row.seed && (row.completed_at.is_some() || row.state == TorrentState::Done) {
                    if requested.len() == 1 {
                        let mut failure = error(InvalidState, "invalid state", None);
                        failure.hint = Some("inspect fbittorrent log".into());
                        failure.details = Some(serde_json::Map::from_iter([
                            ("state".into(), json!(row.state)),
                            ("reason".into(), json!("complete_no_seed")),
                            ("hash".into(), json!(hash)),
                        ]));
                        return Err(failure);
                    }
                    skipped.push(Skipped {
                        hash: (*hash).clone(),
                        reason: "complete_no_seed".into(),
                    });
                } else if row.intent == Intent::Run && row.state != TorrentState::Error {
                    skipped.push(Skipped {
                        hash: (*hash).clone(),
                        reason: "already_running".into(),
                    });
                } else {
                    wanted.push(hash.to_string());
                }
            }
            if !wanted.is_empty() {
                let EngineResult::Start(result) = execute(
                    daemon,
                    EngineOp::Start {
                        hashes: HashSelection::Hashes(wanted),
                        skip_complete: requested.len() > 1,
                    },
                )?
                else {
                    unreachable!("typed start result")
                };
                started.extend(
                    result
                        .started
                        .into_iter()
                        .map(|hash| Hash::new(hash).unwrap()),
                );
                skipped.extend(result.skipped.into_iter().map(|entry| Skipped {
                    hash: Hash::new(entry.hash).unwrap(),
                    reason: entry.reason,
                }));
            }
            Reply::TorrentsStart(StartResult {
                state: DoneState::Done,
                started,
                skipped,
            })
        }
        Call::TorrentsStop { args } => {
            execute(
                daemon,
                EngineOp::Stop {
                    hashes: hashes(&args.hashes),
                },
            )?;
            Reply::TorrentsStop(StopResult {
                state: DoneState::Done,
                stopped: args.hashes.as_slice().to_vec(),
            })
        }
        Call::TorrentsRemove { args } => {
            execute(
                daemon,
                EngineOp::Delete {
                    hashes: hashes(&args.hashes),
                    files: args.delete_files,
                },
            )?;
            Reply::TorrentsRemove(RemoveResult {
                status: MutationResult::Done,
                undo_until: None,
            })
        }
        Call::TorrentsSetFiles { args } => {
            let all = files(daemon, &args.hash)?;
            let mut include: Vec<usize> = all
                .iter()
                .filter(|f| f.included)
                .map(|f| f.index as usize)
                .collect();
            for index in args
                .include
                .iter()
                .flatten()
                .chain(args.skip.iter().flatten())
            {
                if *index >= all.len() as u64 {
                    return Err(error(
                        InvalidArgs,
                        "file index is out of range",
                        Some("include/skip"),
                    ));
                }
            }
            if let Some(indices) = &args.include {
                include.extend(indices.iter().map(|i| *i as usize));
            }
            if let Some(skip) = &args.skip {
                include.retain(|i| !skip.contains(&(*i as u64)));
            }
            include.sort_unstable();
            include.dedup();
            execute(
                daemon,
                EngineOp::SetFiles {
                    hash: args.hash.to_string(),
                    include,
                },
            )?;
            Reply::TorrentsSetFiles(DoneResult {
                state: DoneState::Done,
            })
        }
        Call::TorrentsSetSeed { args } => {
            execute(
                daemon,
                EngineOp::SetSeed {
                    hashes: hashes(&args.hashes),
                    on: args.on,
                },
            )?;
            Reply::TorrentsSetSeed(DoneResult {
                state: DoneState::Done,
            })
        }
        Call::TorrentsSetUploadLimit { args } => {
            match args.bps {
                Update::Unset => {}
                Update::Clear => {
                    execute(
                        daemon,
                        EngineOp::SetUploadLimit {
                            hashes: hashes(&args.hashes),
                            limit: 0,
                        },
                    )?;
                }
                Update::Set(bps) => {
                    let limit = i64::try_from(bps).map_err(|_| {
                        error(
                            InvalidArgs,
                            "upload limit exceeds engine range",
                            Some("bps"),
                        )
                    })?;
                    execute(
                        daemon,
                        EngineOp::SetUploadLimit {
                            hashes: hashes(&args.hashes),
                            limit,
                        },
                    )?;
                }
            }
            Reply::TorrentsSetUploadLimit(MutationResult::Done)
        }
        Call::TorrentsOpenTarget { args } => {
            let EngineResult::OpenTarget(mut path) = execute(
                daemon,
                EngineOp::OpenTarget {
                    hash: args.hash.to_string(),
                },
            )?
            else {
                unreachable!("typed target result")
            };
            if let Some(index) = args.file {
                let entries = files(daemon, &args.hash)?;
                let entry = entries.get(index as usize).ok_or_else(|| {
                    error(InvalidArgs, "file index is out of range", Some("file"))
                })?;
                let row = list(daemon)?
                    .into_iter()
                    .find(|t| t.hash == args.hash)
                    .ok_or_else(|| error(NotFound, "torrent not found", None))?;
                path = Path::new(&row.save_path).join(&entry.path);
                if !path.is_file() {
                    return Err(error(InvalidState, "downloaded file is missing", None));
                }
            }
            let kind = if path.is_dir() {
                TargetKind::Folder
            } else {
                TargetKind::File
            };
            Reply::TorrentsOpenTarget(OpenTargetResult {
                path: path.to_string_lossy().into(),
                kind,
            })
        }
        _ => {
            return Err(error(
                Unsupported,
                "operation is not implemented in this phase",
                None,
            ))
        }
    })
}

/// Validate before acknowledging a URL task. The actual download stays in the
/// bounded dispatch pool and uses the engine's interface-bound HTTP client.
pub fn validate_url_add(args: &AddArgs) -> ApiResult<()> {
    if args.stage.is_some() {
        return Err(error(
            ErrorCode::InvalidArgs,
            "staging is not implemented",
            Some("stage"),
        ));
    }
    if args
        .origin
        .as_deref()
        .is_some_and(|value| !matches!(value, "manual" | "handler" | "agent"))
    {
        return Err(error(
            ErrorCode::InvalidArgs,
            "unsupported origin",
            Some("origin"),
        ));
    }
    let AddSource::Url(source) = &args.source else {
        return Err(error(
            ErrorCode::InvalidArgs,
            "expected URL source",
            Some("source"),
        ));
    };
    let url = url::Url::parse(source)
        .map_err(|_| error(ErrorCode::InvalidArgs, "invalid URL", Some("source.url")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(
            ErrorCode::InvalidArgs,
            "expected http or https",
            Some("source.url"),
        ));
    }
    Ok(())
}
