pub(crate) mod fixture;
use fixture::*;
    fn dispatch_parked_in(request: &Request, directory: &Path) -> Option<Result<String>> {
        fixture::dispatch_parked_in(request, directory, &Lifecycle::default())
    }

    include!("info_fixture.rs");

    fn test_cx() -> Cx {
        Cx::current().expect("native test runtime context")
    }

    use super::*;

    fn engine_test_diagnostic(action: &str, expect: &str, observed: serde_json::Value, ok: bool) {
        // The P0-17 runner adds the E.1 UTC timestamp, case and step and retains
        // this structured observation alongside the unmodified test output.
        eprintln!(
            "FBT-P017-STEP {}",
            serde_json::json!({
                "action": action, "expect": expect,
                "observed": observed.to_string(), "ok": ok,
            })
        );
    }

    pub(super) fn test_app(
        directory: &Path,
        downloads: &Path,
        ledger: Ledger,
        port: Option<u16>,
    ) -> AppEngine {
        let instance =
            InstanceLock::acquire_in(directory.join("runtime")).unwrap_or_else(|error| {
                use std::os::unix::fs::MetadataExt;
                let path = directory.join("runtime/engine.lock");
                let inode = std::fs::metadata(&path).map(|metadata| metadata.ino()).ok();
                let locks = std::fs::read_to_string("/proc/locks").ok().map(|locks| {
                    locks
                        .lines()
                        .filter(|line| {
                            inode.is_some_and(|inode| line.contains(&format!(":{inode} ")))
                        })
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                });
                engine_test_diagnostic(
                    "acquire isolated engine instance lock",
                    "The lock for this test's runtime directory is available.",
                    serde_json::json!({
                        "path": path, "inode": inode, "pid": std::process::id(),
                        "kernel_locks": locks, "error": format!("{error:#}"),
                    }),
                    false,
                );
                panic!("acquiring {}: {error:#}", path.display());
            });
        std::fs::create_dir_all(directory).unwrap();
        std::fs::create_dir_all(downloads).unwrap();
        let runtime = NativeRuntime::new().unwrap();
        let session = runtime
            .block_on(Session::new_with_opts(
                runtime.cx(),
                downloads.to_path_buf(),
                SessionOptions {
                    upload_enabled: false,
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    listen: port.map(|port| ListenerOptions {
                        mode: ListenerMode::TcpOnly,
                        listen_addr: ([127, 0, 0, 1], port).into(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ))
            .unwrap();
        let generation = ledger.generation.clone();
        let inner = Arc::new(Inner {
            session,
            bus_changed: rqbit_runtime::sync::Notify::new(),
            completed_count: Default::default(),
            completions: Default::default(),
            ledger: Arc::new(Mutex::new(ledger)),
            additions: Arc::new(Mutex::new(())),
            operations: Default::default(), moves: Default::default(),
            directory: directory.to_path_buf(),
            default_folder: downloads.to_path_buf(),
            rechecks: Default::default(),
            saved: Default::default(),
            resume: Arc::new(resume::Store::open(directory).unwrap()),
            resume_prepared: Default::default(),
            undo: Default::default(),
            rates: Default::default(),
            stopping: AtomicBool::new(false),
            max_active_downloads: AtomicUsize::new(3),
            queue_pins: Default::default(),
            network_blocked: AtomicBool::new(false),
            restores: std::sync::Mutex::new(HashMap::new()),
            retired_restores: std::sync::Mutex::new(Vec::new()),
            restore_join_started: std::sync::Mutex::new(None),
            cancelled_checks: std::sync::Mutex::new(HashMap::new()),
        });
        runtime.block_on(async {
            let records = OwnedMutexGuard::lock(inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records
                .clone();
            for record in records {
                inner
                    .restore_record(&test_cx(), &record, true)
                    .await
                    .unwrap();
            }
            inner.maintain(&test_cx()).await.unwrap();
        });
        // Mirror production start_shared's maintenance task: an active check
        // retains paused transfer intent until a post-check queue decision.
        // In particular a restored seed must resume after verification even
        // when this fixture's caller sends no additional engine operations.
        let maintenance = inner.clone();
        task::spawn_with(runtime.cx(), move |cx| async move {
            let mut interval = time::interval(&cx, Duration::from_secs(1));
            loop {
                if cx.checkpoint().is_err() { break; }
                interval.tick(&cx).await;
                if maintenance.stopping.load(Ordering::Acquire) { break; }
                if let Err(error) = maintenance.maintain(&cx).await {
                    if cx.checkpoint().is_err() || maintenance.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    panic!("test queue maintenance failed: {error:#}");
                }
            }
        });
        let dispatcher = EngineDispatcher {
            generation,
            lifecycle: Lifecycle::default(),
            inner,
            runtime: runtime.handle(),
        };
        AppEngine {
            dispatcher,
            runtime: Some(runtime),
            instance: Arc::new(instance),
        }
    }

    pub(super) fn request(path: &str, args: &[(&str, &str)]) -> Request {
        Request {
            method: "POST".into(),
            path: path.into(),
            args: args
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
            file: None,
        }
    }

    pub(super) fn review_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fbittorrent-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }


include!("ownership.rs");
include!("startup.rs");
include!("dormant.rs");
include!("dormant_delete.rs");
include!("tracker_edit.rs");
include!("metadata_cache.rs");
include!("queue.rs");
include!("persistence.rs");
include!("transfer.rs");
include!("live_limits.rs");
include!("handle_lookup.rs");
mod typed_seed {
    include!("typed_seed.rs");
    mod check_reconciliation {
        include!("check_reconciliation.rs");
    }
}
include!("legacy_wire.rs");
include!("wire_probe.rs");
include!("upload.rs");
include!("absent_completed.rs");
include!("live_presence.rs");
include!("parked_edits.rs");
