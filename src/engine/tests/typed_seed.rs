use super::*;
use librqbit::storage::{
    filesystem::FilesystemStorageFactory, BoxStorageFactory, StorageFactory, StorageFactoryExt,
    TorrentStorage,
};
use librqbit::{ManagedTorrentShared, TorrentMetadata};
use ops::{EngineOp, EngineResult, HashSelection};
use view::{TorrentState, UploadPolicy};

#[derive(Default)]
struct StorageCounts {
    creates: AtomicUsize,
    reads: AtomicUsize,
    bytes: AtomicUsize,
}
impl StorageCounts {
    fn snapshot(&self) -> (usize, usize, usize) {
        (
            self.creates.load(Ordering::SeqCst),
            self.reads.load(Ordering::SeqCst),
            self.bytes.load(Ordering::SeqCst),
        )
    }
}

// Instrument the real filesystem backend: every operation still reaches the
// production storage implementation. With one five-byte piece and no peers,
// one successful read is one full real check; any extra check is counted even
// if it begins and ends between state observations.
#[derive(Clone)]
struct CountedFilesystemFactory(Arc<StorageCounts>);
struct CountedFilesystem {
    storage: Box<dyn TorrentStorage>,
    counts: Arc<StorageCounts>,
}
impl StorageFactory for CountedFilesystemFactory {
    type Storage = CountedFilesystem;
    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> Result<Self::Storage> {
        self.0.creates.fetch_add(1, Ordering::SeqCst);
        Ok(CountedFilesystem {
            storage: Box::new(FilesystemStorageFactory::default().create(shared, metadata)?),
            counts: self.0.clone(),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}
impl TorrentStorage for CountedFilesystem {
    fn init(&mut self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> Result<()> {
        self.storage.init(shared, metadata)
    }
    fn pread_exact(&self, file: usize, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.counts.reads.fetch_add(1, Ordering::SeqCst);
        self.storage.pread_exact(file, offset, buf)?;
        self.counts.bytes.fetch_add(buf.len(), Ordering::SeqCst);
        Ok(())
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
            counts: self.counts.clone(),
        }))
    }
}

fn fixture(dormant: bool) -> (AppEngine, PathBuf, String, Arc<StorageCounts>) {
    let directory = review_directory("typed-seed");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("hello.txt"), b"hello").unwrap();
    let mut bytes =
        b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    let mut record = review_record(&hash, &downloads);
    record.name = "hello.txt".into();
    record.tags = "keep".into();
    record.completion_on = if dormant { 123 } else { 0 };
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
            default_storage_factory: Some(CountedFilesystemFactory(counts.clone()).boxed()),
            ..Default::default()
        },
    )
    .unwrap();
    if !dormant {
        let dispatcher = app.dispatcher();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while dispatcher.inner.handle(&hash).is_err() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        wait_for_check(&dispatcher, &hash);
    }
    (app, directory, hash, counts)
}

fn set_seed(dispatcher: &EngineDispatcher, hash: &str, on: bool) -> Result<EngineResult> {
    dispatcher.execute(EngineOp::SetSeed {
        hashes: HashSelection(vec![hash.into()]),
        on,
    })
}

fn row(dispatcher: &EngineDispatcher) -> view::TorrentView {
    dormant_rows(dispatcher).pop().unwrap()
}

fn observe(case: &str, action: &str, expect: &str, observed: serde_json::Value) {
    println!(
        "FBT-P116C-STEP {}",
        serde_json::json!({
            "case": case, "action": action, "expect": expect, "observed": observed, "ok": true,
        })
    );
}

#[test]
fn typed_seed_live_retains_handle_and_never_checks() {
    use std::os::unix::fs::MetadataExt;
    let (app, directory, hash, counts) = fixture(false);
    let dispatcher = app.dispatcher();
    let original = dispatcher.inner.handle(&hash).unwrap();
    let metadata = original.metadata.load_full().unwrap();
    let torrent_path = directory.join(format!("{hash}.torrent"));
    let stamp = |path: &Path| {
        let m = std::fs::metadata(path).unwrap();
        (m.ino(), m.mtime(), m.mtime_nsec())
    };
    let original_stamp = stamp(&torrent_path);
    let completion = row(&dispatcher).completed_at;
    assert!(completion.is_some());
    assert_eq!(counts.snapshot(), (1, 1, 5));
    assert_eq!(row(&dispatcher).state, TorrentState::Done);
    let mut states = vec![];
    for on in [false, true, false] {
        if on || !states.is_empty() {
            let saved = std::fs::read(directory.join("torrents.json")).unwrap();
            let before = row(&dispatcher);
            // The limiter and both durable/in-memory policies must stay put if
            // saving the requested seed intent fails in either direction.
            std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
            assert!(set_seed(&dispatcher, &hash, on).is_err());
            std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
            assert_eq!(
                std::fs::read(directory.join("torrents.json")).unwrap(),
                saved
            );
            let after = row(&dispatcher);
            assert_eq!(after.seed, before.seed);
            assert_eq!(after.upload_policy, before.upload_policy);
            assert_eq!(after.upload_limit_bps, before.upload_limit_bps);
            assert_eq!(after.state, before.state);
            observe(
                "typed_seed_live",
                "fail the durable seed-policy save",
                "A failed save preserves the current policy, cap and running state.",
                serde_json::json!({"requested_seed":on,"row":wire(&after)}),
            );
        }
        for _ in 0..2 {
            set_seed(&dispatcher, &hash, on).unwrap();
            let current = dispatcher.inner.handle(&hash).unwrap();
            assert!(Arc::ptr_eq(&original, &current));
            assert!(Arc::ptr_eq(
                &metadata,
                &current.metadata.load_full().unwrap()
            ));
            assert_eq!(stamp(&torrent_path), original_stamp);
            assert_eq!(counts.snapshot(), (1, 1, 5));
            let r = row(&dispatcher);
            assert!(r.checked && current.stats().finished);
            assert_eq!(r.completed_bytes, Some(5));
            assert_eq!(r.completed_at, completion);
            assert_eq!(r.seed, on);
            assert_eq!(
                r.state,
                if on {
                    TorrentState::Seeding
                } else {
                    TorrentState::Done
                }
            );
            assert_eq!(
                r.upload_policy,
                if on {
                    UploadPolicy::Unlimited
                } else {
                    UploadPolicy::Off
                }
            );
            assert_eq!(r.upload_limit_bps, Some(0));
            assert!(!matches!(
                current.stats().state,
                TorrentStatsState::Initializing { .. }
            ));
            assert_eq!(current.is_paused(), !on);
            let saved = read_ledger(&directory.join("torrents.json")).unwrap();
            assert_eq!(saved.records[0].upload_limit, 0);
            assert_eq!(saved.records[0].ratio_limit, if on { -2.0 } else { 0.0 });
            assert_eq!(saved.records[0].tags, if on { "keep,seed" } else { "keep" });
            states.push(r.state);
            observe("typed_seed_live", "apply typed SetSeed twice to the same checked handle", "No read, new storage, handle/metadata replacement or checking; policy and pause state match seed intent.",
                serde_json::json!({"seed":on,"handle_id":current.id(),"creates_reads_bytes":counts.snapshot(),"row":wire(&r)}));
        }
    }
    assert_eq!(
        states,
        [
            TorrentState::Done,
            TorrentState::Done,
            TorrentState::Seeding,
            TorrentState::Seeding,
            TorrentState::Done,
            TorrentState::Done
        ]
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn typed_seed_dormant_checks_once_then_retains_handle() {
    let (app, directory, hash, counts) = fixture(true);
    let dispatcher = app.dispatcher();
    assert_eq!(counts.snapshot(), (0, 0, 0));
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 0);
    assert!(!row(&dispatcher).checked);
    set_seed(&dispatcher, &hash, false).unwrap();
    assert_eq!(counts.snapshot(), (0, 0, 0));
    set_seed(&dispatcher, &hash, true).unwrap();
    let admitted = dispatcher.inner.handle(&hash).unwrap();
    wait_for_check(&dispatcher, &hash);
    assert_eq!(counts.snapshot(), (1, 1, 5));
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 1);
    assert_eq!(
        dispatcher.restore_outcomes()[&hash],
        RestoreOutcome::Restored
    );
    let verified = row(&dispatcher);
    assert!(verified.checked && admitted.stats().finished);
    assert_eq!(verified.state, TorrentState::Seeding);
    assert_eq!(verified.completed_bytes, Some(5));
    assert_eq!(verified.upload_policy, UploadPolicy::Unlimited);
    assert_eq!(verified.upload_limit_bps, Some(0));
    observe("typed_seed_dormant", "activate a dormant completed torrent with typed SetSeed", "Exactly one real filesystem admission and one five-byte checksum read; checked seeding with unlimited policy.",
        serde_json::json!({"handle_id":admitted.id(),"creates_reads_bytes":counts.snapshot(),"row":wire(&verified)}));
    for on in [true, false, true, false] {
        set_seed(&dispatcher, &hash, on).unwrap();
        assert!(Arc::ptr_eq(
            &admitted,
            &dispatcher.inner.handle(&hash).unwrap()
        ));
        assert_eq!(counts.snapshot(), (1, 1, 5));
        let r = row(&dispatcher);
        assert!(r.checked);
        assert_eq!(
            r.state,
            if on {
                TorrentState::Seeding
            } else {
                TorrentState::Done
            }
        );
        assert_eq!(r.upload_limit_bps, Some(0));
        observe("typed_seed_dormant", "toggle seed policy after the dormant check", "Retain the admitted handle and the single check; apply unlimited/default caps without checking again.",
            serde_json::json!({"seed":on,"handle_id":admitted.id(),"creates_reads_bytes":counts.snapshot(),"row":wire(&r)}));
    }
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn recheck_dormant_exactly_one_check() {
    let (app, directory, hash, counts) = fixture(true);
    assert_eq!(counts.snapshot(), (0,0,0));
    let dispatcher = app.dispatcher();
    let EngineResult::Recheck(batch) = dispatcher.execute(EngineOp::RecheckBatch { hashes: vec![hash.clone()] }).unwrap() else { panic!("recheck receipt"); };
    let began = std::time::Instant::now();
    let report = loop {
        if let Some(report) = batch.result() { break report; }
        assert!(began.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(report.rechecked, vec![hash.clone()]);
    assert_eq!(report.have_bytes[&hash],5);
    assert!(report.failed.is_empty());
    assert_eq!(counts.snapshot(), (1,1,5), "one storage create and one 5-byte hash read");
    println!("dormant recheck storage creates/reads/bytes={:?}",counts.snapshot());
    drop(dispatcher); drop(app); std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_recheck_cannot_restore_the_old_completion_claim() {
    let (app,directory,hash,_counts) = fixture(false);
    let dispatcher=app.dispatcher();
    let path=directory.join(format!("{hash}.torrent"));
    let bytes=std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();std::fs::create_dir(&path).unwrap();
    let EngineResult::Recheck(batch)=dispatcher.execute(EngineOp::RecheckBatch {hashes:vec![hash.clone()]}).unwrap() else {panic!("receipt");};
    let began=std::time::Instant::now();
    while batch.result().is_none() {
        assert!(began.elapsed()<Duration::from_secs(5));std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(batch.result().unwrap().failed.len(),1);
    dispatcher.runtime.block_on(dispatcher.inner.maintain(dispatcher.runtime.cx())).unwrap();
    let row=dispatcher.runtime.block_on(dispatcher.info(dispatcher.runtime.cx())).unwrap().pop().unwrap();
    assert_eq!(row.state,TorrentState::Error);
    assert!(row.completed_at.is_none());
    assert_eq!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].completion_on,0);
    std::fs::remove_dir(&path).unwrap();std::fs::write(path,bytes).unwrap();
    println!("failed metainfo cache write: failed receipt, error row, completion stays invalidated under reconciliation");
    drop(dispatcher);drop(app);std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn force_on_finished_without_saved_timestamp_is_refused() {
    let (app,directory,hash,_counts)=fixture(false);
    let dispatcher=app.dispatcher();
    dispatcher.runtime.block_on(async {
        let mut ledger=OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(),dispatcher.runtime.cx()).await.unwrap();
        ledger.records[0].completion_on=0;
    });
    let failure=dispatcher.execute(EngineOp::ForceStart{hashes:HashSelection(vec![hash.clone()]),on:true}).unwrap_err();
    assert_eq!(failure.downcast_ref::<InvalidTorrentState>().unwrap().reason,"complete_no_seed");
    let mut ledger=dispatcher.runtime.block_on(OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(),dispatcher.runtime.cx())).unwrap();
    ledger.records[0].force=true;
    drop(ledger);
    dispatcher.runtime.block_on(dispatcher.inner.maintain(dispatcher.runtime.cx())).unwrap();
    let ledger=read_ledger(&directory.join("torrents.json")).unwrap();
    assert!(!ledger.records[0].force,"completion clears force durably");
    println!("verified-complete handle refuses force before its completion timestamp is saved; completion also clears a prior force intent");
    drop(dispatcher);drop(app);std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn undo_rechecks_real_data_once_without_redownload() {
    let (app,directory,hash,counts)=fixture(true);
    let dispatcher=app.dispatcher();
    assert_eq!(counts.snapshot(),(0,0,0));
    dispatcher.execute(EngineOp::RemoveKeep {hashes:HashSelection(vec![hash.clone()])}).unwrap();
    assert!(dormant_rows(&dispatcher).is_empty());
    dispatcher.execute(EngineOp::RestoreUndo {hash:hash.clone()}).unwrap();
    let deadline=std::time::Instant::now()+Duration::from_secs(10);
    loop {
        let row=row(&dispatcher);
        if row.state==TorrentState::Done && row.checked {break;}
        assert!(std::time::Instant::now()<deadline,"undo row: {row:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(counts.snapshot(),(1,1,5),"restore must perform one real hash check");
    assert_eq!(std::fs::read(directory.join("downloads/hello.txt")).unwrap(),b"hello");
    println!("undo: dormant removed, appended and checked; physical storage creates=1 reads=1 bytes=5, original payload untouched, no peers");
    drop(dispatcher);drop(app);std::fs::remove_dir_all(directory).unwrap();
}

#[path = "fast_resume.rs"]
mod fast_resume;
#[path="moving.rs"]
mod moving;

#[path = "dormant_operations.rs"]
mod dormant_operations;
