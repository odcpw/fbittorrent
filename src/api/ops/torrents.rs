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
        | EngineOp::EditTracker { hash, .. }
        | EngineOp::Commit {hash, ..}
        | EngineOp::Discard {hash}
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
    if value.get("origin").is_none() {
    if let Some(origin) = legacy["tags"].as_str().and_then(|tags| {
        tags.split(',')
            .find_map(|tag| tag.strip_prefix("fbt-origin:"))
    }) {
        value["origin"] = json!(origin);
    } else { value["origin"]=json!("manual"); }
    }
    serde_json::from_value(value)
        .map_err(|_| error(ErrorCode::Internal, "invalid normalized torrent", None))
}
pub fn list(daemon: &Daemon) -> ApiResult<Vec<Torrent>> {
    let EngineResult::Torrents(rows) = execute(daemon, EngineOp::Info)? else {
        unreachable!("typed info result")
    };
    let mut rows=rows.into_iter().map(torrent).collect::<ApiResult<Vec<_>>>()?;
    daemon.watch.lock().unwrap().decorate(&mut rows);
    Ok(rows)
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
            ready: daemon.watch.lock().unwrap().file_ready(hash.as_str(),file.index as u64),
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
        Call::TorrentsCommit {args} => return crate::daemon::staged_sources::commit(daemon,&args.hash,|| {
            if args.save_path.as_deref().is_some_and(|path| !Path::new(path).is_absolute()) {
                return Err(error(InvalidArgs,"save_path must be absolute",Some("save_path")));
            }
            if let Some(indices)=&args.files {
                let entries=files(daemon,&args.hash)?;
                if indices.iter().any(|index|*index>=entries.len() as u64) {return Err(error(InvalidArgs,"file index is out of range",Some("files")));}
            }
            execute(daemon,EngineOp::Commit {hash:args.hash.to_string(),
                files:args.files.as_ref().map(|values|values.iter().map(|v|*v as usize).collect()),
                seed:args.seed,save_path:args.save_path.as_ref().map(Into::into)})?;
            Ok(Reply::TorrentsCommit(DoneResult {state:DoneState::Done}))
        }),
        Call::TorrentsDiscard {args} => return crate::daemon::staged_sources::discard(daemon,&args.hash,|| {
            execute(daemon,EngineOp::Discard {hash:args.hash.to_string()})?;
            Ok(Reply::TorrentsDiscard(DoneResult {state:DoneState::Done}))
        }),
        Call::ImdbLookup { args } => return super::imdb::lookup(&daemon.imdb, args),
        Call::TorrentsList { args } => {
            let mut rows = list(daemon)?;
            rows.retain(|row| {
                args.states
                    .as_ref()
                    .is_none_or(|states| states.contains(&row.state))
                    && args.origin.as_ref().is_none_or(|origins| {
                        row.origin
                            .as_ref()
                            .is_some_and(|origin| origins.iter().any(|wanted| wanted==origin || (wanted=="manual" && !matches!(origin.as_str(),"search"|"rss"|"shows"))))
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
            let mut rows=vec![torrent(detail.torrent)?];
            daemon.watch.lock().unwrap().decorate(&mut rows);
            Reply::TorrentsGet(GetResult {
                torrent: TorrentDetail {
                    torrent: rows.remove(0),
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
        Call::TorrentsAddTracker {args} | Call::TorrentsRemoveTracker {args} => {
            execute(daemon,EngineOp::EditTracker{hash:args.hash.to_string(),url:args.url.clone(),add:matches!(call,Call::TorrentsAddTracker{..})})?;
            if matches!(call,Call::TorrentsAddTracker{..}) {Reply::TorrentsAddTracker(DoneResult{state:DoneState::Done})}
            else {Reply::TorrentsRemoveTracker(DoneResult{state:DoneState::Done})}
        },
        Call::TorrentsPeers { args } => Reply::TorrentsPeers(PeersResult { peers: daemon.peers.read(daemon, &args.hash)? }),
        Call::TorrentsFiles { args } => Reply::TorrentsFiles(FilesResult {
            files: files(daemon, &args.hash)?,
        }),
        Call::TorrentsAdd { args } => {
            if args.stage==Some(true) && matches!(&args.source,AddSource::Restore(_)) {
                return Err(error(InvalidArgs,"restore cannot be staged",Some("stage")));
            }
            if let AddSource::Restore(source)=&args.source {
                let EngineResult::Added(mut added)=execute(daemon,EngineOp::RestoreUndo {hash:source.as_str().into()})? else {unreachable!("restore result")};
                let added=added.pop().ok_or_else(||error(Internal,"engine returned no restore result",None))?;
                return Ok(Reply::TorrentsAdd(AddResult {
                    status:MutationResult::Done,hash:Some(Hash::new(added.hash).unwrap()),name:Some(added.name),
                    duplicate:Some(added.duplicate),deferred:added.deferred.then_some(true),disk:None,
                }));
            }
            let origin = args.origin.as_deref().unwrap_or("manual");
            if origin.is_empty() || origin.len()>128 || origin.chars().any(char::is_control) {
                return Err(error(InvalidArgs, "unsupported origin", Some("origin")));
            }
            let cfg = crate::config::Config::load().map_err(engine_error)?;
            let seed = args.seed.unwrap_or(cfg.seed_by_default);
            let mut add = engine::ops::AddRequest {
                sources: Vec::new(),
                torrent: None,
                save_path: args.save_path.clone().or(cfg.save_path.clone()),
                upload_limit: if seed { 0 } else { 32768 },
                ratio_limit: if seed { -2.0 } else { 0.0 },
                stopped: args.stopped.unwrap_or(false),
                category: args.category.clone().unwrap_or_default(),
                tags: format!("fbt-origin:{origin}"), origin: Some(origin.into()),
            };
            let magnet = match &args.source {
                AddSource::Restore(_) => unreachable!("handled before ordinary admission"),
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
                    // Fetch through the live slot's interface-bound runtime,
                    // then admit validated bytes through the usual atomic add.
                    let began = std::time::Instant::now();
                    let dispatcher = loop {
                        if daemon.is_stopping() {
                            return Err(engine_error(anyhow::anyhow!("engine is closing")));
                        }
                        let slot = daemon.slot.read().unwrap();
                        match &slot.slot {
                            crate::daemon::slot::Slot::Running(app) => break app.dispatcher(),
                            crate::daemon::slot::Slot::Starting
                                if began.elapsed() < std::time::Duration::from_secs(60) => {}
                            _ => {
                                return Err(engine_error(anyhow::anyhow!(
                                    "engine_unavailable state={}",
                                    slot.slot.state().name()
                                )))
                            }
                        }
                        drop(slot);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    };
                    let network = daemon.network();
                    let bytes = dispatcher
                        .fetch_api_torrent(url, network.interface.as_deref())
                        .map_err(|value| {
                            let mut failure = engine_error(value);
                            if failure.code == Internal {
                                failure =
                                    error(Upstream, "could not fetch valid torrent metainfo", None);
                            }
                            failure.details = Some(serde_json::Map::from_iter([(
                                "source".into(),
                                json!("url"),
                            )]));
                            failure
                        })?;
                    add.torrent = Some(("URL metadata.torrent".into(), bytes));
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
                    if source
                        .query_pairs()
                        .any(|(key, value)| key == "xt" && value.starts_with("urn:btmh:"))
                        && !source
                            .query_pairs()
                            .any(|(key, value)| key == "xt" && value.starts_with("urn:btih:"))
                    {
                        return Err(error(
                            Unsupported,
                            "v2-only magnets are unsupported",
                            Some("source.magnet"),
                        ));
                    }
                    let parsed = librqbit::Magnet::parse(source.as_str()).map_err(|_| {
                        error(InvalidArgs, "invalid v1 magnet", Some("source.magnet"))
                    })?;
                    if parsed.as_id20().is_none() {
                        return Err(error(
                            Unsupported,
                            "a v1 magnet hash is required",
                            Some("source.magnet"),
                        ));
                    }
                    add.sources.push(source.to_string());
                    true
                }
            };
            let selected = if let Some((_, bytes)) = &add.torrent {
                let metadata = librqbit::torrent_from_bytes(bytes)
                    .map_err(|value| engine_error(value.into()))?;
                let count = metadata
                    .info
                    .data
                    .validate()
                    .map_err(|value| engine_error(value.into()))?
                    .iter_file_details()
                    .count();
                let indices = args.files.clone().unwrap_or_else(|| (0..count as u64).collect());
                if indices.iter().any(|index| *index >= count as u64) {
                    return Err(error(
                        InvalidArgs,
                        "file index is out of range",
                        Some("files"),
                    ));
                }
                Some(indices.into_iter().map(|index| index as usize).collect())
            } else { None };
            let disk = add.torrent.as_ref().and_then(|(_,bytes)| crate::daemon::supervisor::add_warning(Path::new(&add.save_path.clone().unwrap_or_else(||super::settings::view(&cfg).save_path)),bytes,selected.as_deref()));
            let EngineResult::Added(mut added) = execute(
                daemon,
                if args.stage==Some(true) {
                    EngineOp::Stage {options:add,files:selected}
                } else if magnet {
                    EngineOp::AddMagnet(add)
                } else if let Some(files) = selected {
                    EngineOp::AddSelected {
                        options: add,
                        files,
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
                deferred: added.deferred.then_some(true),
                disk,
            })
        }
        Call::TorrentsStart { args } => {
            if args.force==Some(true) {
                return super::queue::start_forced(daemon,args);
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
            for hash in &started { daemon.disk.lock().unwrap().restarted(hash.as_str()); }
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
            if !args.delete_files {
                let EngineResult::RemovedUndo {undo_until}=execute(daemon,EngineOp::RemoveKeep {hashes:hashes(&args.hashes)})? else {unreachable!("remove result")};
                return Ok(Reply::TorrentsRemove(RemoveResult {status:MutationResult::Done,undo_until:Some(undo_until)}));
            }
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
        Call::TorrentsSetDownloadLimit{args}=>{
            let rows=list(daemon)?;
            if let Some(hash)=args.hashes.as_slice().iter().find(|h|!rows.iter().any(|row|&row.hash==*h)){
                let mut failure=error(NotFound,"torrent record not found",None);
                failure.details=Some(serde_json::Map::from_iter([("hash".into(),json!(hash))]));
                return Err(failure);
            }
            if !matches!(args.bps,Update::Unset){execute(daemon,EngineOp::SetDownloadLimit{hashes:hashes(&args.hashes),bps:match args.bps{Update::Set(bps)=>Some(bps),_=>None}})?;}
            Reply::TorrentsSetDownloadLimit(DoneResult{state:DoneState::Done})
        },
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
            if args.file.is_none() {
                if let Some(video)=daemon.watch.lock().unwrap().target(args.hash.as_str()) {path=video;}
            }
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
            daemon.watch.lock().unwrap().acknowledge(Some(args.hash.as_str()));
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
    if args
        .origin
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len()>128 || value.chars().any(char::is_control))
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

#[cfg(test)]
#[test]
fn deferred_add_fixture_roundtrips() {
    crate::api::fixtures::check_group("deferred-add", include_str!("../../../contracts/ipc/v1/deferred-add.ndjson")).unwrap();
    println!("deferred-add fixture: parked hello, done/deferred add and waiting_vpn detail pass the frozen v1 types and bounds");
}

#[cfg(test)]
#[test]
fn expired_undo_maps_to_nonretryable_exit_one() {
    let failure=engine_error(engine::InvalidTorrentState {hash:"a".repeat(40),reason:"undo_expired"}.into());
    assert_eq!(failure.code,ErrorCode::InvalidState);
    assert!(!failure.retryable);
    assert_eq!(failure.details.as_ref().unwrap()["reason"],"undo_expired");
    let error=crate::api::errors::app_error(failure);
    assert_eq!(crate::agent::classify(&error).0 as u8,1);
}
