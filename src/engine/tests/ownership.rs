    #[test]
    fn two_engines_one_process() {
        use crate::daemon::lifecycle::{Request as LifecycleRequest, RestartReason};
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        fn open(label: &str, lifecycle: Lifecycle) -> (AppEngine, PathBuf, PathBuf) {
            let directory = review_directory(label);
            let socket = directory.join("runtime/engine.sock");
            let cfg = Config {
                save_path: Some(directory.join("downloads").to_string_lossy().into()),
                ..Default::default()
            };
            let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
            let mut app = start_in(&cfg, lifecycle.clone(), directory.clone(), instance, SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                ..Default::default()
            }).unwrap();
            let snapshot = crate::network_app::state(&cfg);
            lifecycle.set_expected_network(Some(snapshot.clone()));
            let dispatcher = app.dispatcher();
            app.serve(&cfg, move |request| {
                crate::network_app::controls(request, &snapshot, &lifecycle)
                    .or_else(|| dispatcher.dispatch(request)).unwrap()
            }).unwrap();
            println!("engine {label}: directory={} socket={}", directory.display(), socket.display());
            (app, directory, socket)
        }

        fn call(socket: &Path, path: &str) -> String {
            let mut stream = UnixStream::connect(socket).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let bytes = serde_json::to_vec(&request(path, &[])).unwrap();
            stream.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
            stream.write_all(&bytes).unwrap();
            let mut header = [0; 4];
            stream.read_exact(&mut header).unwrap();
            let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut bytes).unwrap();
            let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(response["error"].is_null(), "{response}");
            response["value"].as_str().unwrap().to_owned()
        }

        let first = Lifecycle::default();
        let second = Lifecycle::default();
        let (app_a, directory_a, socket_a) = open("lifecycle-a", first.clone());
        assert_eq!(call(&socket_a, "app/restart"), "Ok.");
        // Starting another real session must not clear the first owner's request.
        let (app_b, directory_b, socket_b) = open("lifecycle-b", second.clone());
        assert_ne!(directory_a, directory_b);
        assert_ne!(socket_a, socket_b);
        assert_eq!(first.pending_request(), Some(LifecycleRequest::Restart(RestartReason::Explicit)));
        assert_eq!(second.wait(Duration::ZERO), None);
        assert_eq!(call(&socket_a, "torrents/info"), "[]");
        assert_eq!(call(&socket_b, "torrents/info"), "[]");

        // Exercise the engine's own shutdown route as well as network_app's
        // control handler: both must target only the lifecycle passed to them.
        app_a.dispatcher().dispatch(&request("app/shutdown", &[])).unwrap().unwrap();
        assert_eq!(first.wait(Duration::ZERO), Some(LifecycleRequest::Stop(StopReason::Quit)));
        assert_eq!(second.pending_request(), None);
        drop(app_a);
        assert!(!socket_a.exists());
        assert_eq!(call(&socket_b, "torrents/info"), "[]");
        assert_eq!(call(&socket_b, "app/shutdown"), "Ok.");
        assert_eq!(second.wait(Duration::ZERO), Some(LifecycleRequest::Stop(StopReason::Quit)));
        assert_eq!(first.pending_request(), None);
        drop(app_b);
        assert!(!socket_b.exists());
        std::fs::remove_dir_all(directory_a).unwrap();
        std::fs::remove_dir_all(directory_b).unwrap();
    }

    #[test]
    fn parked_shutdown_uses_its_lifecycle() {
        use crate::daemon::lifecycle::Request as LifecycleRequest;
        let directory = review_directory("parked-lifecycle");
        let lifecycle = Lifecycle::default();
        assert_eq!(super::dispatch_parked_in(&request("app/shutdown", &[]), &directory, &lifecycle)
            .unwrap().unwrap(), "Ok.");
        assert_eq!(lifecycle.wait(Duration::ZERO), Some(LifecycleRequest::Stop(StopReason::Quit)));
    }

    #[test]
    fn engine_store_saves_are_private_for_new_and_reused_temporaries() {
        let directory = review_directory("private-engine");
        let app = test_app(&directory, &directory.join("downloads"), Ledger::default(), None);
        let hash = "1234567890123456789012345678901234567890";
        let names = ["torrents.json".to_string(), format!("{hash}.torrent"), "last-error.txt".into()];
        for replacement in [false, true] {
            if replacement {
                for name in &names {
                    std::fs::set_permissions(directory.join(name), std::fs::Permissions::from_mode(0o644)).unwrap();
                    let temporary = directory.join(format!("{name}.tmp"));
                    std::fs::write(&temporary, b"old temporary").unwrap();
                    std::fs::set_permissions(temporary, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            app.dispatcher.runtime.block_on(async {
                let inner = &app.dispatcher.inner;
                inner.save(&test_cx(), &Ledger::default()).await.unwrap();
                inner.write_metadata(&test_cx(), hash, b"private metadata").await.unwrap();
                inner.write_store(&test_cx(), "last-error.txt", b"private error\n").await.unwrap();
            });
            for name in &names {
                let path = directory.join(name);
                let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", path.display());
                assert!(!directory.join(format!("{name}.tmp")).exists());
                println!("{}: {mode:o} after live engine save", path.display());
            }
            assert_eq!(std::fs::read(directory.join(&names[1])).unwrap(), b"private metadata");
            assert!(read_ledger(&directory.join("torrents.json")).unwrap().records.is_empty());
        }
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parked_ledger_save_is_private_for_new_and_reused_temporaries() {
        let directory = review_directory("private-parked-ledger");
        for replacement in [false, true] {
            if replacement {
                for name in ["torrents.json", "torrents.json.tmp"] {
                    let path = directory.join(name);
                    std::fs::write(&path, b"old file").unwrap();
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            save_parked_ledger(&directory, &Ledger::default()).unwrap();
            let path = directory.join("torrents.json");
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            assert!(read_ledger(&path).unwrap().records.is_empty());
            assert!(!directory.join("torrents.json.tmp").exists());
            println!("{}: {mode:o} after parked engine save", path.display());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn instance_lock_excludes_the_same_directory_but_allows_another() {
        let directory = review_directory("instance-lock-directories");
        let first = InstanceLock::acquire_in(directory.join("first")).unwrap();
        let same = InstanceLock::acquire_in(directory.join("first"));
        let other = InstanceLock::acquire_in(directory.join("other"));
        engine_test_diagnostic(
            "check instance lock directory scope",
            "The same directory is excluded while another directory remains available.",
            serde_json::json!({"same_directory_excluded": same.is_err(),
                "other_directory_available": other.is_ok()}),
            same.is_err() && other.is_ok(),
        );
        assert!(same.is_err());
        assert!(other.is_ok());
        drop(first);
        drop(other);
        let reopened = InstanceLock::acquire_in(directory.join("first")).unwrap();
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropping_instance_lock_releases_a_fork_inherited_descriptor() {
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let directory = review_directory("instance-lock-inheritance");
        let instance = InstanceLock::acquire_in(directory.clone()).unwrap();
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let parent_fd = parent.as_raw_fd();
        let child_fd = child.as_raw_fd();
        // Model a parallel provider spawn parked between fork and exec. In the
        // child, use only async-signal-safe syscalls and _exit: other libtest
        // threads may have held allocator or runtime locks when fork happened.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            unsafe {
                libc::close(parent_fd);
                let mut byte = 1u8;
                if libc::write(child_fd, (&byte as *const u8).cast(), 1) != 1 {
                    libc::_exit(1);
                }
                let read = libc::read(child_fd, (&mut byte as *mut u8).cast(), 1);
                libc::_exit(if read == 1 { 0 } else { 1 });
            }
        }
        drop(child);
        let mut ready = [0u8];
        parent.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
        drop(instance);
        let reopened = InstanceLock::acquire_in(directory.clone());
        // Reap the child before asserting, including on the pre-fix failure.
        parent.write_all(&[1]).unwrap();
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        engine_test_diagnostic(
            "release instance lock while a forked child retains its descriptor",
            "Dropping the owner permits immediate reacquisition before the child exits.",
            serde_json::json!({"reacquired": reopened.is_ok(),
                "error": reopened.as_ref().err().map(|error| format!("{error:#}")),
                "child_pid": pid, "child_waited": waited, "child_status": status}),
            reopened.is_ok() && waited == pid && status == 0,
        );
        assert_eq!(waited, pid);
        assert_eq!(status, 0);
        assert!(
            reopened.is_ok(),
            "owner dropped but inherited lock remained: {:?}",
            reopened.err()
        );
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    pub(super) fn review_record(hash: &str, downloads: &Path) -> Record {
        Record {
            display_name:None,
            hash: hash.into(),
            name: "saved fixture".into(),
            source: String::new(),
            output_folder: downloads.to_string_lossy().into_owned(),
            only_files: None,
            upload_limit: NO_UPLOAD_LIMIT,
            trackers: Vec::new(), removed_trackers: Vec::new(), download_limit: None,
            ratio_limit: 0.0,
            category: String::new(),
            tags: String::new(), origin: "manual".into(),
            added_on: 1,
            completion_on: 0,
            paused: false,
            resume_after_migration: false, deferred: false, staged: false, force: false,
        }
    }
