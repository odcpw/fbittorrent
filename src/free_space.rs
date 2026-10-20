//! Read-only destination checks shared by the daemon and native CLI.
use crate::api::FreeSpaceResult;
use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

pub fn valid_path(value: &str) -> Result<(), &'static str> {
    if !value.starts_with('/') || value.len() > 4096 || value.bytes().any(|b| b < 32 || b == 127) {
        return Err(
            "path must be an absolute local directory, at most 4096 bytes, without controls",
        );
    }
    Ok(())
}

fn available(blocks: i128, block_size: i128) -> Option<u64> {
    if blocks < 0 || blocks > i64::MAX as i128 || block_size <= 0 {
        return None;
    }
    u64::try_from(blocks.checked_mul(block_size)?).ok()
}

pub fn inspect(path: &Path) -> FreeSpaceResult {
    let mut result = FreeSpaceResult {
        free_bytes: None,
        writable: false,
        message: None,
    };
    let Ok(path_c) = CString::new(path.as_os_str().as_bytes()) else {
        return result;
    };
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir()=>{},
        Ok(_)=>{result.message=Some("The destination is not a folder. Choose a download folder, then retry.".into());return result;},
        Err(error)=>{result.message=Some(crate::user_messages::storage(&error.to_string()).unwrap_or("Cannot inspect the download folder. Choose another folder, then retry.").into());return result;},
    }
    // No write probe: creating/removing a file would alter a read operation.
    result.writable = unsafe {
        libc::faccessat(
            libc::AT_FDCWD,
            path_c.as_ptr(),
            libc::W_OK | libc::X_OK,
            libc::AT_EACCESS,
        ) == 0
    };
    if !result.writable {result.message=Some(crate::user_messages::PERMISSION.into());}
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path_c.as_ptr(), stats.as_mut_ptr()) } == 0 {
        let stats = unsafe { stats.assume_init() };
        let block_size = if stats.f_frsize == 0 {
            stats.f_bsize
        } else {
            stats.f_frsize
        };
        result.free_bytes = available(stats.f_bavail as i128, block_size as i128);
        if stats.f_flag & libc::ST_RDONLY != 0 {
            result.writable = false;
            result.message=Some(crate::user_messages::PERMISSION.into());
        }
    }
    result
}

pub fn require_writable(path: &str) -> anyhow::Result<()> {
    let failure = valid_path(path).err().map(str::to_owned).or_else(|| {
        let result=inspect(Path::new(path));
        (!result.writable).then(||result.message.unwrap_or_else(||crate::user_messages::PERMISSION.into()))
    });
    if let Some(message) = failure {
        return Err(
            crate::agent::AppError::new(crate::agent::Code::Usage, message)
                .hint("choose a writable folder with --save <directory>")
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDir(std::path::PathBuf);
    impl TestDir {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("fbt-free-space-{}-{stamp}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn absent_on_error() {
        let tmp = TestDir::new();
        let missing = inspect(&tmp.path().join("absent"));
        println!("statvfs missing directory: {missing:?}");
        assert_eq!(missing.free_bytes, None);
        assert!(!missing.writable);
        let file = tmp.path().join("file");
        std::fs::write(&file, b"keep").unwrap();
        assert_eq!(inspect(&file).free_bytes, None);
        assert!(!inspect(&file).writable);
    }
    #[test]
    fn never_negative() {
        for (blocks, size, expected) in [
            (-1, 4096, None),
            (0, 4096, Some(0)),
            (3, 4096, Some(12288)),
            (1, 0, None),
            (1, -1, None),
            (i64::MAX as i128, 4096, None),
            (u64::MAX as i128, 1, None),
        ] {
            println!(
                "statvfs blocks={blocks} block_size={size}: {:?}",
                available(blocks, size)
            );
            assert_eq!(available(blocks, size), expected);
        }
    }
    #[test]
    fn writable_directory_check_is_read_only() {
        let tmp = TestDir::new();
        let before = std::fs::read_dir(tmp.path()).unwrap().count();
        let result = inspect(tmp.path());
        println!("real statvfs: {result:?}");
        assert!(result.writable);
        assert!(result.free_bytes.is_some());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), before);
        assert!(require_writable(tmp.path().to_str().unwrap()).is_ok());
        assert!(require_writable(tmp.path().join("missing").to_str().unwrap()).is_err());
        for invalid in ["relative", "", "/tmp/a\0b", "/tmp/a\nb"] {
            assert!(valid_path(invalid).is_err());
        }
    }
}
