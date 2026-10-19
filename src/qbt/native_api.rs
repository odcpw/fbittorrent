//! Existing RSS/Shows/UI add callers share v1 when a daemon is present.
use super::*;
use serde_json::{json, Value};
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
struct Metainfo(std::path::PathBuf);
impl Drop for Metainfo {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub(super) fn add(
    cfg: &Config,
    mut source: Value,
    options: &AddOptions,
    bytes: Option<&[u8]>,
) -> Result<()> {
    if let Some(url) = source["magnet"].as_str() {
        if !url.starts_with("magnet:") {
            source = json!({"url":url});
        }
    }
    let spool = if let Some(bytes) = bytes {
        let directory = crate::config::data_dir().join("cli-uploads");
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let path = directory.join(format!(
            "{}-{}.torrent",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        source = json!({"path":path.to_string_lossy()});
        Some(Metainfo(path))
    } else {
        None
    };
    let client = crate::api::client::Client::connect(cfg, false)?;
    let mut args =
        json!({"source":source,"stopped":options.stopped,"seed":options.seed,"origin":options.origin.as_deref().unwrap_or("manual")});
    if let Some(path) = &options.save_path {
        args["save_path"] = json!(path);
    }
    if let Some(category) = &options.category {
        args["category"] = json!(category);
    }
    let response = client.call("torrents.add", args)?;
    if response["state"] == "pending" && response.get("hash").is_none() {
        let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs.max(95));
        loop {
            if Instant::now() >= deadline {
                return Err(crate::agent::AppError::new(
                    crate::agent::Code::Timeout,
                    "add confirmation timed out; outcome is unknown",
                )
                .into());
            }
            if let Some(event) = client.event(Duration::from_millis(100))? {
                if event["ev"] == "mutation.result" && event["ref"] == response["ref"] {
                    if event["outcome"] != "done" {
                        return Err(crate::api::errors::app_error(serde_json::from_value(
                            event["error"].clone(),
                        )?));
                    }
                    break;
                }
                if event["ev"] == "bye" || event["ev"] == "client.disconnected" {
                    bail!("daemon closed before add outcome was confirmed");
                }
            }
        }
    }
    drop(spool);
    Ok(())
}
