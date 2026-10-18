use std::{
    fs::File,
    io::IoSlice,
    path::{Path, PathBuf},
};

#[cfg(not(unix))]
use std::fs::OpenOptions;

use anyhow::Context;
use tracing::warn;

use crate::{
    storage::{StorageFactoryExt, filesystem::opened_file::OurFileExt},
    torrent_state::{ManagedTorrentShared, TorrentMetadata},
};

use crate::storage::{StorageFactory, TorrentStorage};

use super::opened_file::OpenedFile;

#[cfg(unix)]
fn open_parent_directory(
    root: &Path,
    relative: &Path,
    create: bool,
) -> anyhow::Result<(std::os::fd::OwnedFd, std::ffi::OsString)> {
    use nix::{
        fcntl::{OFlag, open, openat},
        sys::stat::{Mode, mkdirat},
    };
    use std::path::Component;

    let names = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => anyhow::bail!("torrent path must contain only relative filename components"),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let (filename, parents) = names.split_last().context("torrent file has no filename")?;
    if create {
        std::fs::create_dir_all(root)?;
    }
    // The chosen output folder is the anchor. Torrent-controlled children are
    // opened through directory descriptors and never follow a symlink.
    let mut directory = open(
        root,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    for name in parents {
        if create {
            match mkdirat(&directory, *name, Mode::from_bits_truncate(0o777)) {
                Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
                Err(error) => return Err(error).context("creating torrent directory"),
            }
        }
        directory = openat(
            &directory,
            *name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .context("opening torrent directory without following symlinks")?;
    }
    Ok((directory, (*filename).to_owned()))
}

#[cfg(unix)]
fn open_download_file(root: &Path, relative: &Path, overwrite: bool) -> anyhow::Result<File> {
    use nix::{
        fcntl::{OFlag, openat},
        sys::stat::Mode,
    };
    let (directory, filename) = open_parent_directory(root, relative, true)?;
    let mut flags = OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    if !overwrite {
        flags |= OFlag::O_EXCL;
    }
    let file = openat(
        &directory,
        filename.as_os_str(),
        flags,
        Mode::from_bits_truncate(0o666),
    )
    .with_context(|| format!("opening torrent file {relative:?} without following symlinks"))?;
    Ok(File::from(file))
}

#[cfg(not(unix))]
fn open_download_file(root: &Path, relative: &Path, overwrite: bool) -> anyhow::Result<File> {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().context("torrent file has no parent")?)?;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(overwrite)
        .create_new(!overwrite)
        .open(path)?)
}

#[derive(Default, Clone, Copy)]
pub struct FilesystemStorageFactory {}

impl StorageFactory for FilesystemStorageFactory {
    type Storage = FilesystemStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        _metadata: &TorrentMetadata,
    ) -> anyhow::Result<FilesystemStorage> {
        Ok(FilesystemStorage {
            output_folder: shared.options.output_folder.clone(),
            opened_files: Default::default(),
        })
    }

    fn clone_box(&self) -> crate::storage::BoxStorageFactory {
        self.boxed()
    }
}

pub struct FilesystemStorage {
    pub(crate) output_folder: PathBuf,
    pub(crate) opened_files: Vec<OpenedFile>,
}

impl FilesystemStorage {
    #[allow(dead_code)]
    pub(crate) fn take_fs(&self) -> anyhow::Result<Self> {
        Ok(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
        })
    }
}

impl TorrentStorage for FilesystemStorage {
    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.opened_files
            .get(file_id)
            .context("no such file")?
            .lock_read()?
            .pread_exact(offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all(offset, buf);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all(offset, buf);
    }

    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        let of = self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        return of.try_mark_sparse()?.pwrite_all_vectored(offset, bufs);
        #[cfg(not(windows))]
        return of.lock_read()?.pwrite_all_vectored(offset, bufs);
    }

    fn remove_file(&self, _file_id: usize, filename: &Path) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            let (directory, filename) =
                open_parent_directory(&self.output_folder, filename, false)?;
            nix::unistd::unlinkat(
                &directory,
                filename.as_os_str(),
                nix::unistd::UnlinkatFlags::NoRemoveDir,
            )?;
            return Ok(());
        }
        #[cfg(not(unix))]
        Ok(std::fs::remove_file(self.output_folder.join(filename))?)
    }

    fn ensure_file_length(&self, file_id: usize, len: u64) -> anyhow::Result<()> {
        let f = &self.opened_files.get(file_id).context("no such file")?;
        #[cfg(windows)]
        f.try_mark_sparse()?;
        Ok(f.lock_read()?.set_len(len)?)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            opened_files: self
                .opened_files
                .iter()
                .map(|f| f.take_clone())
                .collect::<anyhow::Result<Vec<_>>>()?,
            output_folder: self.output_folder.clone(),
        }))
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            if path.as_os_str().is_empty() {
                return match std::fs::remove_dir(&self.output_folder) {
                    Ok(()) => Ok(()),
                    Err(error) if matches!(error.raw_os_error(), Some(code) if code == nix::errno::Errno::ENOTEMPTY as i32 || code == nix::errno::Errno::EEXIST as i32) =>
                    {
                        warn!(
                            "did not remove {:?} as it was not empty",
                            self.output_folder
                        );
                        Ok(())
                    }
                    Err(error) => Err(error).context("removing empty torrent output directory"),
                };
            }
            let (directory, filename) = open_parent_directory(&self.output_folder, path, false)?;
            return match nix::unistd::unlinkat(
                &directory,
                filename.as_os_str(),
                nix::unistd::UnlinkatFlags::RemoveDir,
            ) {
                Ok(()) => Ok(()),
                Err(nix::errno::Errno::ENOTEMPTY | nix::errno::Errno::EEXIST) => {
                    warn!("did not remove {path:?} as it was not empty");
                    Ok(())
                }
                Err(error) => {
                    Err(error).with_context(|| format!("removing torrent directory {path:?}"))
                }
            };
        }
        #[cfg(not(unix))]
        {
            let path = self.output_folder.join(path);
            if !path.is_dir() {
                anyhow::bail!("cannot remove dir: {path:?} is not a directory")
            }
            if std::fs::read_dir(&path)?.count() == 0 {
                std::fs::remove_dir(&path).with_context(|| format!("error removing {path:?}"))
            } else {
                warn!("did not remove {path:?} as it was not empty");
                Ok(())
            }
        }
    }

    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        let mut files = Vec::<OpenedFile>::new();
        for file_details in metadata.file_infos.iter() {
            let mut full_path = self.output_folder.clone();
            let relative_path = &file_details.relative_filename;
            full_path.push(relative_path);

            if file_details.attrs.padding {
                files.push(OpenedFile::new_dummy());
                continue;
            };
            let f = open_download_file(
                &self.output_folder,
                relative_path,
                shared.options.allow_overwrite,
            )?;
            files.push(OpenedFile::new(full_path.clone(), f));
        }

        self.opened_files = files;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn torrent_paths_cannot_follow_symlinks_outside_the_selected_folder() {
        let directory = tempfile::TempDir::new().unwrap();
        let root = directory.path().join("downloads");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        std::fs::write(&sentinel, b"original outside data").unwrap();
        symlink(&sentinel, root.join("file-link")).unwrap();
        symlink(&outside, root.join("directory-link")).unwrap();
        assert!(open_download_file(&root, Path::new("file-link"), true).is_err());
        assert!(open_download_file(&root, Path::new("directory-link/sentinel"), true).is_err());
        assert!(
            open_download_file(&root, Path::new("directory-link/new-dir/payload"), true).is_err()
        );
        assert!(!outside.join("new-dir").exists());

        let storage = FilesystemStorage {
            output_folder: root.clone(),
            opened_files: Vec::new(),
        };
        assert!(
            storage
                .remove_file(0, Path::new("directory-link/sentinel"))
                .is_err()
        );
        assert!(
            storage
                .remove_directory_if_empty(Path::new("directory-link"))
                .is_err()
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"original outside data");
        assert!(open_download_file(&root, Path::new("../outside/sentinel"), true).is_err());
    }

    #[test]
    fn selected_folder_can_be_a_symlink_and_normal_files_keep_positioned_io() {
        let directory = tempfile::TempDir::new().unwrap();
        let root = directory.path().join("actual-downloads");
        std::fs::create_dir_all(&root).unwrap();
        let selected = directory.path().join("selected-downloads");
        symlink(&root, &selected).unwrap();
        let relative = Path::new("nested/payload");
        let file = open_download_file(&selected, relative, true).unwrap();
        file.pwrite_all(0, b"hello").unwrap();
        assert_eq!(std::fs::read(root.join(relative)).unwrap(), b"hello");
        assert!(open_download_file(&selected, relative, false).is_err());
        let storage = FilesystemStorage {
            output_folder: selected,
            opened_files: Vec::new(),
        };
        storage.remove_file(0, relative).unwrap();
        storage
            .remove_directory_if_empty(Path::new("nested"))
            .unwrap();
        assert!(!root.join("nested").exists());
    }
}
