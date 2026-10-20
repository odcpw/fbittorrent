//! At most one kernel sync plus one coalesced latest snapshot. No thread or API
//! reader waits for fsync under Watch's mutex; a successful CQE precedes rename.
use crate::durable_sync::FileSync;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) struct Store {
    path: PathBuf,
    latest: Option<Vec<u8>>,
    writing: Option<Pending>,
    retry_after: Instant,
}
struct Pending {
    sync: FileSync,
    temporary: PathBuf,
    bytes: Vec<u8>,
    renamed: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.renamed {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}
impl Pending {
    fn begin(
        path: &Path,
        bytes: Vec<u8>,
        start: impl FnOnce(&File) -> io::Result<FileSync>,
    ) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("watch history has no parent"))?;
        fs::create_dir_all(parent)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary = path.with_extension(format!("{}-{nonce}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        let create = || {
            file.write_all(&bytes)?;
            start(&file)
        };
        match create() {
            Ok(sync) => Ok(Self {
                sync,
                temporary,
                bytes,
                renamed: false,
            }),
            Err(error) => {
                let _ = fs::remove_file(temporary);
                Err(error)
            }
        }
    }
}
impl Store {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            latest: None,
            writing: None,
            retry_after: Instant::now(),
        }
    }
    pub(super) fn pending(&self) -> bool {
        self.latest.is_some() || self.writing.is_some()
    }
    pub(super) fn queue(&mut self, bytes: Vec<u8>) {
        if bytes.len() > 2 * 1024 * 1024 {
            eprintln!("WARN watch history exceeds 2 MiB; retaining durable checkpoint");
            return;
        }
        self.latest = Some(bytes);
        self.poll();
    }
    pub(super) fn poll(&mut self) {
        if let Some(mut writing) = self.writing.take() {
            match writing.sync.completion() {
                None => {
                    self.writing = Some(writing);
                    return;
                }
                Some(Ok(())) if !writing.renamed => {
                    let mut publish = || {
                        fs::rename(&writing.temporary, &self.path)?;
                        writing.renamed = true;
                        FileSync::start(&File::open(self.path.parent().unwrap())?)
                    };
                    match publish() {
                        Ok(sync) => {
                            writing.sync = sync;
                            self.writing = Some(writing);
                            return;
                        }
                        Err(error) => self.failed(error, Some(&writing.bytes)),
                    }
                }
                Some(Ok(())) => {}
                Some(Err(error)) => self.failed(error, Some(&writing.bytes)),
            }
        }
        if Instant::now() < self.retry_after {
            return;
        }
        if let Some(bytes) = self.latest.take() {
            // One extra bounded snapshot is retained only during creation, to
            // allow retry if writing/opening fails before a Pending exists.
            match Pending::begin(&self.path, bytes.clone(), FileSync::start) {
                Ok(writing) => self.writing = Some(writing),
                Err(error) => self.failed(error, Some(&bytes)),
            }
        }
    }
    fn failed(&mut self, error: io::Error, bytes: Option<&[u8]>) {
        if self.latest.is_none() {
            self.latest = bytes.map(<[u8]>::to_vec);
        }
        self.retry_after = Instant::now() + Duration::from_secs(1);
        eprintln!("WARN saving watch history: {error}");
    }
    pub(super) fn flush(&mut self, deadline: Instant) {
        while self.pending() && Instant::now() < deadline {
            self.poll();
            if self.pending() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if self.pending() {
            eprintln!("INFO watch checkpoint pending at shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    #[test]
    fn pending_kernel_sync_never_blocks_readiness_and_coalesces_latest() {
        let root = std::env::temp_dir().join(format!(
            "fbt-watch-sync-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("watch.json");
        fs::write(&path, b"old checkpoint").unwrap();
        let (gate, mut release) = UnixStream::pair().unwrap();
        let mut store = Store::new(path.clone());
        store.writing = Some(
            Pending::begin(&path, b"first queued".to_vec(), |file| {
                FileSync::gated_for_test(file, &gate)
            })
            .unwrap(),
        );
        let begin = Instant::now();
        // A real linked kernel poll keeps the fsync pending during every call.
        // New snapshots replace one slot; the previous checkpoint stays valid.
        for n in 0..1000 {
            store.queue(format!("latest {n}").into_bytes());
        }
        assert!(begin.elapsed() < Duration::from_millis(250));
        assert_eq!(store.latest.as_deref(), Some(b"latest 999".as_slice()));
        assert_eq!(fs::read(&path).unwrap(), b"old checkpoint");
        assert!(store.writing.is_some());
        release.write_all(b"go").unwrap();
        store.flush(Instant::now() + Duration::from_secs(1));
        assert!(!store.pending());
        assert_eq!(fs::read(&path).unwrap(), b"latest 999");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stopping_during_a_pending_checkpoint_keeps_the_previous_record() {
        let root = std::env::temp_dir().join(format!(
            "fbt-watch-drop-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("watch.json");
        fs::write(&path, b"old durable").unwrap();
        let (gate, _held) = UnixStream::pair().unwrap();
        let mut store = Store::new(path.clone());
        store.writing = Some(
            Pending::begin(&path, b"uncommitted".to_vec(), |file| {
                FileSync::gated_for_test(file, &gate)
            })
            .unwrap(),
        );
        let begin = Instant::now();
        store.flush(begin + Duration::from_millis(30));
        assert!(store.pending());
        drop(store);
        assert!(begin.elapsed() < Duration::from_millis(250));
        assert_eq!(fs::read(&path).unwrap(), b"old durable");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
