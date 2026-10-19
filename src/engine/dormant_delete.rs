//! Delete only metainfo-declared files through pinned, no-follow directories.
use super::*;
use std::{
    ffi::CString,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::Component,
};

pub(super) struct Plan {
    root: PathBuf,
    files: Vec<PathBuf>,
}
impl Inner {
    pub(super) fn dormant_delete_plan(&self, record: &Record) -> Result<Plan> {
        let bytes = parked::parked_metadata(record, &self.directory)?
            .context("saved torrent metadata is unavailable")?;
        let torrent = librqbit::torrent_from_bytes(&bytes)?;
        let info = torrent.info.data.validate()?;
        let files: Vec<_> = info
            .iter_file_details()
            .map(|file| file.filename.to_pathbuf())
            .collect();
        for file in &files {
            anyhow::ensure!(
                !file.as_os_str().is_empty()
                    && file
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "saved torrent file must be relative to its download directory"
            );
        }
        Ok(Plan {
            root: PathBuf::from(&record.output_folder),
            files,
        })
    }
}
impl Plan {
    pub(super) async fn prepare(self, cx: &Cx) -> Result<Prepared> {
        task::spawn_blocking(cx, move || prepare_files(&self.root, &self.files)).await?
    }
}
// Validate every parent before changing the ledger or deleting any payload.
// Keeping descriptors pins the validated directories across concurrent renames.
pub(super) struct Prepared(Vec<(File, CString)>);
impl Prepared {
    pub(super) async fn execute(self, cx: &Cx) -> Result<()> {
        task::spawn_blocking(cx, move || self.unlink()).await?
    }
    fn unlink(self) -> Result<()> {
        for (parent, leaf) in self.0 {
            if unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error).context("unlinking downloaded file");
                }
            }
        }
        Ok(())
    }
}
/// Directory descriptors keep intermediate symlinks and rename races from
/// redirecting unlink outside the configured root. A leaf symlink is unlinked,
/// never followed. Empty directories and unrelated files are retained.
fn prepare_files(root: &Path, files: &[PathBuf]) -> Result<Prepared> {
    let directory = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
    {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Prepared(Vec::new())),
        Err(error) => return Err(error).context("opening downloaded-file directory"),
    };
    let mut entries = Vec::with_capacity(files.len());
    for path in files {
        let parts: Vec<_> = path
            .components()
            .map(|part| match part {
                Component::Normal(name) => {
                    CString::new(name.as_bytes()).map_err(anyhow::Error::from)
                }
                _ => bail!("downloaded-file path is not relative"),
            })
            .collect::<Result<_>>()?;
        let (leaf, parents) = parts.split_last().context("empty downloaded-file path")?;
        let mut parent = directory.try_clone()?;
        let mut absent = false;
        for part in parents {
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    part.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::NotFound {
                    absent = true;
                    break;
                }
                return Err(error)
                    .context("opening downloaded-file parent without following symlinks");
            }
            parent = unsafe { File::from_raw_fd(fd) };
        }
        if absent {
            continue;
        }
        entries.push((parent, leaf.clone()));
    }
    Ok(Prepared(entries))
}
#[cfg(test)]
fn unlink_files(root: &Path, files: &[PathBuf]) -> Result<()> {
    prepare_files(root, files)?.unlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_symlinks_are_refused_before_any_unlink() {
        let root = super::super::tests::review_directory("root-unlink");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("payload"), b"outside retained").unwrap();
        let link = root.join("downloads");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(unlink_files(&link, &["payload".into()]).is_err());
        assert_eq!(std::fs::read(outside.join("payload")).unwrap(), b"outside retained");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn all_parents_are_validated_before_any_unlink_and_then_pinned() {
        let root = super::super::tests::review_directory("pinned-unlink");
        let downloads = root.join("downloads");
        let outside = root.join("outside");
        std::fs::create_dir_all(downloads.join("nested")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(downloads.join("payload"), b"owned").unwrap();
        std::fs::write(downloads.join("nested/payload"), b"owned nested").unwrap();
        std::fs::write(outside.join("payload"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("redirect")).unwrap();
        assert!(unlink_files(&downloads, &["payload".into(), "redirect/payload".into()]).is_err());
        assert!(downloads.join("payload").is_file(), "failed preflight deletes nothing");
        let prepared = prepare_files(&downloads, &["nested/payload".into()]).unwrap();
        std::fs::rename(downloads.join("nested"), downloads.join("original")).unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("nested")).unwrap();
        prepared.unlink().unwrap();
        assert!(!downloads.join("original/payload").exists());
        assert_eq!(std::fs::read(outside.join("payload")).unwrap(), b"outside");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parent_symlinks_cannot_redirect_dormant_deletion() {
        let root = super::super::tests::review_directory("dormant-unlink");
        let downloads = root.join("downloads");
        let outside = root.join("outside");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("payload");
        std::fs::write(&sentinel, b"retained").unwrap();
        std::os::unix::fs::symlink(&outside, downloads.join("redirect")).unwrap();
        assert!(unlink_files(&downloads, &[PathBuf::from("redirect/payload")]).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"retained");
        std::os::unix::fs::symlink(&sentinel, downloads.join("leaf")).unwrap();
        unlink_files(
            &downloads,
            &[PathBuf::from("leaf"), PathBuf::from("already-absent")],
        )
        .unwrap();
        assert!(std::fs::symlink_metadata(downloads.join("leaf")).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"retained");
        std::fs::remove_dir_all(root).unwrap();
    }
}
