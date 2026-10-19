//! Saved reads and offline intents use the daemon's process lock and durable store.
use crate::{
    api,
    config::{self, Config},
    daemon::lifecycle::Lifecycle,
    engine::{
        ops::{EngineOp, EngineResult},
        ParkedLedger,
    },
    ipc::InstanceLock,
};
use anyhow::Result;
use std::time::Duration;

pub fn locked() -> Result<(InstanceLock, ParkedLedger)> {
    let directory = config::runtime_dir();
    std::fs::create_dir_all(&directory)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let lock = InstanceLock::acquire_wait(config::runtime_dir(), Duration::from_secs(2))?;
    Ok((
        lock,
        ParkedLedger::new(config::data_dir().join("engine"), Lifecycle::default()),
    ))
}
pub fn list() -> Result<Vec<api::Torrent>> {
    let (_lock, ledger) = locked()?;
    let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info)? else {
        unreachable!()
    };
    rows.into_iter()
        .map(|row| api::ops::torrents::torrent(row).map_err(api::errors::app_error))
        .collect()
}
/// Resolve and persist under the same owner lock. Two hello/lock rounds bound
/// a simultaneous shutdown without racing its final durable ledger save.
pub fn mutation(
    cfg: &Config,
    selectors: &[String],
    stop: bool,
    json_mode: bool,
) -> Result<(Vec<String>, bool)> {
    for _ in 0..2 {
        if let Ok(client) = api::client::Client::connect(cfg, json_mode) {
            super::native::warn(&client, json_mode);
            if client.hello.state != api::DaemonState::Stopping {
                if let Ok(value) = client.call("torrents.list", serde_json::json!({})) {
                    let rows: Vec<api::Torrent> =
                        serde_json::from_value(value["torrents"].clone())?;
                    let hashes =
                        crate::cli_sources::resolve_in(&super::shapes::legacy(&rows)?, selectors)?;
                    for chunk in hashes.chunks(256) {
                        let args = if stop {
                            serde_json::json!({"hashes":chunk})
                        } else {
                            serde_json::json!({"hashes":chunk,"delete_files":false})
                        };
                        client.call(
                            if stop {
                                "torrents.stop"
                            } else {
                                "torrents.remove"
                            },
                            args,
                        )?;
                    }
                    return Ok((hashes, true));
                }
            }
        }
        // A v0.1 TUI owner has no API socket. Refuse before the offline lock
        // fallback, even if that older owner does not hold the ledger lock.
        if !crate::ipc::socket_path(cfg)
            .with_file_name("api.sock")
            .exists()
            && crate::ipc::is_running(cfg)
        {
            return Err(crate::agent::AppError::new(
                crate::agent::Code::Backend,
                "an older fbittorrent app is running; quit it with Q and retry",
            )
            .into());
        }
        if let Ok((_lock, ledger)) = locked() {
            let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info)? else {
                unreachable!()
            };
            let rows = rows
                .into_iter()
                .map(api::ops::torrents::torrent)
                .collect::<Result<Vec<_>, _>>()
                .map_err(api::errors::app_error)?;
            let hashes = crate::cli_sources::resolve_in(&super::shapes::legacy(&rows)?, selectors)?;
            use crate::engine::ops::HashSelection;
            let selection = HashSelection::Hashes(hashes.clone());
            ledger.execute(if stop {
                EngineOp::Stop { hashes: selection }
            } else {
                EngineOp::Delete {
                    hashes: selection,
                    files: false,
                }
            })?;
            return Ok((hashes, false));
        }
    }
    Err(crate::agent::AppError::new(
        crate::agent::Code::Backend,
        "daemon is starting or draining; saved intent was not changed",
    )
    .hint("fbittorrent status --json")
    .into())
}
