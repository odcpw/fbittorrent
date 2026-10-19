//! Anchored payload transactions. Copy keeps every source until durable commit.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CString, OsStr},
    fs::{File, FileTimes},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Debug)]
pub(super) struct SourceProblem;
impl std::fmt::Display for SourceProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reading move source")
    }
}
impl std::error::Error for SourceProblem {}
#[derive(Debug)]
pub(super) struct DestinationProblem;
impl std::fmt::Display for DestinationProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("writing move destination")
    }
}
impl std::error::Error for DestinationProblem {}
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Strategy {
    Rename,
    Copy,
}
pub(super) fn strategy(source: u64, target: u64) -> Strategy {
    if source == target {
        Strategy::Rename
    } else {
        Strategy::Copy
    }
}
fn name(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).context("NUL in payload path")
}
fn io_error<T>(fd: i32, make: impl FnOnce(i32) -> T) -> Result<T> {
    if fd < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(make(fd))
    }
}
struct Dir(File);
impl Dir {
    fn root(path: &Path, create: bool) -> Result<Self> {
        ensure!(path.is_absolute(), "save_path must be absolute");
        let root = CString::new("/").unwrap();
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        let mut directory = Self(io_error(fd, |fd| unsafe { File::from_raw_fd(fd) })?);
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(part) => directory = directory.child(part, create)?,
                _ => bail!("parent traversal in save_path"),
            }
        }
        Ok(directory)
    }
    fn child(&self, part: &OsStr, create: bool) -> Result<Self> {
        let part = name(part)?;
        if create {
            let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), part.as_ptr(), 0o700) };
            if result == 0 {
                self.0.sync_all()?;
            }
            if result != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        Ok(Self(io_error(fd, |fd| unsafe { File::from_raw_fd(fd) })?))
    }
    fn parent(&self, path: &Path, create: bool) -> Result<(Self, CString)> {
        let mut parts = path
            .components()
            .map(|part| match part {
                Component::Normal(value) => Ok(value),
                _ => bail!("invalid relative payload path"),
            })
            .collect::<Result<Vec<_>>>()?;
        let leaf = name(parts.pop().context("empty payload path")?)?;
        let mut directory = Self(self.0.try_clone()?);
        for part in parts {
            directory = directory.child(part, create)?;
        }
        Ok((directory, leaf))
    }
    fn file(&self, path: &Path, create: bool) -> Result<Option<File>> {
        let (parent, leaf) = match self.parent(path, create) {
            Ok(value) => value,
            Err(error)
                if !create
                    && error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if create {
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
        let fd = unsafe { libc::openat(parent.0.as_raw_fd(), leaf.as_ptr(), flags, 0o600) };
        if fd < 0
            && !create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            return Ok(None);
        }
        let file = io_error(fd, |fd| unsafe { File::from_raw_fd(fd) })?;
        ensure!(file.metadata()?.is_file(), "payload must be a regular file");
        Ok(Some(file))
    }
    fn rename(&self, relative: &Path, to: &Dir) -> Result<()> {
        let (source, leaf) = self.parent(relative, false)?;
        let (target, target_leaf) = to.parent(relative, true)?;
        let result = unsafe {
            libc::renameat2(
                source.0.as_raw_fd(),
                leaf.as_ptr(),
                target.0.as_raw_fd(),
                target_leaf.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn sync_parent(&self, path: &Path) -> Result<()> {
        self.parent(path, false)?.0.0.sync_all()?;
        Ok(())
    }
    fn unlink(&self, relative: &Path, directory: bool) -> Result<()> {
        let (parent, leaf) = self.parent(relative, false)?;
        let result = unsafe {
            libc::unlinkat(
                parent.0.as_raw_fd(),
                leaf.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        parent.0.sync_all()?;
        Ok(())
    }
}
struct Entry {
    path: PathBuf,
    published: bool,
    stamp: (u64, u64, u64, i64, i64),
}
fn stamp(file: &File) -> Result<(u64, u64, u64, i64, i64)> {
    let m = file.metadata()?;
    Ok((m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec()))
}
/// Returned only after the complete batch is published. Drop restores renamed
/// sources or discards copies until the caller durably changes the ledger.
pub(super) struct Prepared {
    source: Dir,
    target: Dir,
    stage: Dir,
    stage_name: PathBuf,
    strategy: Strategy,
    entries: Vec<Entry>,
    committed: bool,
}
impl Prepared {
    #[cfg(test)]
    pub(super) fn new(source: &Path, target: &Path, paths: &[PathBuf]) -> Result<Self> {
        Self::with_io(source, target, paths, strategy, |source, target| {
            copy(&mut &*source, target)
        })
    }
    #[cfg(test)]
    fn with_io(
        source: &Path,
        target: &Path,
        paths: &[PathBuf],
        choose: impl Fn(u64, u64) -> Strategy,
        copier: impl FnMut(&File, &mut File) -> Result<()>,
    ) -> Result<Self> {
        Self::with_checks(
            source,
            target,
            paths,
            &paths.iter().cloned().collect(),
            &Default::default(),
            choose,
            copier,
            || Ok(()),
        )
    }
    pub(super) fn cancellable(
        source: &Path,
        target: &Path,
        paths: &[PathBuf],
        required: &std::collections::HashSet<PathBuf>,
        lengths: &std::collections::HashMap<PathBuf, (u64, u64)>,
        stopping: &std::sync::atomic::AtomicBool,
    ) -> Result<Self> {
        let check = || {
            ensure!(
                !stopping.load(Ordering::Acquire),
                "engine is closing during move"
            );
            Ok(())
        };
        Self::with_checks(
            source,
            target,
            paths,
            required,
            lengths,
            strategy,
            |source, target| copy_checked(&mut &*source, target, &check),
            check,
        )
        .context(DestinationProblem)
    }
    fn with_checks(
        source: &Path,
        target: &Path,
        paths: &[PathBuf],
        required: &std::collections::HashSet<PathBuf>,
        lengths: &std::collections::HashMap<PathBuf, (u64, u64)>,
        choose: impl Fn(u64, u64) -> Strategy,
        mut copier: impl FnMut(&File, &mut File) -> Result<()>,
        check: impl Fn() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        let source = Dir::root(source, false).context(SourceProblem)?;
        let target = Dir::root(target, true)?;
        let strategy = choose(source.0.metadata()?.dev(), target.0.metadata()?.dev());
        let stage_name = PathBuf::from(format!(
            ".fbittorrent-move-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let stage_leaf = name(stage_name.as_os_str())?;
        let created = unsafe { libc::mkdirat(target.0.as_raw_fd(), stage_leaf.as_ptr(), 0o700) };
        if created != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let stage = target.child(stage_name.as_os_str(), false)?;
        let mut prepared = Self {
            source,
            target,
            stage,
            stage_name,
            strategy,
            entries: Vec::new(),
            committed: false,
        };
        for path in paths {
            check()?;
            let source = match prepared.source.file(path, false) {
                Ok(Some(file)) => file,
                Ok(None) if !required.contains(path) => continue,
                Ok(None) => {
                    return Err(anyhow::anyhow!("required source payload is missing")
                        .context(SourceProblem));
                }
                Err(error) => return Err(error.context(SourceProblem)),
            };
            // Reject all collisions, including dangling symlinks, before touching
            // this source. O_EXCL publication also protects the final race.
            let (parent, leaf) = prepared.target.parent(path, true)?;
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            let exists = unsafe {
                libc::fstatat(
                    parent.0.as_raw_fd(),
                    leaf.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            ensure!(
                exists != 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound,
                "destination payload already exists"
            );
            let original = stamp(&source).context(SourceProblem)?;
            if let Some((minimum, maximum)) = lengths.get(path) {
                if original.2 < *minimum || original.2 > *maximum {
                    return Err(anyhow::anyhow!(
                        "source size no longer matches its verified pieces"
                    )
                    .context(SourceProblem));
                }
            }
            source.sync_all().context(SourceProblem)?;
            prepared.entries.push(Entry {
                path: path.clone(),
                published: false,
                stamp: original,
            });
            match strategy {
                Strategy::Rename => prepared.source.rename(path, &prepared.stage)?,
                Strategy::Copy => {
                    let mut target = prepared.stage.file(path, true)?.unwrap();
                    copier(&source, &mut target)?;
                    let metadata = source.metadata()?;
                    target.set_permissions(std::fs::Permissions::from_mode(
                        metadata.mode() & 0o777,
                    ))?;
                    target.set_times(
                        FileTimes::new()
                            .set_modified(metadata.modified()?)
                            .set_accessed(metadata.accessed()?),
                    )?;
                    target.sync_all()?;
                    ensure!(
                        stamp(&source)? == original,
                        "source changed during payload copy"
                    );
                }
            }
            prepared.source.sync_parent(path)?;
            prepared.stage.sync_parent(path)?;
        }
        for index in 0..prepared.entries.len() {
            check()?;
            let path = prepared.entries[index].path.clone();
            prepared.stage.rename(&path, &prepared.target)?;
            prepared.entries[index].published = true;
            prepared.stage.sync_parent(&path)?;
            prepared.target.sync_parent(&path)?;
        }
        Ok(prepared)
    }
    /// The durable ledger now names the destination. A failed unlink may leave
    /// a redundant source, but can never discard the valid destination.
    pub(super) fn commit(mut self) -> Result<()> {
        self.committed = true;
        if self.strategy == Strategy::Copy {
            for entry in &self.entries {
                let file = self
                    .source
                    .file(&entry.path, false)?
                    .context("source vanished before commit")?;
                ensure!(
                    stamp(&file)? == entry.stamp,
                    "source changed before cleanup"
                );
                self.source.unlink(&entry.path, false)?;
            }
        }
        self.clean_stage();
        Ok(())
    }
    pub(super) fn retain(mut self) {
        self.committed = true;
    }
    fn clean_stage(&self) {
        let mut directories = std::collections::BTreeSet::new();
        for entry in &self.entries {
            for parent in entry
                .path
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                directories.insert(parent.to_owned());
            }
        }
        for path in directories.into_iter().rev() {
            let _ = self.stage.unlink(&path, true);
        }
        let _ = self.target.unlink(&self.stage_name, true);
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.committed {
            for entry in self.entries.iter().rev() {
                let location = if entry.published {
                    &self.target
                } else {
                    &self.stage
                };
                match self.strategy {
                    Strategy::Rename => {
                        let _ = location.rename(&entry.path, &self.source);
                    }
                    Strategy::Copy => {
                        let _ = location.unlink(&entry.path, false);
                    }
                }
            }
        }
        self.clean_stage();
    }
}
#[cfg(test)]
fn copy(source: &mut impl Read, target: &mut impl Write) -> Result<()> {
    copy_checked(source, target, &|| Ok(()))
}
fn copy_checked(
    source: &mut impl Read,
    target: &mut impl Write,
    check: &impl Fn() -> Result<()>,
) -> Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        check()?;
        let size = source.read(&mut buffer).context(SourceProblem)?;
        if size == 0 {
            break;
        }
        target
            .write_all(&buffer[..size])
            .context(DestinationProblem)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
