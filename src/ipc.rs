//! Private process-wide ownership; API v1 owns its listener separately.
use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::Duration;

/// Held for the whole engine lifetime, before any session or file I/O starts.
pub(crate) struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub(crate) fn acquire() -> Result<Self> {
        Self::acquire_in(crate::config::runtime_dir())
    }

    pub(crate) fn acquire_in(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        Self::acquire_wait(directory, Duration::ZERO)
    }

    /// The daemon validates the directory before this call. Do not repair or
    /// follow an ambiguous path here; keep the same open file across retries.
    pub(crate) fn acquire_wait(directory: PathBuf, timeout: Duration) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("engine.lock"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            bail!("engine.lock must be a regular private file owned by this uid");
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        // The stable lock file must never be unlinked while another app can open it.
        let began = std::time::Instant::now();
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                if began.elapsed() < timeout {
                    std::thread::sleep(Duration::from_millis(20).min(timeout.saturating_sub(began.elapsed())));
                    continue;
                }
                return Err(crate::agent::AppError::new(crate::agent::Code::Failure,
                    "already_running: fbittorrent is already open; use the existing app")
                    .hint("fbittorrent status --json").into());
            }
            return Err(error).context("locking the native torrent engine");
        }
        Ok(Self {
            _file: file,
        })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // flock belongs to the open file description, which fork inherits even
        // with CLOEXEC. A parallel provider spawn may not have reached exec yet
        // when the engine closes and reopens this directory. Release ownership
        // explicitly instead of waiting for every inherited descriptor to close.
        // The stable lock file remains in place for the next owner.
        unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    #[test]
    fn instance_lock_waits_for_a_short_offline_owner() {
        let directory = std::env::temp_dir().join(format!("fbt-lock-wait-{}", std::process::id()));
        let first = InstanceLock::acquire_in(directory.clone()).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(first);
        });
        let began = Instant::now();
        let next = InstanceLock::acquire_wait(directory.clone(), Duration::from_secs(1)).unwrap();
        println!("offline lock acquired after {} ms", began.elapsed().as_millis());
        assert!(began.elapsed() >= Duration::from_millis(100));
        release.join().unwrap();
        drop(next);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn instance_lock_repairs_existing_permissions_without_replacing_the_file() {
        use std::os::unix::fs::MetadataExt;
        let directory = std::env::temp_dir().join(format!("fbittorrent-private-lock-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("engine.lock");
        std::fs::write(&path, b"stable lock fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        let lock = InstanceLock::acquire_in(directory.clone()).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.ino(), inode);
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"stable lock fixture");
        assert!(InstanceLock::acquire_in(directory.clone()).is_err());
        println!("{}: 600 after acquiring an existing 644 lock", path.display());
        drop(lock);
        drop(InstanceLock::acquire_in(directory.clone()).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
