//! A real peer supplies bytes only after the engine observer is armed. No API
//! sampler or periodic maintenance can rescue a missing activity notification.
use super::*;
use librqbit::storage::{
    filesystem::FilesystemStorageFactory, BoxStorageFactory, StorageFactory, StorageFactoryExt,
    TorrentStorage,
};
use librqbit::{ManagedTorrentShared, TorrentMetadata};
use std::sync::{mpsc, Condvar, Mutex as StdMutex};

#[derive(Default)]
struct Gate {
    state: StdMutex<(bool, bool, bool)>, // armed, blocked, released
    changed: Condvar,
}
impl Gate {
    fn read(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.0 {
            return Ok(());
        }
        state.1 = true;
        self.changed.notify_all();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |s| !s.2)
            .unwrap();
        anyhow::ensure!(state.2, "real peer payload gate was not released");
        Ok(())
    }
    fn wait_blocked(&self) {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), Duration::from_secs(5), |s| !s.1)
            .unwrap();
        assert!(
            state.1,
            "client must request real payload before arming observer"
        );
    }
    fn release(&self) {
        self.state.lock().unwrap().2 = true;
        self.changed.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[derive(Clone)]
struct GatedFactory(Arc<Gate>);
struct GatedStorage {
    storage: Box<dyn TorrentStorage>,
    gate: Arc<Gate>,
}
impl StorageFactory for GatedFactory {
    type Storage = GatedStorage;
    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> Result<Self::Storage> {
        Ok(GatedStorage {
            storage: Box::new(FilesystemStorageFactory::default().create(shared, metadata)?),
            gate: self.0.clone(),
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
        self.gate.read()?;
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

#[test]
fn receiving_payload_wakes_observer_without_a_sampler_tick() {
    const PIECE: usize = 16384;
    const COUNT: usize = 128;
    let root = std::env::temp_dir().join(format!(
        "fbt-receiving-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let seed = super::tests::test_app_with_session(
        &root.join("seed-state"),
        &root.join("seed"),
        Ledger::default(),
        SessionOptions {
            upload_enabled: false,
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: ([127, 0, 0, 1], 0).into(),
                enable_upnp_port_forwarding: false,
                ..Default::default()
            }),
            ..Default::default()
        },
        false,
    );
    let client = super::tests::test_app_with_session(
        &root.join("client-state"),
        &root.join("client"),
        Ledger::default(),
        SessionOptions {
            upload_enabled: false,
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
        false,
    );
    let gate = Arc::new(Gate::default());
    let release = Release(gate.clone()); // releases reads before either runtime drops on panic
    std::fs::write(root.join("seed/payload.bin"), vec![b'a'; PIECE * COUNT]).unwrap();
    let mut bytes = format!(
        "d4:infod6:lengthi{}e4:name11:payload.bin12:piece lengthi{PIECE}e6:pieces{}:",
        PIECE * COUNT,
        COUNT * 20
    )
    .into_bytes();
    for _ in 0..COUNT {
        bytes.extend_from_slice(&[
            0x12, 0xff, 0x34, 0x7b, 0x4f, 0x27, 0xd6, 0x9e, 0x1f, 0x32, 0x8e, 0x6f, 0x4b, 0x55,
            0x73, 0xe3, 0x66, 0x6e, 0x12, 0x2f,
        ]);
    }
    bytes.extend_from_slice(b"ee");
    let add = |app: &AppEngine, options| {
        app.dispatcher.runtime.block_on(async {
            let response = app
                .dispatcher
                .inner
                .session
                .add_torrent(
                    app.dispatcher.runtime.cx(),
                    AddTorrent::from_bytes(bytes.clone()),
                    Some(options),
                )
                .await
                .unwrap();
            let AddTorrentResponse::Added(_, handle) = response else {
                panic!("expected new handle")
            };
            time::timeout(
                app.dispatcher.runtime.cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(app.dispatcher.runtime.cx()),
            )
            .await
            .unwrap()
            .unwrap();
            handle
        })
    };
    let seed_handle = add(
        &seed,
        AddTorrentOptions {
            overwrite: true,
            upload_enabled: Some(true),
            ratelimits: LimitsConfig {
                upload_bps: NonZeroU32::new(32768),
                ..Default::default()
            },
            storage_factory: Some(GatedFactory(gate.clone()).boxed()),
            ..Default::default()
        },
    );
    assert!(seed_handle.stats().finished && !seed_handle.is_paused());
    gate.state.lock().unwrap().0 = true;
    let handle = add(
        &client,
        AddTorrentOptions {
            upload_enabled: Some(false),
            initial_peers: Some(vec![seed.dispatcher.inner.session.listen_addr().unwrap()]),
            ..Default::default()
        },
    );
    gate.wait_blocked();
    let (sender, receiver) = mpsc::sync_channel(32);
    let observed = handle.clone();
    let watch = client.dispatcher.bus_watch(Arc::new(move || {
        let receiving = observed
            .stats()
            .live
            .is_some_and(|live| live.download_speed.as_bytes() > 0);
        let _ = sender.try_send(receiving);
    }));
    assert!(
        !receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        "observer is armed while payload remains gated"
    );
    assert_eq!(handle.stats().live.unwrap().snapshot.fetched_bytes, 0);
    gate.release();
    let began = std::time::Instant::now();
    loop {
        let received = receiver
            .recv_timeout(Duration::from_secs(5).saturating_sub(began.elapsed()))
            .expect("receiving must wake observer without periodic sampling or completion");
        if received {
            break;
        }
    }
    let stats = handle.stats();
    assert!(
        !stats.finished,
        "completion cannot substitute for a receiving notification"
    );
    let live = stats.live.unwrap();
    assert!(live.snapshot.fetched_bytes > 0);
    assert_eq!(
        live.snapshot.uploaded_bytes, 0,
        "download client must never upload"
    );
    eprintln!(
        "first receiving wake after {:?}: {} bytes",
        began.elapsed(),
        live.snapshot.fetched_bytes
    );
    watch.abort();
    client.dispatcher.runtime.block_on(watch).ok();
    drop(release);
    drop(client);
    drop(seed);
    std::fs::remove_dir_all(root).unwrap();
}
