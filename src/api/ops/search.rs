//! Daemon search operations. Providers never belong to a UI or an engine slot.
use super::{error, ApiResult};
use crate::{
    api::*,
    daemon::Daemon,
    search::{self, service::upstream},
};
use serde_json::json;
use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};

pub fn dispatch(daemon: &Daemon, owner: u64, call: &Call) -> ApiResult<Reply> {
    Ok(match call {
        Call::SearchStart { args } => Reply::SearchStart(SearchStarted {
            search_id: daemon.search.start(owner, args)?,
        }),
        Call::SearchStop { args } => {
            daemon.search.stop(args.search_id)?;
            Reply::SearchStop(DoneResult {
                state: DoneState::Done,
            })
        }
        Call::SearchProviders { .. } => Reply::SearchProviders(ProvidersResult {
            providers: search::plugins()
                .map_err(upstream)?
                .into_iter()
                .map(|p| Provider {
                    name: p.name,
                    kind: ProviderKind::Nova,
                    enabled: p.enabled,
                    full_name: (!p.full_name.is_empty()).then_some(p.full_name),
                    url: (!p.url.is_empty()).then_some(p.url),
                    version: (!p.version.is_empty()).then_some(p.version),
                })
                .collect(),
        }),
        Call::SearchSetProvider { args } => {
            search::set_enabled(&args.name, args.enabled).map_err(upstream)?;
            Reply::SearchSetProvider(DoneResult {
                state: DoneState::Done,
            })
        }
        Call::SearchInstall { args } => {
            let sources = args.sources.clone().unwrap_or_else(|| {
                crate::qbt::COMMUNITY_SEARCH_PLUGINS
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            });
            let mut report = InstalledResult {
                installed: vec![],
                failed: vec![],
            };
            let began = std::time::Instant::now();
            for source in sources {
                if daemon.is_stopping() {
                    return Err(error(ErrorCode::EngineUnavailable, "daemon draining", None));
                }
                let remaining = 80u64.saturating_sub(began.elapsed().as_secs());
                if remaining == 0 {
                    report.failed.push(InstallFailure {
                        source,
                        message: "provider installation budget exhausted".into(),
                    });
                    continue;
                }
                match search::install(std::slice::from_ref(&source), remaining.min(10)) {
                    Ok(()) => report.installed.push(source),
                    Err(e) => report.failed.push(InstallFailure {
                        source,
                        message: format!("{e:#}").chars().take(200).collect(),
                    }),
                }
            }
            Reply::SearchInstall(InstallReply::Complete(report))
        }
        Call::SearchBrowseSources { .. } => Reply::SearchBrowseSources(BrowseSourcesResult {
            sources: crate::browse::catalogue()
                .into_iter()
                .enumerate()
                .map(|(id, e)| BrowseSource {
                    id: id.to_string(),
                    label: e.label.into(),
                    provider: e.provider.into(),
                    in_app: !matches!(e.source, crate::browse::Source::Site(_)),
                    url: if let crate::browse::Source::Site(url) = e.source {
                        Some(url)
                    } else {
                        None
                    },
                })
                .collect(),
        }),
        Call::SearchBrowse { args } => {
            let source = args
                .source
                .parse::<usize>()
                .ok()
                .and_then(|index| crate::browse::catalogue().into_iter().nth(index))
                .ok_or_else(|| {
                    error(ErrorCode::NotFound, "unknown browse source", Some("source"))
                })?;
            let rows = crate::browse::fetch(&source.source).map_err(upstream)?;
            let mut items = search::normalize::aggregate(&rows, 5000);
            mark_added(daemon, &mut items)?;
            Reply::SearchBrowse(SearchItemsResult { items })
        }
        Call::SearchGrab { args } => grab(daemon, args)?,
        _ => unreachable!("search dispatch only"),
    })
}

pub fn mark_added(daemon: &Daemon, items: &mut [SearchItem]) -> ApiResult<()> {
    let rows = super::torrents::list(daemon)?;
    let hashes: std::collections::BTreeSet<_> = rows.into_iter().map(|r| r.hash).collect();
    for item in items {
        item.added = item
            .magnet
            .as_deref()
            .and_then(search::normalize::btih)
            .or_else(|| Hash::new(&item.key).ok().map(|h| h.to_string()))
            .and_then(|h| Hash::new(h).ok())
            .filter(|h| hashes.contains(h));
    }
    Ok(())
}

/// A URL/path already contains metainfo and uses the normal add receipt.
/// Only an unresolved magnet has a later metadata restore outcome to await.
pub fn awaits_metadata(daemon: &Daemon, reply: &Reply) -> bool {
    let Reply::SearchGrab(added) = reply else {
        return false;
    };
    if added.duplicate == Some(true) {
        return false;
    }
    let Some(hash) = &added.hash else {
        return false;
    };
    matches!(daemon.execute(crate::engine::ops::EngineOp::Get { hash: hash.to_string() }),
        Ok(crate::engine::ops::EngineResult::Detail(detail)) if detail.file_count.is_none())
}

pub fn validate_grab(daemon: &Daemon) -> ApiResult<()> {
    // A parked ledger may admit direct magnets elsewhere; a search grab must
    // not suggest a download started when the selected action cannot run.
    if daemon.info()["state"] == "parked" {
        let mut e = error(ErrorCode::EngineUnavailable, "VPN is not ready", None);
        e.details = Some(serde_json::Map::from_iter([(
            "state".into(),
            json!("parked"),
        )]));
        e.hint = Some("connect ProtonVPN before downloading this search result".into());
        return Err(e);
    }
    Ok(())
}

fn grab(daemon: &Daemon, args: &SearchGrabArgs) -> ApiResult<Reply> {
    validate_grab(daemon)?;
    let result = if let Some(result) = &args.result {
        result.clone()
    } else {
        daemon
            .search
            .items(args.search_id.unwrap())?
            .into_iter()
            .find(|r| Some(&r.key) == args.key.as_ref())
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown search result", Some("key")))?
    };
    let legacy = search::normalize::legacy(&result);
    let cfg = crate::config::Config::load().map_err(super::engine_error)?;
    let direct = legacy.file_url.starts_with("magnet:")
        || url::Url::parse(&legacy.file_url).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.path().to_ascii_lowercase().ends_with(".torrent")
        });
    let known_provider = crate::config::data_dir()
        .join("nova3/engines")
        .join(format!("{}.py", legacy.engine))
        .is_file();
    let http =
        url::Url::parse(&legacy.file_url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    let resolved = if cfg.network_interface.is_some() {
        if direct || (!known_provider && http) {
            search::ResolvedDownload::Url(legacy.file_url)
        } else {
            return Err(error(
                ErrorCode::Unsupported,
                "provider download helper cannot use VPN binding; choose a magnet or .torrent URL",
                Some("result"),
            ));
        }
    } else {
        search::resolve_download(&legacy, cfg.timeout_secs).map_err(upstream)?
    };
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut temporary = None;
    let source = match resolved {
        search::ResolvedDownload::Url(url) => {
            if url.starts_with("magnet:") {
                AddSource::Magnet(url)
            } else {
                AddSource::Url(url)
            }
        }
        search::ResolvedDownload::Torrent { bytes, .. } => {
            let path = crate::config::runtime_dir().join(format!(
                "search-{}-{}.torrent",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| super::engine_error(e.into()))?;
            temporary = Some(Temporary(path.clone()));
            file.write_all(&bytes)
                .map_err(|e| super::engine_error(e.into()))?;
            AddSource::Path(path.to_string_lossy().into_owned())
        }
    };
    let call = Call::TorrentsAdd {
        args: AddArgs {
            source,
            stage: args.stage,
            stopped: args.stopped,
            seed: args.seed,
            save_path: args.save_path.clone(),
            files: args.files.clone(),
            category: args.category.clone(),
            origin: Some("search".into()),
        },
    };
    let Reply::TorrentsAdd(result) = super::torrents::dispatch(daemon, &call)? else {
        unreachable!()
    };
    drop(temporary);
    Ok(Reply::SearchGrab(result))
}
