// Native Live positioned-write failure test.
//
// This invokes actual PeerHandler.on_received_piece_frame -> StorageLease ->
// write_received_piece -> FileOps -> real filesystem storage. The sole fault
// seam is one selected positioned write. It observes the actual fatal oneshot
// and Have broadcast. It does not traverse TCP or the managed fatal receiver's
// transition to Error; those are separate app/interoperability cases.
#![cfg(not(feature = "_disable_disk_write_net_benchmark"))]

use super::*;
use crate::{
    AddTorrent, AddTorrentOptions, Session, SessionOptions,
    storage::{
        BoxStorageFactory, StorageFactory, StorageFactoryExt, TorrentStorage,
        filesystem::FilesystemStorageFactory,
    },
    torrent_state::ManagedTorrentState,
    vectored_traits::AsyncReadVectored,
};
use rqbit_runtime::{
    Budget,
    io::AsyncRead,
    runtime::{Runtime, RuntimeBuilder},
};
use sha1w::ISha1;
use std::{
    io::IoSlice,
    path::Path,
    pin::Pin,
    sync::{Mutex as TraceMutex, atomic::AtomicUsize},
    task::{Context as TaskContext, Poll},
};

const BOUND: Duration = Duration::from_secs(2);
const SIZE: usize = 8_192;
const FAULT: &str = "actual Live first positioned write fault";

#[derive(Debug, PartialEq, Eq)]
struct WriteAttempt {
    file: usize,
    offset: u64,
    length: usize,
}

#[derive(Default)]
struct FileTrace {
    armed: AtomicBool,
    attempts: TraceMutex<Vec<WriteAttempt>>,
    successful: TraceMutex<Vec<WriteAttempt>>,
    reads: AtomicUsize,
    completed: TraceMutex<Vec<u32>>,
}

impl FileTrace {
    fn before_write(&self, file: usize, offset: u64, length: usize) -> anyhow::Result<()> {
        if length == 0 {
            return Ok(());
        }
        self.attempts.lock().unwrap().push(WriteAttempt {
            file,
            offset,
            length,
        });
        if file == 0 && offset == 0 && self.armed.swap(false, Ordering::AcqRel) {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, FAULT).into());
        }
        Ok(())
    }
    fn successful_write(&self, file: usize, offset: u64, length: usize) {
        if length != 0 {
            self.successful.lock().unwrap().push(WriteAttempt {
                file,
                offset,
                length,
            });
        }
    }
}

#[derive(Clone)]
struct FirstWriteFaultFactory(Arc<FileTrace>);

impl StorageFactory for FirstWriteFaultFactory {
    type Storage = FirstWriteFaultStorage;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<Self::Storage> {
        Ok(FirstWriteFaultStorage {
            inner: Box::new(FilesystemStorageFactory::default().create(shared, metadata)?),
            trace: self.0.clone(),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

struct FirstWriteFaultStorage {
    inner: Box<dyn TorrentStorage>,
    trace: Arc<FileTrace>,
}

impl TorrentStorage for FirstWriteFaultStorage {
    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        self.inner.init(shared, metadata)
    }
    fn pread_exact(&self, file: usize, offset: u64, bytes: &mut [u8]) -> anyhow::Result<()> {
        self.trace.reads.fetch_add(1, Ordering::AcqRel);
        self.inner.pread_exact(file, offset, bytes)
    }
    fn pwrite_all(&self, file: usize, offset: u64, bytes: &[u8]) -> anyhow::Result<()> {
        self.trace.before_write(file, offset, bytes.len())?;
        self.inner.pwrite_all(file, offset, bytes)?;
        self.trace.successful_write(file, offset, bytes.len());
        Ok(())
    }
    fn pwrite_all_vectored(
        &self,
        file: usize,
        offset: u64,
        bytes: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        let length = bytes[0].len() + bytes[1].len();
        self.trace.before_write(file, offset, length)?;
        let written = self.inner.pwrite_all_vectored(file, offset, bytes)?;
        self.trace.successful_write(file, offset, written);
        Ok(written)
    }
    fn remove_file(&self, file: usize, filename: &Path) -> anyhow::Result<()> {
        self.inner.remove_file(file, filename)
    }
    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.inner.remove_directory_if_empty(path)
    }
    fn ensure_file_length(&self, file: usize, length: u64) -> anyhow::Result<()> {
        self.inner.ensure_file_length(file, length)
    }
    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            inner: self.inner.take()?,
            trace: self.trace.clone(),
        }))
    }
    fn on_piece_completed(&self, piece: ValidPieceIndex) -> anyhow::Result<()> {
        self.trace.completed.lock().unwrap().push(piece.get());
        self.inner.on_piece_completed(piece)
    }
}

struct NativeSession {
    runtime: Option<Runtime>,
    cleanup: Cx,
    session: Arc<Session>,
}

impl NativeSession {
    fn new(output: std::path::PathBuf, trace: Arc<FileTrace>) -> Self {
        let runtime = RuntimeBuilder::new()
            .worker_threads(1)
            .blocking_threads(1, 2)
            .enable_time()
            .build()
            .unwrap();
        let cleanup = runtime.request_cx_with_budget(Budget::INFINITE);
        let session = runtime
            .block_on(rqbit_runtime::time::timeout(
                &cleanup,
                BOUND,
                Session::new_with_opts(
                    &cleanup,
                    output,
                    SessionOptions {
                        dht: None,
                        disable_trackers: true,
                        disable_local_service_discovery: true,
                        runtime_worker_threads: Some(2),
                        default_storage_factory: Some(FirstWriteFaultFactory(trace).boxed()),
                        ..Default::default()
                    },
                ),
            ))
            .unwrap()
            .unwrap();
        Self {
            runtime: Some(runtime),
            cleanup,
            session,
        }
    }
    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.as_ref().unwrap().block_on(future)
    }
    fn finish(mut self) {
        let stopped = self.block_on(rqbit_runtime::time::timeout(
            &self.cleanup,
            BOUND,
            self.session.stop(&self.cleanup),
        ));
        let leaks = self
            .runtime
            .as_ref()
            .unwrap()
            .diagnostics()
            .find_leaked_obligations()
            .len();
        let joined = self.runtime.take().unwrap().shutdown_timeout(BOUND);
        assert!(
            matches!(stopped, Ok(Ok(()))),
            "real session stop: {stopped:?}"
        );
        assert_eq!(
            leaks, 0,
            "all real session/torrent/storage obligations settle"
        );
        assert!(
            joined,
            "native workers must join within the original 2 s bound"
        );
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.block_on(rqbit_runtime::time::timeout(
                &self.cleanup,
                BOUND,
                self.session.stop(&self.cleanup),
            ));
            let _ = runtime.shutdown_timeout(BOUND);
        }
    }
}

fn valid_payload_and_metainfo() -> (Vec<Vec<u8>>, Vec<u8>) {
    let payloads: Vec<Vec<u8>> = (0..3).map(|index| vec![0xa1 + index; SIZE]).collect();
    // One real 24 KiB file, three independently valid one-chunk pieces. All
    // wire frames and the control fence fit the original 32 KiB first read. A
    // wrongly entered later callback can complete/hash/Have a whole piece.
    let mut torrent =
        b"d4:infod6:lengthi24576e4:name11:payload.bin12:piece lengthi8192e6:pieces60:".to_vec();
    for payload in &payloads {
        let mut hash = sha1w::Sha1::new();
        hash.update(payload);
        torrent.extend_from_slice(&hash.finish());
    }
    torrent.extend_from_slice(b"ee");
    (payloads, torrent)
}

struct WireInput {
    wire: Vec<u8>,
    consumed: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
}

impl AsyncRead for WireInput {
    fn poll_read(
        self: Pin<&mut Self>,
        task: &mut TaskContext<'_>,
        output: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut slices = [std::io::IoSliceMut::new(output.unfilled())];
        match AsyncReadVectored::poll_read_vectored(self, task, &mut slices) {
            Poll::Ready(Ok(count)) => {
                output.advance(count);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncReadVectored for WireInput {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        _task: &mut TaskContext<'_>,
        outputs: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::AcqRel);
        let mut position = this.consumed.load(Ordering::Acquire);
        let mut written = 0;
        for output in outputs {
            let count = (this.wire.len() - position).min(output.len());
            output[..count].copy_from_slice(&this.wire[position..position + count]);
            position += count;
            written += count;
        }
        this.consumed.store(position, Ordering::Release);
        Poll::Ready(Ok(written))
    }
}

#[test]
fn native_live_first_positioned_write_error_signals_fatal_and_stops_later_valid_batch_effects() {
    let directory = tempfile::tempdir().unwrap();
    let trace = Arc::new(FileTrace::default());
    let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
    let (payloads, metainfo) = valid_payload_and_metainfo();
    let handle = fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            fixture.session.add_torrent(
                &fixture.cleanup,
                AddTorrent::from_bytes(metainfo),
                Some(AddTorrentOptions {
                    paused: true,
                    disable_trackers: true,
                    ..Default::default()
                }),
            ),
        ))
        .unwrap()
        .unwrap()
        .into_handle()
        .unwrap();
    fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            handle.wait_until_initialized(&fixture.cleanup),
        ))
        .unwrap()
        .unwrap();

    let run_cx = handle.run_region.lock().as_ref().unwrap().cx().clone();
    let paused = handle.with_state_mut(|state| match state.take() {
        ManagedTorrentState::Paused(paused) => paused,
        other => panic!(
            "real initialization must produce paused storage, got {}",
            other.name()
        ),
    });
    assert_eq!(
        paused
            .chunk_tracker
            .get_have_pieces()
            .as_slice()
            .count_ones(),
        0
    );
    let (fatal_tx, mut fatal_rx) = rqbit_runtime::channel::oneshot::channel();
    let live = TorrentStateLive::new(
        &run_cx,
        paused,
        fatal_tx,
        fixture.session.cancellation_token().child_token(),
    )
    .unwrap();
    {
        let mut managed = handle.locked.write();
        managed.paused = false;
        managed.state = ManagedTorrentState::Live(live.clone());
    }
    let mut have_rx = live.have_broadcast_tx.subscribe();
    let addr: SocketAddr = "127.0.0.1:45123".parse().unwrap();
    let (tx, _writer_rx) = unbounded_channel();
    let peer = Peer::new_live_for_incoming_connection(
        addr,
        Id20::new([0x61; 20]),
        tx.clone(),
        &live.peers,
        ConnectionKind::Tcp,
    );
    let counters = peer.stats.counters.clone();
    assert!(live.peers.states.insert(addr, peer).is_none());
    let chunks: Vec<ChunkInfo> = (0..3)
        .map(|index| {
            let piece = live.lengths.validate_piece_index(index).unwrap();
            live.lengths
                .chunk_info_from_received_data(piece, 0, SIZE as u32)
                .unwrap()
        })
        .collect();
    {
        let mut guard = live.lock_write("fatal-write-test reserve actual pieces");
        let priorities = guard.file_priorities.clone();
        for chunk in &chunks {
            let piece = chunk.piece_index;
            let acquired = guard
                .get_pieces_mut()
                .unwrap()
                .acquire_piece(AcquireRequest {
                    peer: addr,
                    peer_avg_time: None,
                    priority_pieces: std::iter::once(piece),
                    file_priorities: &priorities,
                    file_infos: &live.metadata.file_infos,
                    peer_has_piece: |candidate| candidate == piece,
                    can_steal: |_| false,
                });
            assert!(matches!(acquired, AcquireResult::Reserved(actual) if actual == piece));
        }
    }
    live.peers
        .with_live_mut(addr, "fatal-write-test seed actual requests", |peer| {
            for chunk in &chunks {
                assert!(peer.add_inflight_request(*chunk));
            }
            assert_eq!(peer.requested_inflight_count(), 3);
        })
        .unwrap();
    assert!(trace.attempts.lock().unwrap().is_empty());
    let reads_before = trace.reads.load(Ordering::Acquire);
    trace.armed.store(true, Ordering::Release);
    let mut wire = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        let mut frame = vec![0; 13 + payload.len()];
        let written = Message::Piece(Piece::from_data(index as u32, 0, payload))
            .serialize(&mut frame, &Default::default)
            .unwrap();
        assert_eq!(written, frame.len());
        wire.extend_from_slice(&frame);
    }
    // A buffered control fence makes intake immediate even if native
    // scheduling consumes the coalescing window before its final iteration.
    wire.extend_from_slice(&[0, 0, 0, 0]); // KeepAlive after the three valid Pieces.
    let total_wire = wire.len();
    let consumed = Arc::new(AtomicUsize::new(0));
    let input_polls = Arc::new(AtomicUsize::new(0));
    let (task_live, task_counters, task_consumed, task_polls) = (
        live.clone(),
        counters.clone(),
        consumed.clone(),
        input_polls.clone(),
    );
    let task = rqbit_runtime::task::spawn_with(&run_cx, move |original| async move {
        let storage = task_live.shared.spawner.lease(&original).await?;
        storage.run(&original, || ()).await?;
        assert!(
            storage.batch_domain(&original).is_some(),
            "actual warmed caller must take the batch path"
        );
        let handler = PeerHandler {
            storage,
            state: task_live.clone(),
            counters: task_counters,
            flow_control: Mutex::new(PeerFlowControl::default()),
            on_bitfield_notify: Notify::new(),
            addr,
            incoming: true,
            tx,
            first_message_received: AtomicBool::new(false),
            cancel_token: task_live.cancellation_token.child_token(),
            client_name_and_version: task_live.shared.client_name_and_version().to_owned(),
        };
        let mut input: BoxAsyncReadVectored = Box::new(WireInput {
            wire,
            consumed: task_consumed.clone(),
            polls: task_polls,
        });
        let mut receive = ReadBuf::new();
        receive.grow_for_piece_batch();
        let meta = {
            let Message::Piece(piece) = receive.read_message(&original, &mut input, BOUND).await?
            else {
                anyhow::bail!("expected actual first parsed Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        assert_eq!(
            task_consumed.load(Ordering::Acquire),
            total_wire,
            "all three valid frames are already buffered"
        );
        let frame = receive.into_piece_frame(meta.0, meta.1, meta.2, meta.3)?;
        // Actual Live intake/serial preparation/write, not a replacement consumer.
        handler
            .on_received_piece_frame(&original, frame, &mut input)
            .await
            .map(|_| ())
    });
    let joined = fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, task));
    assert!(
        matches!(&joined, Ok(Ok(Err(error))) if format!("{error:#}").contains(FAULT)),
        "actual Live fatal result: {joined:?}"
    );
    let fatal = fatal_rx
        .try_recv()
        .expect("actual on_fatal_error must send its error before physical callback returns");
    assert!(
        format!("{fatal:#}").contains(FAULT),
        "actual fatal channel: {fatal:#}"
    );
    let io_error = fatal
        .downcast_ref::<std::io::Error>()
        .expect("the actual fatal channel preserves the backend I/O error type");
    assert_eq!(io_error.kind(), std::io::ErrorKind::WriteZero);
    assert_eq!(io_error.to_string(), FAULT);
    assert!(!trace.armed.load(Ordering::Acquire));
    assert_eq!(
        *trace.attempts.lock().unwrap(),
        vec![WriteAttempt {
            file: 0,
            offset: 0,
            length: SIZE
        }]
    );
    assert!(trace.successful.lock().unwrap().is_empty());
    assert_eq!(
        trace.reads.load(Ordering::Acquire),
        reads_before,
        "failed write must never enter checksum file reads"
    );
    assert!(trace.completed.lock().unwrap().is_empty());
    assert_eq!(consumed.load(Ordering::Acquire), total_wire);
    assert_eq!(
        input_polls.load(Ordering::Acquire),
        1,
        "one read loads every Piece and its buffered control fence; intake needs no later poll"
    );
    let actual_file = std::fs::read(directory.path().join("payload.bin")).unwrap();
    assert!(
        actual_file == vec![0; 3 * SIZE],
        "actual file must retain all zero bytes: len={}, nonzero={}",
        actual_file.len(),
        actual_file.iter().filter(|byte| **byte != 0).count()
    );
    assert_eq!(
        counters.fetched_chunks.load(Ordering::Acquire),
        1,
        "later Live preparation never enters"
    );
    assert_eq!(counters.fetched_bytes.load(Ordering::Acquire), SIZE as u64);
    assert_eq!(
        counters
            .downloaded_and_checked_pieces
            .load(Ordering::Acquire),
        0
    );
    let stats = live.stats_snapshot();
    assert_eq!(stats.downloaded_and_checked_bytes, 0);
    assert_eq!(stats.downloaded_and_checked_pieces, 0);
    assert_eq!(stats.fetched_bytes, SIZE as u64);
    live.peers
        .with_live(addr, |peer| {
            assert_eq!(peer.requested_inflight_count(), 2);
            assert!(!peer.inflight_requests_debug().contains(&chunks[0]));
            assert!(peer.inflight_requests_debug().contains(&chunks[1]));
            assert!(peer.inflight_requests_debug().contains(&chunks[2]));
        })
        .unwrap();
    {
        let guard = live.lock_read("fatal-write-test inspect actual tracker");
        assert!(guard.fatal_errors_tx.is_none());
        let tracker = guard.get_pieces().unwrap();
        assert_eq!(tracker.inflight_count(), 3);
        for chunk in &chunks {
            assert!(tracker.is_inflight(chunk.piece_index));
        }
        assert_eq!(
            tracker.chunks().get_have_pieces().as_slice().count_ones(),
            0
        );
        assert_eq!(tracker.chunks().get_remaining_bytes(), (3 * SIZE) as u64);
    }
    assert!(
        matches!(
            have_rx.try_recv(),
            Err(rqbit_runtime::channel::broadcast::TryRecvError::Empty)
        ),
        "no actual completion Have may be emitted"
    );

    // ChunkTracker does not expose chunk_status. After every observable
    // assertion, probe its actual public marking operation BEFORE pausing:
    // pause requeues inflight pieces and would clear the status under test.
    // Each one-chunk Piece must be Completed on FIRST marking, never
    // PreviouslyCompleted. This final diagnostic mutation cannot send Have,
    // write a file or enter another production Live callback.
    {
        let mut guard = live.lock_write("fatal-write-test probe actual prior chunk status");
        for (index, payload) in payloads.iter().enumerate() {
            let result = guard
                .get_pieces_mut()
                .unwrap()
                .mark_chunk_downloaded(&Piece::from_data(index as u32, 0, payload));
            assert!(
                matches!(result, Some(ChunkMarkingResult::Completed)),
                "piece {index} had no prior mark: {result:?}"
            );
        }
    }
    handle.pause().unwrap();
    fixture.finish();
}
