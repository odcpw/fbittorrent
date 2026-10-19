use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::{
    fd::AsRawFd,
    unix::fs::{OpenOptionsExt, PermissionsExt},
};
use std::path::Path;

pub(super) const QUIT_NOTICE:&str="Downloads continue in the background; fbittorrent stops itself when idle. Q here, or the bar icon's power button, stops it now.";

/// Serialize concurrent first quits, retain private store modes, and replace atomically.
pub(super) fn first_quit(directory: &Path) -> Result<bool> {
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(directory.join("notices.lock"))?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(false);
        }
        return Err(error.into());
    }
    let path = directory.join("notices.json");
    let mut value = match std::fs::File::open(&path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(4097).read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 4096, "notice file exceeds 4096 bytes");
            serde_json::from_slice::<serde_json::Value>(&bytes)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    if value.get("first_quit_seen").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(false);
    }
    value
        .as_object_mut()
        .context("notices must be a JSON object")?
        .insert("first_quit_seen".into(), true.into());
    let temporary = directory.join(format!(
        ".notices-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(&value)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(true)
}
