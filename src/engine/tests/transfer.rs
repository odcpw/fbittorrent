const TEST_UPLOAD_CAP: i64 = 32768; // Explicit fixture throttle, not the product default.
    fn resumed_transfer_observation(
        seed: &AppEngine,
        client: &ManagedTorrent,
        hash: &str,
        stage: &str,
        elapsed: Duration,
        ok: bool,
    ) {
        let peer_stats = |handle: &ManagedTorrent| {
            handle.live().map(|live| {
                live.per_peer_stats_snapshot(
                    serde_json::from_value(serde_json::json!({"state": "all"})).unwrap(),
                )
            })
        };
        let seed_handle = seed.dispatcher.inner.handle(hash).ok();
        engine_test_diagnostic(
            "observe resumed loopback transfer",
            "Record the client's identity/bytes and the seed/peer states through the unchanged completion deadline.",
            serde_json::json!({
                "stage": stage, "after_s": elapsed.as_secs_f64(), "hash": hash,
                "unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
                "client_id": client.id(), "client_stats": client.stats(),
                "client_peers": peer_stats(client),
                "client_live": client.live().map(|live| format!("{:p}", Arc::as_ptr(&live))),
                "seed_id": seed_handle.as_ref().map(|handle| handle.id()),
                "seed_stats": seed_handle.as_ref().map(|handle| handle.stats()),
                "seed_peers": seed_handle.as_ref().and_then(|handle| peer_stats(handle)),
                "seed_restores": format!("{:?}", seed.dispatcher.restore_outcomes()),
                "runtime_region": format!("{:?}", Cx::current().unwrap().region_id()),
            }),
            ok,
        );
    }

    async fn observe_resumed_completion(
        cx: &Cx,
        seed: &AppEngine,
        client: &ManagedTorrent,
        hash: &str,
        began: std::time::Instant,
    ) -> Result<()> {
        let mut completion = client.wait_until_completed(cx);
        let mut samples = time::interval(cx, Duration::from_millis(200));
        loop {
            rqbit_runtime::select! {
                result = &mut completion => return result,
                _ = samples.tick(cx) => resumed_transfer_observation(
                    seed, client, hash, "completion_sample", began.elapsed(), true),
            }
        }
    }

    async fn wait_for_restored_seed(cx: &Cx, handle: &ManagedTorrent) -> Result<()> {
        handle.wait_until_initialized(cx).await?;
        // Negative control: model the former fixture's missing readiness gate.
        // The dedicated regression must fail on its first-poll witness.
        if std::env::var_os("FBT_P102B_FORMER_ORDER").is_some() {
            return Ok(());
        }
        loop {
            cx.checkpoint()?;
            let stats = handle.stats();
            if let Some(error) = stats.error {
                bail!("restored seed failed: {error}");
            }
            // Verification can finish into Paused before the periodic queue
            // reconciliation applies saved seeding intent. A listening socket
            // and complete bytes alone do not mean it can accept this torrent.
            if stats.finished
                && matches!(stats.state, TorrentStatsState::Live)
                && !handle.is_paused()
            {
                return Ok(());
            }
            time::sleep(cx, Duration::from_millis(25)).await;
        }
    }

    #[test]
    fn restored_seed_readiness_waits_for_running_state() {
        use std::{future::Future, sync::mpsc};
        let directory = review_directory("restored-seed-readiness");
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("hello.txt"), b"hello").unwrap();
        let mut bytes =
            b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        let mut record = review_record(&hash, &downloads);
        record.ratio_limit = -2.0;
        record.tags = SEED_TAG.into();
        record.paused = false;
        let app = test_app(
            &directory,
            &downloads,
            Ledger {
                records: vec![record],
                ..Default::default()
            },
            None,
        );
        let dispatcher = app.dispatcher();
        let handle = dispatcher.inner.handle(&hash).unwrap();
        dispatcher.runtime.block_on(async {
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
        });
        // Real queue policy holds the verified torrent paused. No scheduling
        // delay is used to create or prove this state.
        dispatcher.set_network_blocked(true).unwrap();
        assert!(handle.stats().finished && handle.is_paused());
        assert!(matches!(handle.stats().state, TorrentStatsState::Paused));
        let (first_tx, first_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let observed = handle.clone();
        let waiter = task::spawn_with(dispatcher.runtime.cx(), move |cx| async move {
            let mut waiting = std::pin::pin!(wait_for_restored_seed(&cx, &observed));
            let mut first = Some(first_tx);
            let result = std::future::poll_fn(|poll| {
                let result = waiting.as_mut().poll(poll);
                if let Some(first) = first.take() {
                    first
                        .send(if result.is_pending() {
                            "pending"
                        } else {
                            "ready"
                        })
                        .unwrap();
                }
                result
            })
            .await;
            let _ = finished_tx.send(result);
        });
        let first_poll = first_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        engine_test_diagnostic(
            "hold seed readiness at a verified paused torrent",
            "Readiness polls Pending until real queue policy admits seeding.",
            serde_json::json!({"first_poll": first_poll, "finished": handle.stats().finished,
                "paused": handle.is_paused(), "state": format!("{:?}", handle.stats().state)}),
            first_poll == "pending",
        );
        assert_eq!(
            first_poll, "pending",
            "a verified paused seed is not ready to serve payload"
        );
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        dispatcher.set_network_blocked(false).unwrap();
        finished_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        dispatcher.runtime.block_on(waiter).unwrap();
        assert!(handle.stats().finished && !handle.is_paused());
        assert!(matches!(handle.stats().state, TorrentStatsState::Live));
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

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
        let seed = test_app(&seed_store, &seed_files, Ledger::default(), Some(0));
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
        add.file = Some(crate::engine::tests::fixture::Upload {
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
        let cap = TEST_UPLOAD_CAP.to_string();
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
            "configured_upload_limit_bps": TEST_UPLOAD_CAP,
            "effective_upload_limit_bps": effective_upload_limit,
            "sample_before_s": sample_before, "sample_after_s": elapsed,
            "observer_timestamps": observations,
        });
        let is_partial = partial < payload.len() as u64;
        let within_cap = partial as f64 <= TEST_UPLOAD_CAP as f64 * (elapsed + 1.5);
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
        assert!(InstanceLock::acquire_in(seed_store.join("runtime")).is_ok());
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
        assert_eq!(saved.records[0].upload_limit, TEST_UPLOAD_CAP);
        assert!(
            !saved.records[0].paused,
            "app shutdown must preserve the intent to resume"
        );
        // Stop the client run before reopening the peer's listening port. Then
        // await actual seed readiness: restore returns before verification and
        // the first post-check queue reconciliation have necessarily finished.
        // Readiness and reconnection share the existing 20 s completion budget.
        let former_order = std::env::var_os("FBT_P102B_FORMER_ORDER").is_some();
        let resume_began = Instant::now();
        if !former_order {
            client_runtime
                .block_on(client.pause(client_runtime.cx(), &handle))
                .unwrap();
        }
        let seed = test_app(&seed_store, &seed_files, saved, Some(addr.port()));
        let restored = seed.dispatcher.inner.handle(&hash).unwrap();
        if !former_order {
            seed.dispatcher.runtime.block_on(async {
                time::timeout(
                    &test_cx(),
                    Duration::from_secs(20).saturating_sub(resume_began.elapsed()),
                    wait_for_restored_seed(&test_cx(), &restored),
                )
                .await
                .unwrap()
                .unwrap();
            });
            assert!(restored.stats().finished && !restored.is_paused());
            assert!(matches!(restored.stats().state, TorrentStatsState::Live));
        }
        client_runtime.block_on(async {
            resumed_transfer_observation(
                &seed,
                &handle,
                &hash,
                if former_order {
                    "seed_restarted_before_client_resume"
                } else {
                    "seed_ready_before_client_resume"
                },
                began.elapsed(),
                true,
            );
            if former_order {
                // Exact former fixture sequence, retained only for the sampled
                // hypothesis control. It still has every original assertion.
                client.pause(&test_cx(), &handle).await.unwrap();
                client.unpause(&test_cx(), &handle).await.unwrap();
            }
            let completed = time::timeout(
                &test_cx(),
                if former_order {
                    Duration::from_secs(20)
                } else {
                    Duration::from_secs(20).saturating_sub(resume_began.elapsed())
                },
                async {
                    if !former_order {
                        client.unpause(&test_cx(), &handle).await.unwrap();
                    }
                    resumed_transfer_observation(
                        &seed,
                        &handle,
                        &hash,
                        "client_resumed",
                        began.elapsed(),
                        true,
                    );
                    observe_resumed_completion(&test_cx(), &seed, &handle, &hash, began).await
                },
            )
            .await;
            resumed_transfer_observation(
                &seed,
                &handle,
                &hash,
                "completion_wait_returned",
                began.elapsed(),
                matches!(completed, Ok(Ok(()))),
            );
            completed.unwrap().unwrap();
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
        use std::net::TcpListener;
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
        let app = test_app(
            &directory,
            &directory.join("downloads"),
            Ledger::default(),
            None,
        );
        let dispatcher = app.dispatcher();
        let (handled_tx, handled_rx) = mpsc::channel();
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
        // The API dispatch pool now owns the caller; exercise the same typed
        // operation while its real HTTP metadata fetch remains blocked.
        let pending = std::thread::spawn(move || {
            let result = dispatcher.dispatch(&request("torrents/add", &[("urls", &url)])).unwrap();
            handled_tx.send(result.map(|_| ()).map_err(|e| e.to_string())).unwrap();
        });
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
        pending.join().unwrap();
        assert!(
            closing.elapsed() < Duration::from_secs(4),
            "metadata lookup and dispatch worker must drain when the app closes"
        );
        assert!(InstanceLock::acquire_in(directory.join("runtime")).is_ok());
        release_tx.send(()).unwrap();
        peer.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
