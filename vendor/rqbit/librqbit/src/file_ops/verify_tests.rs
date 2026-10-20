use super::*;
use crate::{TorrentMetaV1File, TorrentMetaV1Info};
use std::{fs::File, os::unix::fs::FileExt, path::Path};

// Real positioned filesystem reads. The only fault seam requests pause after a
// physical read, at the point where a partially verified map must stay private.
struct ReadFiles {
    files: Vec<File>,
    pause: AtomicBool,
    stop_after: AtomicU64,
}
impl TorrentStorage for ReadFiles {
    fn pread_exact(&self, id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.files[id].read_exact_at(buf, offset)?;
        if self.stop_after.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.pause.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    fn init(
        &mut self,
        _: &crate::ManagedTorrentShared,
        _: &crate::TorrentMetadata,
    ) -> anyhow::Result<()> {
        unreachable!("read fixture")
    }
    fn pwrite_all(&self, _: usize, _: u64, _: &[u8]) -> anyhow::Result<()> {
        unreachable!("read fixture")
    }
    fn remove_file(&self, _: usize, _: &Path) -> anyhow::Result<()> {
        unreachable!("read fixture")
    }
    fn remove_directory_if_empty(&self, _: &Path) -> anyhow::Result<()> {
        unreachable!("read fixture")
    }
    fn ensure_file_length(&self, _: usize, _: u64) -> anyhow::Result<()> {
        unreachable!("read fixture")
    }
    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        unreachable!("read fixture")
    }
}

fn fixture() -> (
    tempfile::TempDir,
    ValidatedTorrentMetaV1Info<ByteBufOwned>,
    FileInfos,
    ReadFiles,
) {
    const PIECE: usize = 256 * 1024;
    // Uneven file boundaries, zero-length file and a short final piece.
    let sizes = [PIECE + 37, 0, 2 * PIECE - 11, PIECE + 101];
    let data: Vec<u8> = (0..sizes.iter().sum())
        .map(|i| (i * 31 + i / 17) as u8)
        .collect();
    let mut hashes = Vec::new();
    for bytes in data.chunks(PIECE) {
        let mut h = Sha1::new();
        h.update(bytes);
        hashes.extend(h.finish());
    }
    let info = TorrentMetaV1Info {
        name: Some(b"read-test".to_vec().into()),
        files: Some(
            sizes
                .iter()
                .enumerate()
                .map(|(i, len)| TorrentMetaV1File {
                    length: *len as u64,
                    path: vec![format!("{i}.bin").into_bytes().into()],
                    attr: None,
                    sha1: None,
                    symlink_path: None,
                })
                .collect(),
        ),
        pieces: hashes.into(),
        piece_length: PIECE as u32,
        ..Default::default()
    }
    .validate()
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut offset = 0;
    let mut files = Vec::new();
    let mut infos = Vec::new();
    for (index, len) in sizes.into_iter().enumerate() {
        let name = format!("{index}.bin");
        let path = directory.path().join(&name);
        std::fs::write(&path, &data[offset..offset + len]).unwrap();
        files.push(File::open(&path).unwrap());
        infos.push(FileInfo {
            relative_filename: name.into(),
            offset_in_torrent: offset as u64,
            piece_range: info
                .lengths()
                .iter_pieces_within_offset(offset as u64, len as u64),
            len: len as u64,
            attrs: Default::default(),
        });
        offset += len;
    }
    (
        directory,
        info,
        infos,
        ReadFiles {
            files,
            pause: AtomicBool::new(false),
            stop_after: AtomicU64::new(u64::MAX),
        },
    )
}

#[test]
fn verify_multifile_spanning_and_corrupt_piece() {
    let (dir, info, files, storage) = fixture();
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let check = || {
        FileOps::new(&info, &storage, &files)
            .initial_check(&cx, &AtomicU64::new(0), &storage.pause)
            .unwrap()
    };
    let before = check();
    assert_eq!(before.count_ones(), info.lengths().total_pieces() as usize);
    let parallel = |split| {
        let progress = AtomicU64::new(0);
        let ops = FileOps::new(&info, &storage, &files);
        std::thread::scope(|scope| {
            let progress = &progress;
            let left =
                scope.spawn(|| ops.initial_check_range(&cx, progress, &storage.pause, 0..split));
            let right = ops.initial_check_range(
                &cx,
                progress,
                &storage.pause,
                split..info.lengths().total_pieces(),
            );
            let mut left = left.join().unwrap().unwrap();
            left |= right.unwrap();
            assert_eq!(
                progress.load(Ordering::Relaxed),
                info.lengths().total_length()
            );
            left
        })
    };
    for split in 1..info.lengths().total_pieces() {
        assert_eq!(
            before,
            parallel(split),
            "intact files split at piece {split}"
        );
    }
    File::options()
        .write(true)
        .open(dir.path().join("2.bin"))
        .unwrap()
        .write_all_at(b"wrong", 2)
        .unwrap();
    let after = check();
    for split in 1..info.lengths().total_pieces() {
        assert_eq!(
            after,
            parallel(split),
            "corrupt files split at piece {split}"
        );
    }
    for piece in 0..info.lengths().total_pieces() as usize {
        assert_eq!(
            after[piece],
            piece != 1,
            "only the corrupt spanning piece must be missing"
        );
    }
    let needed = crate::chunk_tracker::ChunkTracker::new(
        crate::bitv::BitV::into_dyn(after),
        before,
        *info.lengths(),
        &files,
    )
    .unwrap();
    let priorities = (0..files.len()).collect();
    assert_eq!(
        needed
            .iter_queued_pieces(&priorities, &files)
            .map(|p| p.get())
            .collect::<Vec<_>>(),
        [1, 1],
        "the bad shared piece remains queued for download in each file priority"
    );
}

#[test]
fn verify_cancel_does_not_return_partial_have() {
    let (_dir, info, files, storage) = fixture();
    storage.stop_after.store(5, Ordering::Relaxed);
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let progress = AtomicU64::new(0);
    let began = std::time::Instant::now();
    let ops = FileOps::new(&info, &storage, &files);
    let results = std::thread::scope(|scope| {
        let left = scope.spawn(|| ops.initial_check_range(&cx, &progress, &storage.pause, 0..2));
        let right = ops.initial_check_range(
            &cx,
            &progress,
            &storage.pause,
            2..info.lengths().total_pieces(),
        );
        [left.join().unwrap(), right]
    });
    assert!(
        results
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
            .is_err(),
        "partially verified maps must not be published"
    );
    assert!(progress.load(Ordering::Relaxed) > 0);
    assert!(began.elapsed() < std::time::Duration::from_secs(1));
}

#[path = "verify_tests/io_failure.rs"]
mod io_failure;
