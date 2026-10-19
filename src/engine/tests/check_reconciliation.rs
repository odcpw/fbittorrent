use super::*;
use librqbit::ManagedTorrentState;
use std::sync::{mpsc, Condvar, Mutex as StdMutex};

const PIECE: usize = 16 * 1024;

#[derive(Default)]
struct GateState {
    armed: bool,
    blocked: bool,
    released: bool,
    timed_out: bool,
}
#[derive(Default)]
struct ReadGate {
    state: StdMutex<GateState>,
    changed: Condvar,
}
impl ReadGate {
    fn arm(&self) {
        *self.state.lock().unwrap() = GateState {
            armed: true,
            ..Default::default()
        };
    }
    fn before_read(&self, offset: u64) -> Result<()> {
        if offset != PIECE as u64 {
            return Ok(());
        }
        let state = self.state.lock().unwrap();
        if !state.armed {
            return Ok(());
        }
        let mut state = state;
        state.armed = false;
        state.blocked = true;
        self.changed.notify_all();
        let (mut state, deadline) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.released)
            .unwrap();
        if deadline.timed_out() && !state.released {
            state.timed_out = true;
            bail!("test checksum-read gate was not released");
        }
        Ok(())
    }
    fn wait_blocked(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.blocked)
            .unwrap();
        assert!(state.blocked, "the real check must reach its second piece");
    }
    fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .released = true;
        self.changed.notify_all();
    }
}
struct ReleaseOnDrop(Arc<ReadGate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

// Reuse P1-16c's unchanged counters and production filesystem storage. The
// outer wrapper only holds the second real read until reconciliation completes.
#[derive(Clone)]
struct GatedFactory {
    counted: CountedFilesystemFactory,
    gate: Arc<ReadGate>,
}
struct GatedStorage {
    storage: Box<dyn TorrentStorage>,
    gate: Arc<ReadGate>,
}
impl StorageFactory for GatedFactory {
    type Storage = GatedStorage;
    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> Result<Self::Storage> {
        Ok(GatedStorage {
            storage: Box::new(self.counted.create(shared, metadata)?),
            gate: self.gate.clone(),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}
impl TorrentStorage for GatedStorage {
    fn init(&mut self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> Result<()> {
        self.storage.init(shared, metadata)
    }
    fn pread_exact(&self, file: usize, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.gate.before_read(offset)?;
        self.storage.pread_exact(file, offset, buf)
    }
    fn pwrite_all(&self, file: usize, offset: u64, buf: &[u8]) -> Result<()> {
        self.storage.pwrite_all(file, offset, buf)
    }
    fn remove_file(&self, file: usize, path: &Path) -> Result<()> {
        self.storage.remove_file(file, path)
    }
    fn remove_directory_if_empty(&self, path: &Path) -> Result<()> {
        self.storage.remove_directory_if_empty(path)
    }
    fn ensure_file_length(&self, file: usize, length: u64) -> Result<()> {
        self.storage.ensure_file_length(file, length)
    }
    fn take(&self) -> Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Self {
            storage: self.storage.take()?,
            gate: self.gate.clone(),
        }))
    }
}

fn gated_fixture() -> (
    AppEngine,
    PathBuf,
    String,
    Arc<StorageCounts>,
    Arc<ReadGate>,
) {
    let directory = review_directory("check-reconciliation");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let name = "incomplete.bin";
    std::fs::write(downloads.join(name), vec![b'a'; 2 * PIECE]).unwrap();
    let mut bytes = format!(
        "d4:infod6:lengthi{}e4:name{}:{name}12:piece lengthi{PIECE}e6:pieces40:",
        2 * PIECE,
        name.len()
    )
    .into_bytes();
    // SHA-1 of the first real 16 KiB 'a' piece. The wrong second hash leaves
    // the fixture genuinely incomplete after one full check of both pieces.
    bytes.extend_from_slice(&[
        0x12, 0xff, 0x34, 0x7b, 0x4f, 0x27, 0xd6, 0x9e, 0x1f, 0x32, 0x8e, 0x6f, 0x4b, 0x55, 0x73,
        0xe3, 0x66, 0x6e, 0x12, 0x2f,
    ]);
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    let mut record = review_record(&hash, &downloads);
    record.name = name.into();
    save_parked_ledger(
        &directory,
        &Ledger {
            generation: Default::default(),
            migrated_legacy: true,
            records: vec![record],
        },
    )
    .unwrap();
    std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
    let counts = Arc::new(StorageCounts::default());
    let gate = Arc::new(ReadGate::default());
    gate.arm();
    let cfg = Config {
        save_path: Some(downloads.to_string_lossy().into()),
        ..Default::default()
    };
    let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
    let app = start_in(
        &cfg,
        Lifecycle::default(),
        directory.clone(),
        instance,
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            fastresume: false,
            default_storage_factory: Some(
                GatedFactory {
                    counted: CountedFilesystemFactory(counts.clone()),
                    gate: gate.clone(),
                }
                .boxed(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    (app, directory, hash, counts, gate)
}

fn init_identity(handle: &ManagedTorrent) -> usize {
    handle.with_state(|state| match state {
        ManagedTorrentState::Initializing(init) => Arc::as_ptr(init) as usize,
        _ => panic!("expected the gated active check"),
    })
}

fn diagnostic(case: &str, action: &str, expect: &str, observed: serde_json::Value) {
    println!(
        "FBT-P116D-STEP {}",
        serde_json::json!({
            "case":case,"action":action,"expect":expect,"observed":observed,"ok":true,
        })
    );
}

fn reconcile_during_check(
    case: &str,
    dispatcher: &EngineDispatcher,
    hash: &str,
    counts: &Arc<StorageCounts>,
    gate: &Arc<ReadGate>,
    expected: (usize, usize, usize),
) -> Arc<ManagedTorrent> {
    gate.wait_blocked();
    let handle = dispatcher.inner.handle(hash).unwrap();
    let initial_check = init_identity(&handle);
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..2 {
        let dispatcher = dispatcher.clone();
        let hash = hash.to_owned();
        let counts = counts.clone();
        let handle = handle.clone();
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            let result = (|| -> Result<usize> {
                for _ in 0..16 {
                    dispatcher
                        .runtime
                        .block_on(dispatcher.inner.maintain(dispatcher.runtime.cx()))?;
                    let current = dispatcher.inner.handle(&hash)?;
                    anyhow::ensure!(
                        Arc::ptr_eq(&handle, &current),
                        "reconciliation replaced the handle"
                    );
                    anyhow::ensure!(
                        init_identity(&current) == initial_check,
                        "reconciliation replaced the active check"
                    );
                    anyhow::ensure!(
                        counts.snapshot() == expected,
                        "extra storage or checksum reads: {:?}",
                        counts.snapshot()
                    );
                    // The fork's scan counter advances at piece entry, before
                    // its read. Physical completed IO is measured separately
                    // by the unchanged P1-16c counters above.
                    anyhow::ensure!(
                        current.stats().progress_bytes == (2 * PIECE) as u64,
                        "scan progress changed behind the gate"
                    );
                    anyhow::ensure!(
                        !dispatcher.inner.check_cancelled(&hash, &current),
                        "active check was cancelled"
                    );
                }
                Ok(16)
            })();
            let _ = tx.send(result);
        }));
    }
    drop(tx);
    // Receive while the real read is still held. Old reconciliation waits for
    // cancellation of that blocked check and cannot return until we release it.
    let results: Vec<_> = (0..2)
        .map(|_| rx.recv_timeout(Duration::from_secs(2)))
        .collect();
    let observed = counts.snapshot();
    gate.release();
    for worker in workers {
        worker.join().unwrap();
    }
    let rounds: usize = results
        .into_iter()
        .map(|result| {
            result
                .expect("reconciliation must return while verification remains active")
                .unwrap()
        })
        .sum();
    assert_eq!(rounds, 32);
    assert_eq!(observed, expected);
    assert!(!gate.state.lock().unwrap().timed_out);
    diagnostic(case, "reconcile concurrently while a real checksum read is blocked",
        "Two threads finish 32 reconciliations without cancelling the active check, changing its identity, recreating storage or repeating reads.",
        serde_json::json!({"reconciliations":rounds,"creates_reads_bytes":observed,"read_prefix_bytes":PIECE,"scan_progress_bytes":2*PIECE,"handle_id":handle.id()}));
    handle
}

fn assert_checked_running(dispatcher: &EngineDispatcher, hash: &str) {
    wait_for_check(dispatcher, hash);
    let handle = dispatcher.inner.handle(hash).unwrap();
    let r = row(dispatcher);
    assert!(r.checked && !handle.is_paused() && !handle.stats().finished);
    assert_eq!(r.state, TorrentState::Stalled);
    assert_eq!(r.completed_bytes, Some(PIECE as u64));
    assert_eq!(r.size, Some(2 * PIECE as u64));
    assert!(r.completed_at.is_none());
    let legacy: Torrent = serde_json::from_str(&r.to_legacy_json().unwrap()).unwrap();
    assert_eq!(legacy.state, "stalledDL");
}

#[test]
fn incomplete_restore_reconciliation_checks_once() {
    let (app, directory, hash, counts, gate) = gated_fixture();
    let _release = ReleaseOnDrop(gate.clone());
    let dispatcher = app.dispatcher();
    let handle = reconcile_during_check(
        "incomplete_restore",
        &dispatcher,
        &hash,
        &counts,
        &gate,
        (1, 1, PIECE),
    );
    assert_checked_running(&dispatcher, &hash);
    assert!(Arc::ptr_eq(
        &handle,
        &dispatcher.inner.handle(&hash).unwrap()
    ));
    assert_eq!(counts.snapshot(), (1, 2, 2 * PIECE));
    diagnostic("incomplete_restore", "release verification and apply post-check running intent",
        "One storage creation and one full two-piece check; incomplete verified data runs without a second check.",
        serde_json::json!({"creates_reads_bytes":counts.snapshot(),"row":row(&dispatcher)}));
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_recheck_reconciliation_checks_once() {
    let (app, directory, hash, counts, gate) = gated_fixture();
    let _release = ReleaseOnDrop(gate.clone());
    let dispatcher = app.dispatcher();
    reconcile_during_check(
        "explicit_recheck",
        &dispatcher,
        &hash,
        &counts,
        &gate,
        (1, 1, PIECE),
    );
    assert_checked_running(&dispatcher, &hash);
    assert_eq!(counts.snapshot(), (1, 2, 2 * PIECE));
    let previous = dispatcher.inner.handle(&hash).unwrap();
    gate.arm();
    let actor = dispatcher.clone();
    let target = hash.clone();
    let recheck = std::thread::spawn(move || actor.execute(EngineOp::Recheck { hash: target }));
    let handle = reconcile_during_check(
        "explicit_recheck",
        &dispatcher,
        &hash,
        &counts,
        &gate,
        (2, 3, 3 * PIECE),
    );
    recheck.join().unwrap().unwrap();
    assert!(
        !Arc::ptr_eq(&previous, &handle),
        "the explicit recheck admits one fresh handle"
    );
    assert_checked_running(&dispatcher, &hash);
    assert!(Arc::ptr_eq(
        &handle,
        &dispatcher.inner.handle(&hash).unwrap()
    ));
    assert_eq!(counts.snapshot(), (2, 4, 4 * PIECE));
    diagnostic("explicit_recheck", "finish the explicit recheck under concurrent reconciliation",
        "Exactly one new admission and one two-piece check for the requested recheck; no additional storage recreation or verification.",
        serde_json::json!({"initial_creates_reads_bytes":[1,2,2*PIECE],"final_creates_reads_bytes":counts.snapshot(),"recheck_delta":[1,2,2*PIECE],"row":row(&dispatcher)}));
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn async_recheck_shares_outcome_without_blocking_ledger_or_queue() {
    let (app, directory, hash, counts, gate) = gated_fixture();
    let _release = ReleaseOnDrop(gate.clone());
    let dispatcher = app.dispatcher();
    gate.wait_blocked();
    gate.release();
    wait_for_check(&dispatcher, &hash);
    let before = counts.snapshot();
    gate.arm();
    let begin = || match dispatcher.execute(EngineOp::RecheckBatch { hashes: vec![hash.clone()] }).unwrap() {
        EngineResult::Recheck(batch) => batch,
        _ => panic!("expected verification receipt"),
    };
    let first = begin();
    {
        let state = gate.state.lock().unwrap();
        let (state, _) = gate.changed.wait_timeout_while(state, Duration::from_secs(5), |s| !s.blocked).unwrap();
        assert!(state.blocked, "second check must reach the read gate: report={:?}, counts={:?}, handle={:?}",
            first.result(), counts.snapshot(), dispatcher.inner.handle(&hash).map(|h|h.stats()));
    }
    let second = begin();
    assert!(first.result().is_none() && second.result().is_none());
    let row = dispatcher.runtime.block_on(dispatcher.info(dispatcher.runtime.cx())).unwrap().pop().unwrap();
    assert_eq!(row.state, TorrentState::Checking);
    assert!(!row.checked);
    let expected = counts.snapshot();
    for _ in 0..8 {
        dispatcher.runtime.block_on(dispatcher.inner.maintain(dispatcher.runtime.cx())).unwrap();
        assert_eq!(counts.snapshot(),expected);
    }
    let EngineResult::Queue(queue) = dispatcher.execute(EngineOp::Queue).unwrap() else { panic!("queue"); };
    assert_eq!(queue.active_downloads,0);
    assert_eq!(queue.queued_downloads,0);
    gate.release();
    let began = std::time::Instant::now();
    while first.result().is_none() || second.result().is_none() {
        assert!(began.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(first.result().unwrap().have_bytes,second.result().unwrap().have_bytes);
    assert_eq!(first.result().unwrap().have_bytes[&hash],PIECE as u64);
    let after=counts.snapshot();
    assert_eq!((after.0-before.0,after.1-before.1,after.2-before.2),(1,2,2*PIECE));
    println!("two pending requests share one real check; ledger reads/reconciliation stay responsive; check uses no queue slot");
    drop(dispatcher);drop(app);std::fs::remove_dir_all(directory).unwrap();
}
