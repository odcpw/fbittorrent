//! Private metadata handoff for local provider helpers until Track B owns them.
use anyhow::Result;
use std::{
    io::Write,
    os::fd::AsRawFd,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

pub(super) fn publish(bytes: &[u8]) -> Result<PathBuf> {
    anyhow::ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "torrent metadata exceeds 16 MiB"
    );
    let directory = dirs::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("no cache directory"))?
        .join("fbittorrent/ui-add");
    publish_in(&directory, bytes)
}
fn publish_in(directory: &std::path::Path, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(&directory)?;
    for path in [directory.parent().unwrap(), directory] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(directory.join(".lock"))?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let count = std::fs::read_dir(directory)?.try_fold(
        0usize,
        |count, entry| -> std::io::Result<usize> {
            Ok(count
                + usize::from(
                    entry?
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "torrent"),
                ))
        },
    )?;
    anyhow::ensure!(count<8,"provider metadata cache has eight unconfirmed handoffs; inspect downloads before clearing {}",directory.display());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = directory.join(format!("{}-{stamp}.torrent", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::File::open(&directory)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_is_private_fresh_and_bounded() {
        let root = std::env::temp_dir().join(format!(
            "fbt-ui-metadata-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("ui-add");
        std::fs::create_dir_all(&directory).unwrap();
        let lock = directory.join(".lock");
        std::fs::write(&lock, b"").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
        let old = directory.join("old.torrent");
        std::fs::write(&old, b"old pending handoff").unwrap();
        let path = publish_in(&directory, b"fresh metadata bytes").unwrap();
        assert_ne!(path, old);
        assert_eq!(std::fs::read(&old).unwrap(), b"old pending handoff");
        assert_eq!(std::fs::read(&path).unwrap(), b"fresh metadata bytes");
        for path in [&directory, &root] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for path in [&path, &lock] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        for index in 0..6 {
            std::fs::write(
                directory.join(format!("pending-{index}.torrent")),
                b"pending",
            )
            .unwrap();
        }
        assert!(publish_in(&directory, b"must not create a ninth handoff").is_err());
        assert_eq!(
            std::fs::read_dir(&directory)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "torrent"))
                .count(),
            8
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
