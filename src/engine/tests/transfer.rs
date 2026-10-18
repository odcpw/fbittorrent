    #[test]
    fn loopback_transfer_obeys_cap_stops_with_app_and_resumes_exact_bytes() {
        use librqbit::{create_torrent, spawn_utils::BlockingSpawner, CreateTorrentOptions};
        use std::time::Instant;

        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-loopback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let seed_store = directory.join("seed-store");
        let seed_files = directory.join("seed-files");
        let client_files = directory.join("client-files");
        std::fs::create_dir_all(&seed_files).unwrap();
        std::fs::create_dir_all(&client_files).unwrap();
        let payload: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();
        std::fs::write(seed_files.join("fixture.bin"), &payload).unwrap();
        let mut seed = test_app(&seed_store, &seed_files, Ledger::default(), Some(0));
        let bytes = seed.dispatcher.runtime.block_on(async {
            create_torrent(
                &test_cx(),
                &seed_files.join("fixture.bin"),
                CreateTorrentOptions {
                    piece_length: Some(16 * 1024),
                    ..Default::default()
                },
                &BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap()
        });
        let mut add = request("torrents/add", &[("tags", SEED_TAG)]);
        add.file = Some(crate::ipc::Upload {
            name: "fixture.torrent".into(),
            bytes: bytes.to_vec(),
        });
        seed.dispatcher.dispatch(&add).unwrap().unwrap();
        let hash = seed.dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(seed.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = seed.dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(handle.stats().finished);
            record.hash
        });
        let cap = NO_UPLOAD_LIMIT.to_string();
        seed.dispatcher
            .dispatch(&request(
                "torrents/setUploadLimit",
                &[("hashes", &hash), ("limit", &cap)],
            ))
            .unwrap()
            .unwrap();
        let effective_upload_limit = seed.dispatcher.runtime.block_on(async {
            OwnedMutexGuard::lock(seed.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .options(false)
                .unwrap()
                .ratelimits
                .upload_bps
                .map(|limit| limit.get())
        });
        let addr = seed.dispatcher.inner.session.listen_addr().unwrap();
        let retained = seed.dispatcher.inner.session.clone();
        let dispatcher = seed.dispatcher();
        seed.serve(&Config::default(), move |req| {
            dispatcher.dispatch(req).unwrap()
        })
        .unwrap();
        let socket = seed_store.join("runtime/engine.sock");
        assert!(socket.exists());
        assert!(InstanceLock::acquire_in(seed_store.join("runtime")).is_err());

        let client_runtime = NativeRuntime::new().unwrap();
        let client = client_runtime
            .block_on(Session::new_with_opts(
                client_runtime.cx(),
                client_files.clone(),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    ..Default::default()
                },
            ))
            .unwrap();
        let began = Instant::now();
        let (handle, partial, sample_before, elapsed, observations) =
            client_runtime.block_on(async {
                let handle = client
                    .add_torrent(
                        &test_cx(),
                        // Resolve real BEP 9 metadata from the seed before payload.
                        AddTorrent::from_url(format!("magnet:?xt=urn:btih:{hash}")),
                        Some(AddTorrentOptions {
                            overwrite: true,
                            initial_peers: Some(vec![addr]),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap()
                    .into_handle()
                    .unwrap();
                let mut observations = Vec::new();
                let (partial, sample_before, elapsed) =
                    time::timeout(&test_cx(), Duration::from_secs(15), async {
                        // During initialization progress_bytes counts disk bytes checked,
                        // including invalid/missing payload. Wait for the engine's state
                        // notification before interpreting it as downloaded payload.
                        // Otherwise a full hash-check scan looks like an uncapped transfer.
                        handle.wait_until_initialized(&test_cx()).await.unwrap();
                        loop {
                            let before = began.elapsed().as_secs_f64();
                            let stats = handle.stats();
                            let after = began.elapsed().as_secs_f64();
                            observations.push(serde_json::json!({
                                "before_s": before, "after_s": after,
                                "state": format!("{:?}", stats.state),
                                "progress_bytes": stats.progress_bytes,
                            }));
                            if stats.progress_bytes >= 64 * 1024 {
                                // Retain this sample and its bracketing monotonic times;
                                // resampling after block_on returns can race later payload.
                                break (stats.progress_bytes, before, after);
                            }
                            time::sleep(&test_cx(), Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .unwrap();
                (handle, partial, sample_before, elapsed, observations)
            });
        let observed = serde_json::json!({
            "partial": partial, "payload_len": payload.len(),
            "configured_upload_limit_bps": NO_UPLOAD_LIMIT,
            "effective_upload_limit_bps": effective_upload_limit,
            "sample_before_s": sample_before, "sample_after_s": elapsed,
            "observer_timestamps": observations,
        });
        let is_partial = partial < payload.len() as u64;
        let within_cap = partial as f64 <= NO_UPLOAD_LIMIT as f64 * (elapsed + 1.5);
        engine_test_diagnostic(
            "observe partial loopback payload",
            "The first observation is smaller than the full payload.",
            observed.clone(),
            is_partial,
        );
        engine_test_diagnostic(
            "check loopback upload cap allowance",
            "Payload bytes do not exceed cap * (monotonic elapsed seconds + 1.5).",
            observed,
            within_cap,
        );
        assert!(
            within_cap,
            "payload rate exceeded the configured cap, allowing its initial token burst"
        );
        assert!(is_partial);
        let closed = Instant::now();
        drop(seed);
        assert!(closed.elapsed() < Duration::from_secs(5));
        assert!(retained.cancellation_token().is_cancelled());
        assert!(!socket.exists());
        assert!(std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err());
        let stopped_bytes = handle.stats().progress_bytes;
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            handle.stats().progress_bytes,
            stopped_bytes,
            "closing the app must stop payload transfer"
        );

        let saved: Ledger =
            serde_json::from_slice(&std::fs::read(seed_store.join("torrents.json")).unwrap())
                .unwrap();
        assert_eq!(saved.records[0].upload_limit, NO_UPLOAD_LIMIT);
        assert!(
            !saved.records[0].paused,
            "app shutdown must preserve the intent to resume"
        );
        let seed = test_app(&seed_store, &seed_files, saved, Some(addr.port()));
        client_runtime.block_on(async {
            client.pause(&test_cx(), &handle).await.unwrap();
            client.unpause(&test_cx(), &handle).await.unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(20),
                handle.wait_until_completed(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                rqbit_runtime::fs::read(client_files.join("fixture.bin"))
                    .await
                    .unwrap(),
                payload
            );
            assert_eq!(handle.stats().progress_bytes, payload.len() as u64);
            client.stop(&test_cx()).await.unwrap();
        });
        assert!(
            began.elapsed() >= Duration::from_secs(5),
            "the fixture cannot complete at an uncapped loopback speed"
        );
        drop(seed);
        drop(retained);
        drop(client);
        client_runtime
            .shutdown_timeout(Duration::from_secs(2))
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pending_metadata_keeps_reads_responsive_and_cancels_on_close() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::os::unix::net::UnixStream;
        use std::sync::mpsc;
        use std::time::Instant;

        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-metadata-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = test_app(
            &directory,
            &directory.join("downloads"),
            Ledger::default(),
            None,
        );
        let dispatcher = app.dispatcher();
        let (handled_tx, handled_rx) = mpsc::channel();
        app.serve(&Config::default(), move |request| {
            let result = dispatcher.dispatch(request).unwrap();
            handled_tx
                .send(
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                )
                .unwrap();
            result
        })
        .unwrap();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!("http://{}/pending.torrent", listener.local_addr().unwrap());
        let (connected_tx, connected_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            connected_tx.send(()).unwrap();
            // Keep the HTTP lookup pending until the app closes.
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            drop(stream);
        });
        // Exercise the real owned socket worker, whose join is part of app
        // teardown. The HTTP peer remains open throughout the shutdown check.
        let mut client = UnixStream::connect(directory.join("runtime/engine.sock")).unwrap();
        let frame = serde_json::to_vec(&request("torrents/add", &[("urls", &url)])).unwrap();
        client
            .write_all(&(frame.len() as u32).to_be_bytes())
            .unwrap();
        client.write_all(&frame).unwrap();
        connected_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let began = Instant::now();
        let info = app
            .dispatcher
            .dispatch(&request("torrents/info", &[]))
            .unwrap()
            .unwrap();
        assert_eq!(info, "[]");
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "metadata lookup must not hold the ledger lock"
        );
        let closing = Instant::now();
        drop(app);
        assert!(handled_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err()
            .contains("closing"));
        assert!(
            closing.elapsed() < Duration::from_secs(4),
            "metadata lookup and socket worker must drain when the app closes"
        );
        assert!(!directory.join("runtime/engine.sock").exists());
        drop(client);
        release_tx.send(()).unwrap();
        peer.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
