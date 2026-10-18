    fn dispatch_parked_in(request: &Request, directory: &Path) -> Option<Result<String>> {
        super::dispatch_parked_in(request, directory, &Lifecycle::default())
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
            ledger: Arc::new(Mutex::new(ledger)),
            additions: Arc::new(Mutex::new(())),
            directory: directory.to_path_buf(),
            default_folder: downloads.to_path_buf(),
            stopping: AtomicBool::new(false),
            max_active_downloads: AtomicUsize::new(3),
            network_blocked: AtomicBool::new(false),
            restores: std::sync::Mutex::new(HashMap::new()),
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
        let dispatcher = EngineDispatcher {
            generation,
            lifecycle: Lifecycle::default(),
            inner,
            runtime: runtime.handle(),
        };
        AppEngine {
            dispatcher,
            runtime: Some(runtime),
            server: None,
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
include!("queue.rs");
include!("persistence.rs");
include!("transfer.rs");
include!("live_limits.rs");
include!("legacy_wire.rs");
include!("wire_probe.rs");
