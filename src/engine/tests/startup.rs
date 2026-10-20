    fn hashing_fixture(saved_paused: bool, storage: Option<librqbit::storage::BoxStorageFactory>) -> (AppEngine, PathBuf, String, u64) {
        let directory = review_directory("stop-during-hashing");
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let name = "P0-09.hashing.bin";
        let length = 1024u64 * 1024 * 1024;
        let piece_length = 4 * 1024 * 1024;
        std::fs::File::create(downloads.join(name)).unwrap().set_len(length).unwrap();
        // SHA-1 of a real 4 MiB zero piece. The sparse payload is read and hashed
        // by librqbit; the deliberately wrong last hash keeps it incomplete.
        let piece_hash = [0x2b, 0xcc, 0xbd, 0x2f, 0x38, 0xf1, 0x5c, 0x13, 0xeb, 0x7d, 0x5a, 0x89, 0xfd, 0x9d, 0x85, 0xf5, 0x95, 0xe2, 0x3b, 0xc3];
        let mut pieces = piece_hash.repeat((length / piece_length) as usize);
        let last = pieces.len() - 20;
        pieces[last..].fill(0);
        let mut bytes = format!("d4:infod6:lengthi{length}e4:name{}:{}12:piece lengthi{piece_length}e6:pieces{}:", name.len(), name, pieces.len()).into_bytes();
        bytes.extend_from_slice(&pieces);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
        let mut record = review_record(&hash, &downloads);
        record.paused = saved_paused;
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        std::fs::write(directory.join("torrents.json"), serde_json::to_vec(&Ledger { generation: Default::default(),
            migrated_legacy: true, records: vec![record],
        }).unwrap()).unwrap();
        let cfg = Config { save_path: Some(downloads.to_string_lossy().into()), ..Default::default() };
        let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
        let app = start_in(&cfg, Lifecycle::default(), directory.clone(), instance, SessionOptions {
            dht: None, disable_trackers: true, disable_local_service_discovery: true,
            concurrent_init_limit: Some(1), default_storage_factory: storage, ..Default::default()
        }).unwrap();
        (app, directory, hash, length)
    }

    #[test]
    fn stop_can_persist_intent_with_both_checksum_workers_blocked() {
        use std::time::Instant;
        use typed_seed::check_reconciliation;
        // The production checker selects two workers on hosts with >=8 CPUs.
        if std::thread::available_parallelism().unwrap().get() < 8 {
            eprintln!("requires two production checksum workers (8 logical CPUs)");
            return;
        }
        let (factory, gates) = check_reconciliation::read_gates_storage(&[
            65536, 512 * 1024 * 1024 + 65536,
        ]);
        let (app, directory, hash, _) = hashing_fixture(false, Some(factory));
        let _release: Vec<_> = gates.iter().cloned()
            .map(check_reconciliation::ReleaseOnDrop).collect();
        for gate in &gates {
            gate.wait_blocked();
        }
        let dispatcher = app.dispatcher();
        assert_eq!(row(&dispatcher, &hash).state, view::TorrentState::Checking);
        let started = Instant::now();
        std::thread::scope(|scope| {
            let stop = scope.spawn(|| dispatcher.execute(ops::EngineOp::Stop {
                hashes: hashes(&[&hash]),
            }));
            while !gates.iter().all(|gate| gate.cancelled())
                && started.elapsed() < Duration::from_secs(1)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            let cancelled = gates.iter().all(|gate| gate.cancelled());
            for gate in &gates {
                gate.release();
            }
            stop.join().unwrap().unwrap();
            assert!(cancelled, "durable Stop must not queue behind both physical checksum workers");
        });
        assert!(started.elapsed() < Duration::from_secs(1));
        println!("both physical verifiers cancelled: stop_ms={}", started.elapsed().as_millis());
        assert!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].paused);
        assert_eq!(row(&dispatcher, &hash).state, view::TorrentState::Paused);
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stop_during_hashing_is_paused_and_can_resume() {
        use std::time::Instant;
        use typed_seed::check_reconciliation;
        for saved_paused in [false, true] {
            let (factory, gate) = check_reconciliation::read_gate_storage(65536);
            let (app, directory, hash, length) = hashing_fixture(saved_paused, Some(factory));
            let _release = check_reconciliation::ReleaseOnDrop(gate.clone());
            let dispatcher = app.dispatcher();
            gate.wait_blocked();
            let row = || crate::engine::tests::row(&dispatcher, &hash);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let checking = row();
                if checking.state == view::TorrentState::Checking && checking.completed_bytes.is_some_and(|bytes| bytes > 0 && bytes < length) {
                    println!("active hashing: state={:?} checked_bytes={:?}", checking.state, checking.completed_bytes);
                    break;
                }
                assert!(Instant::now() < deadline, "must stop during real hashing: {checking:?}");
                std::thread::sleep(Duration::from_millis(1));
            }
            let stop_check = || {
                let handle = dispatcher.inner.handle(&hash).unwrap();
                let stop_began = Instant::now();
                std::thread::scope(|scope| {
                    let stop = scope.spawn(|| dispatcher.execute(ops::EngineOp::Stop { hashes: crate::engine::tests::hashes(&[&hash]) }));
                    let deadline = Instant::now() + Duration::from_secs(1);
                    // Observe cancellation of the actual blocking worker's Cx,
                    // including a fresh stop of a saved-paused initialization.
                    while !gate.cancelled() {
                        if Instant::now() >= deadline { break; }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let cancelled = gate.cancelled();
                    gate.release();
                    stop.join().unwrap().unwrap();
                    assert!(cancelled, "stop must cancel physical verification before it completes");
                });
                assert!(stop_began.elapsed() < Duration::from_secs(1), "stopping an active verifier must stay prompt");
                assert!(handle.have_pieces_snapshot().is_none(),
                    "a cancelled check must not publish its partial have map");
                println!("physical verifier cancelled: stop_ms={} partial_have=false", stop_began.elapsed().as_millis());
            };
            stop_check();
            let stopped = row();
            println!("stopped hashing (saved_paused={saved_paused}): state={:?} checked_bytes={:?}", stopped.state, stopped.completed_bytes);
            assert_eq!(stopped.state, view::TorrentState::Paused);
            assert_eq!(stopped.intent, view::Intent::Pause, "the shared client must offer Resume");
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(row().completed_bytes, stopped.completed_bytes, "Stop must drain the real check, including a saved-paused check");
            let saved = read_ledger(&directory.join("torrents.json")).unwrap();
            assert!(saved.records[0].paused);
            dispatcher.execute(ops::EngineOp::Start { hashes: crate::engine::tests::hashes(&[&hash]), skip_complete: false }).unwrap();
            let handle = dispatcher.inner.handle(&hash).unwrap();
            dispatcher.runtime.block_on(async {
                time::timeout(dispatcher.runtime.cx(), Duration::from_secs(15),
                    handle.wait_until_initialized(dispatcher.runtime.cx())).await.unwrap().unwrap();
            });
            let resumed = row();
            println!("resumed hashing: state={:?} verified_bytes={:?}", resumed.state, resumed.completed_bytes);
            assert_eq!(resumed.state, view::TorrentState::Stalled);
            assert!(resumed.completed_bytes.is_some_and(|bytes| bytes > 0 && bytes < length));
            assert!(!read_ledger(&directory.join("torrents.json")).unwrap().records[0].paused);
            gate.arm();
            let batch = match dispatcher.execute(ops::EngineOp::RecheckBatch { hashes: vec![hash.clone()] }).unwrap() {
                ops::EngineResult::Recheck(batch) => batch,
                _ => panic!("expected pending recheck"),
            };
            gate.wait_blocked();
            assert!(batch.result().is_none(), "recheck must still be in physical verification");
            stop_check();
            let deadline = Instant::now() + Duration::from_secs(1);
            while batch.result().is_none() {
                assert!(Instant::now() < deadline, "cancelled recheck receipt must settle promptly");
                std::thread::sleep(Duration::from_millis(1));
            }
            let report = batch.result().unwrap();
            assert!(report.have_bytes.is_empty() && report.rechecked.is_empty() && report.failed.len() == 1,
                "cancelled recheck must never publish a partial have measurement: {report:?}");
            assert_eq!(row().state, view::TorrentState::Paused);
            assert_eq!(row().intent, view::Intent::Pause);
            drop(app);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn saved_paused_check_reports_activity_until_complete() {
        use std::time::Instant;
        use typed_seed::check_reconciliation;
        let (factory, gate) = check_reconciliation::read_gate_storage(65536);
        let (app, directory, hash, length) = hashing_fixture(true, Some(factory));
        let _release = check_reconciliation::ReleaseOnDrop(gate.clone());
        let dispatcher = app.dispatcher();
        gate.wait_blocked();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut active_observations = 0;
        loop {
            let row = crate::engine::tests::row(&dispatcher, &hash);
            assert!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].paused);
            match row.state {
                view::TorrentState::Checking => {
                    if row.completed_bytes.is_some_and(|bytes| bytes > 0 && bytes < length) {
                        active_observations += 1;
                        gate.release();
                    }
                }
                view::TorrentState::Paused => {
                    assert!(active_observations > 0, "saved-paused verification must be shown as checking while active: {row:?}");
                    assert!(matches!(dispatcher.inner.handle(&hash).unwrap().stats().state, TorrentStatsState::Paused));
                    println!("saved-paused verification: {active_observations} active checking observations, then state={:?} verified_bytes={:?}", row.state, row.completed_bytes);
                    break;
                }
                _ => panic!("unexpected saved-paused verification state: {row:?}"),
            }
            assert!(Instant::now() < deadline, "saved-paused check must finish: {row:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn bad_record_does_not_block() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Instant;

        let directory = review_directory("bad-record");
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        // A real local tracker keeps discovery open, but supplies no peers.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let tracker = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let tracker_stop = stop.clone();
        let server = std::thread::spawn(move || {
            while !tracker_stop.load(Ordering::Acquire) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    let body = b"d8:intervali3600e5:peers0:e";
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(body);
                } else { std::thread::sleep(Duration::from_millis(10)); }
            }
        });
        let make_cached = |name: &str| {
            let mut bytes = format!("d4:infod6:lengthi5e4:name{}:{}12:piece lengthi16384e6:pieces20:", name.len(), name).into_bytes();
            bytes.extend_from_slice(&[0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d]);
            bytes.extend_from_slice(b"7:privatei1eee");
            let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
            let mut record = review_record(&hash, &downloads);
            record.name = name.into();
            std::fs::write(downloads.join(name), b"hello").unwrap();
            (record, bytes)
        };
        let (valid, valid_bytes) = make_cached("valid.txt");
        let (mut bad, repaired_bytes) = make_cached("corrupt.txt");
        bad.source = format!("magnet:?xt=urn:btih:{}&tr=http%3A%2F%2F127.0.0.1%3A{}%2Fannounce", bad.hash, tracker.port());
        std::fs::write(directory.join(format!("{}.torrent", valid.hash)), &valid_bytes).unwrap();
        let bad_path = directory.join(format!("{}.torrent", bad.hash));
        std::fs::write(&bad_path, b"corrupt cache").unwrap();
        let mut magnet = review_record("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &downloads);
        magnet.source = format!("magnet:?xt=urn:btih:{}&tr=http%3A%2F%2F127.0.0.1%3A{}%2Fannounce", magnet.hash, tracker.port());
        let records = vec![magnet.clone(), bad.clone(), valid.clone()]; // Worst ledger order.
        std::fs::write(directory.join("torrents.json"), serde_json::to_vec(&Ledger { generation: Default::default(), migrated_legacy: true, records }).unwrap()).unwrap();
        let cfg = Config { save_path: Some(downloads.to_string_lossy().into()), ..Default::default() };
        let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
        let began = Instant::now();
        let app = start_in(&cfg, Lifecycle::default(), directory.clone(), instance, SessionOptions {
            dht: None, disable_local_service_discovery: true, concurrent_init_limit: Some(1), ..Default::default()
        }).unwrap();
        let duration = began.elapsed();
        println!("startup duration: {duration:?}");
        assert!(duration < Duration::from_secs(10));
        let dispatcher = app.dispatcher();
        {
            let tasks = dispatcher.inner.restores.lock().unwrap();
            assert!(tasks[&valid.hash].generation < tasks[&magnet.hash].generation);
            assert!(tasks[&bad.hash].generation < tasks[&magnet.hash].generation);
        }
        let list = || rows(&dispatcher);
        let start_one = |hash: &str| dispatcher.execute(ops::EngineOp::Start { hashes: crate::engine::tests::hashes(&[hash]), skip_complete: false }).unwrap();
        let stop_one = |hash: &str| dispatcher.execute(ops::EngineOp::Stop { hashes: crate::engine::tests::hashes(&[hash]) }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let rows = loop {
            let rows = list();
            assert_eq!(rows.len(), 3);
            let resolving = rows.iter().find(|row| row.hash == magnet.hash).unwrap();
            assert_eq!(resolving.state, view::TorrentState::Metadata);
            assert!(resolving.error.is_none());
            if rows.iter().any(|row| row.hash == bad.hash && row.state == view::TorrentState::Error) &&
                dispatcher.restore_outcomes().get(&valid.hash) == Some(&RestoreOutcome::Restored) { break rows; }
            assert!(Instant::now() < deadline, "rows: {rows:?}");
            std::thread::sleep(Duration::from_millis(20));
        };
        for row in &rows { println!("{} state={:?} error={:?}", row.hash, row.state, row.error); }
        let failed = rows.iter().find(|row| row.hash == bad.hash).unwrap();
        assert!(failed.error.as_ref().is_some_and(|message| message.contains("could not restore")));
        assert!(matches!(dispatcher.restore_outcomes().get(&bad.hash), Some(RestoreOutcome::Failed { reason: "restore_failed", .. })));
        // Verify the cached pending mapping without racing a five-byte hash check.
        let mut pending = dispatcher.inner.restores.lock().unwrap();
        let entry = pending.get_mut(&bad.hash).unwrap();
        let failed_outcome = std::mem::replace(&mut entry.outcome, RestoreOutcome::Checking);
        drop(pending);
        assert_eq!(dispatcher.inner.unrestored_view(&bad, true).state, view::TorrentState::Checking);
        assert_eq!(dispatcher.inner.unrestored_view(&bad, true).error, failed.error);
        dispatcher.inner.restores.lock().unwrap().get_mut(&bad.hash).unwrap().outcome = failed_outcome;

        // A retry may spend another 90 seconds resolving metadata. Keep the
        // corruption diagnostic while it waits, and replace it only on a new
        // failure or clear it after successful restoration.
        std::fs::remove_file(&bad_path).unwrap();
        start_one(&bad.hash);
        let retry = list().into_iter().find(|row| row.hash == bad.hash).unwrap();
        assert_eq!(retry.state, view::TorrentState::Metadata);
        assert_eq!(retry.error, failed.error);
        println!("pending retry state={:?} error={:?}", retry.state, retry.error);
        stop_one(&bad.hash);
        std::fs::write(&bad_path, &valid_bytes).unwrap(); // Real but mismatched metainfo.
        start_one(&bad.hash);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let row = list().into_iter().find(|row| row.hash == bad.hash).unwrap();
            assert!(row.error.is_some(), "a retry must retain a diagnostic: {row:?}");
            if row.state == view::TorrentState::Error {
                assert!(row.error.as_ref().unwrap().contains("does not match saved record"));
                println!("failed retry state={:?} error={:?}", row.state, row.error);
                break;
            }
            assert!(Instant::now() < deadline, "{row:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&bad_path, repaired_bytes).unwrap();
        start_one(&bad.hash);
        let deadline = Instant::now() + Duration::from_secs(10);
        while dispatcher.restore_outcomes().get(&bad.hash) != Some(&RestoreOutcome::Restored) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(list().iter().find(|row| row.hash == bad.hash).unwrap().error.is_none());
        println!("repaired cache retried and restored");

        stop_one(&magnet.hash);
        assert_eq!(list().iter().find(|row| row.hash == magnet.hash).unwrap().state, view::TorrentState::Paused);
        let deadline = Instant::now() + Duration::from_secs(1);
        while dispatcher.inner.restores.lock().unwrap()[&magnet.hash].task.as_ref().is_some_and(|task| !task.is_finished()) {
            assert!(Instant::now() < deadline, "stopped resolver must finish its cleanup");
            std::thread::sleep(Duration::from_millis(10));
        }
        start_one(&magnet.hash);
        assert_eq!(list().iter().find(|row| row.hash == magnet.hash).unwrap().state, view::TorrentState::Metadata);
        dispatcher.execute(ops::EngineOp::Delete { hashes: crate::engine::tests::hashes(&[&magnet.hash]), files: false }).unwrap();
        assert_eq!(list().len(), 2);
        assert!(!dispatcher.restore_outcomes().contains_key(&magnet.hash));
        // Leave a resolver active at shutdown, proving it belongs to this runtime.
        dispatcher.runtime.block_on(async {
            let cx = dispatcher.runtime.cx();
            let mut ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx).await.unwrap();
            ledger.records.push(magnet.clone());
            dispatcher.inner.spawn_restore(cx, magnet.clone()).unwrap();
        });
        let began = Instant::now();
        drop(app);
        assert!(began.elapsed() < Duration::from_secs(5));
        println!("active resolver shutdown: {:?}", began.elapsed());
        assert!(dispatcher.inner.restores.lock().unwrap().values().all(|restore| restore.task.as_ref().is_none_or(|task| task.is_finished())));
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    // The tracker is reachable at the socket layer but never answers its UDP
    // connect request, so no metadata or peers can arrive during shutdown.
    fn peerless_resolver_fixture() -> (AppEngine, PathBuf, Record, std::net::UdpSocket) {
        use std::net::UdpSocket;

        let directory = review_directory("resolver-teardown");
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&directory).unwrap();
        let tracker = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        tracker
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut record = review_record(hash, &downloads);
        record.source = format!(
            "magnet:?xt=urn:btih:{hash}&tr=udp%3A%2F%2F127.0.0.1%3A{}%2Fannounce",
            tracker.local_addr().unwrap().port()
        );
        std::fs::write(
            directory.join("torrents.json"),
            serde_json::to_vec(&Ledger {
                generation: Default::default(),
                migrated_legacy: true,
                records: vec![record.clone()],
            })
            .unwrap(),
        )
        .unwrap();
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
                disable_local_service_discovery: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut packet = [0; 2048];
        let (length, _) = tracker.recv_from(&mut packet).unwrap();
        assert_eq!(
            length, 16,
            "the real resolver sent a UDP tracker connect request"
        );
        assert_eq!(&packet[8..12], &0_u32.to_be_bytes());
        (app, directory, record, tracker)
    }

    struct ResolverCleanupPhase(Arc<AtomicUsize>);
    impl Drop for ResolverCleanupPhase {
        fn drop(&mut self) {
            if self.0.load(Ordering::Acquire) != 2 {
                self.0.store(3, Ordering::Release);
            }
        }
    }

    // Negative-control path: the former production order, with no restore join.
    // This switch only makes the regression fail; it cannot bypass an assertion.
    fn drain_with_former_order(app: &mut AppEngine, bound: Duration) -> bool {
        let began = std::time::Instant::now();
        app.dispatcher.inner.stopping.store(true, Ordering::Release);
        for restore in app.dispatcher.inner.restores.lock().unwrap().values() {
            restore.cancel.cancel();
        }
        let runtime = app.runtime.take().unwrap();
        let cx = runtime.cx();
        let stopped = matches!(
            runtime.block_on(time::timeout(
                cx,
                bound,
                app.dispatcher.inner.session.stop(cx)
            )),
            Ok(Ok(()))
        );
        let closed = runtime
            .shutdown_timeout(bound.saturating_sub(began.elapsed()))
            .is_ok();
        stopped && closed
    }

    fn assert_peerless_resolver_drains(retire: Option<bool>) {
        use std::time::Instant;
        let (mut app, directory, record, _tracker) = peerless_resolver_fixture();
        let dispatcher = app.dispatcher();
        let hash = record.hash.as_str();
        let cancel = dispatcher.inner.restores.lock().unwrap()[hash]
            .cancel
            .clone();
        assert!(!cancel.is_cancelled());
        assert_eq!(
            dispatcher.restore_outcomes()[hash],
            RestoreOutcome::Metadata
        );
        let joining = CancellationToken::new();
        *dispatcher.inner.restore_join_started.lock().unwrap() = Some(joining.clone());

        // The real resolver's epilogue waits for the join owner's explicit
        // signal. Application cancellation before that rendezvous gives phase
        // 3, even if cancellation drops the future before its final stores.
        // No sleep, resolver, tracker response or runtime is mocked.
        let resolver = dispatcher
            .inner
            .restores
            .lock()
            .unwrap()
            .get_mut(hash)
            .unwrap()
            .task
            .take()
            .unwrap();
        let phase = Arc::new(AtomicUsize::new(0));
        let cleanup_phase = phase.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let task = task::try_spawn_with(dispatcher.runtime.cx(), move |cx| async move {
            let _phase_guard = ResolverCleanupPhase(cleanup_phase.clone());
            started_tx.send(()).unwrap();
            let joined = resolver.await;
            cleanup_phase.store(1, Ordering::Release);
            joining.cancelled_with(&cx).await;
            cleanup_phase.store(
                if joined.is_ok() && joining.is_cancelled() && cx.checkpoint().is_ok() {
                    2
                } else {
                    3
                },
                Ordering::Release,
            );
        })
        .unwrap();
        dispatcher
            .inner
            .restores
            .lock()
            .unwrap()
            .get_mut(hash)
            .unwrap()
            .task = Some(task);
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if let Some(remove) = retire {
            dispatcher.inner.cancel_restore(hash, remove);
            if !remove {
                dispatcher
                    .inner
                    .spawn_restore(dispatcher.runtime.cx(), record.clone())
                    .unwrap();
            }
        }
        println!("resolver before drain: tracker=unanswered_udp_connect restore={:?} cleanup_phase={} retired={retire:?}",
                        dispatcher.restore_outcomes().get(hash), phase.load(Ordering::Acquire));
        let budget = Duration::from_millis(250);
        let began = Instant::now();
        let drained = if std::env::var_os("FBT_P103B_FORMER_ORDER").is_some() {
            drain_with_former_order(&mut app, budget)
        } else {
            app.drain(budget)
        };
        let elapsed = began.elapsed();
        println!("resolver after drain: cancelled={} restore={:?} cleanup_phase={} elapsed={elapsed:?} drained={drained}",
                        cancel.is_cancelled(), dispatcher.restore_outcomes().get(hash), phase.load(Ordering::Acquire));
        assert!(
            drained,
            "session and runtime must drain without their teardown deadline"
        );
        assert!(
            elapsed < budget,
            "resolver shutdown exceeded the short drain budget: {elapsed:?}"
        );
        assert!(
            cancel.is_cancelled(),
            "drop must cancel the in-flight resolver"
        );
        assert_eq!(
            phase.load(Ordering::Acquire),
            2,
            "the real resolver and its cleanup must join before application-region cancellation"
        );
        assert!(
            dispatcher
                .inner
                .restores
                .lock()
                .unwrap()
                .values()
                .all(|restore| restore.task.as_ref().is_some_and(|task| task.is_finished())),
            "retained restores keep their finished join handles"
        );
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn peerless_resolver_drains_before_runtime_deadline() {
        assert_peerless_resolver_drains(None);
    }

    #[test]
    fn removed_and_retried_resolvers_are_joined() {
        assert_peerless_resolver_drains(Some(true));
        assert_peerless_resolver_drains(Some(false));
    }

    fn join_timeout_probe() {
        use std::time::Instant;
        let (mut app, directory, record, _tracker) = peerless_resolver_fixture();
        let dispatcher = app.dispatcher();
        let hash = record.hash.as_str();
        let resolver = dispatcher
            .inner
            .restores
            .lock()
            .unwrap()
            .get_mut(hash)
            .unwrap()
            .task
            .take()
            .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (exited_tx, exited_rx) = std::sync::mpsc::channel();
        let task = task::try_spawn_with(dispatcher.runtime.cx(), move |cx| async move {
            resolver.await.unwrap();
            // Deliberately uncooperative cleanup, after a real resolver joined.
            // The owning test releases this barrier after observing the fuse.
            cx.checkpoint_with("blocked_restore_cleanup at src/engine/tests/startup.rs").unwrap();
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            exited_tx.send(()).unwrap();
        })
        .unwrap();
        {
            let mut restores = dispatcher.inner.restores.lock().unwrap();
            let restore = restores.get_mut(hash).unwrap();
            restore.task = Some(task);
            restore.cancel.cancel();
        }
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let budget = Duration::from_millis(50);
        let began = Instant::now();
        let drained = app.drain(budget);
        let elapsed = began.elapsed();
        let (present, unfinished, old_completion_assertion) = {
            let restores = dispatcher.inner.restores.lock().unwrap();
            let task = restores[hash].task.as_ref();
            (
                task.is_some(),
                task.is_some_and(|task| !task.is_finished()),
                restores
                    .values()
                    .all(|restore| restore.task.as_ref().is_none_or(|task| task.is_finished())),
            )
        };
        // Release our own worker before any assertion can panic.
        release_tx.send(()).unwrap();
        exited_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while dispatcher.inner.restores.lock().unwrap()[hash]
            .task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            assert!(Instant::now() < deadline, "released cleanup must finish");
            std::thread::yield_now();
        }
        println!("timeout probe: drained={drained} present={present} unfinished={unfinished} old_completion_assertion={old_completion_assertion} elapsed_ms={}", elapsed.as_millis());
        assert!(!drained, "the full teardown must report its join failure");
        // Permit scheduler/syscall overhead around the 50 ms fuse, never a 4 s wait.
        assert!(
            elapsed < Duration::from_millis(250),
            "teardown exceeded its short fuse: {elapsed:?}"
        );
        assert!(
            present && unfinished,
            "timeout must preserve the unfinished current handle"
        );
        assert!(
            !old_completion_assertion,
            "the original completion assertion must remain non-vacuous"
        );
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn join_timeout_preserves_unfinished_restore() {
        const CHILD: &str = "FBT_P103B_TIMEOUT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            join_timeout_probe();
            return;
        }
        // Expected engine error lines stay in a captured subprocess artifact;
        // unexpected panics/leaks still fail this test and the ordinary gate.
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine::tests::join_timeout_preserves_unfinished_restore",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "timeout-control subprocess hung: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "timeout control failed: {stdout}\n{stderr}"
        );
        assert!(
            stdout.contains(
                "drained=false present=true unfinished=true old_completion_assertion=false"
            ),
            "{stdout}"
        );
        assert!(stderr.contains("ERROR engine native torrent session did not drain before its deadline error=\"joining restore tasks\""), "{stderr}");
        let lines: Vec<_> = stderr.lines().collect();
        assert_eq!(
            lines.len(),
            5,
            "unexpected worker diagnostic in timeout control: {stderr}"
        );
        assert_eq!(lines[0], "ERROR engine native torrent session did not drain before its deadline error=\"joining restore tasks\"");
        assert_eq!(
            lines[1],
            "ERROR engine native torrent session did not drain before its deadline"
        );
        let failures = [
            "native application region did not drain before its deadline",
            "native runtime teardown exceeded its deadline",
        ];
        for (line, failure) in lines[2..4].iter().zip(failures) {
            let suffix = line
                .strip_prefix(&format!("ERROR engine {failure} elapsed_ms="))
                .unwrap_or_else(|| panic!("wrong deadline format: {line}"));
            let (elapsed, snapshot) = suffix.split_once(" outstanding=").unwrap();
            assert!(elapsed.parse::<u128>().unwrap() < 250, "{line}");
            let snapshot: serde_json::Value = serde_json::from_str(snapshot).unwrap();
            assert!(
                snapshot["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|task| task["checkpoint"]
                        == "blocked_restore_cleanup at src/engine/tests/startup.rs"),
                "{line}"
            );
        }
        let returned: String = serde_json::from_str(
            lines[4]
                .strip_prefix("ERROR engine closing native runtime: error=")
                .unwrap(),
        )
        .unwrap();
        assert!(
            failures.iter().all(|failure| returned.contains(failure)),
            "both phase errors must reach AppEngine: {returned}"
        );
        assert!(
            returned.contains("blocked_restore_cleanup at src/engine/tests/startup.rs"),
            "{returned}"
        );
        use base64::Engine;
        println!(
            "FBT-E3E-TIMEOUT-LOG {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{stdout}\n{stderr}"))
        );
        println!("Expected bounded join failure logged; unfinished current handle remained observable; cleanup barrier released.");
    }
