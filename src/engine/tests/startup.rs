    fn hashing_fixture(saved_paused: bool) -> (AppEngine, PathBuf, String, u64) {
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
        std::fs::write(directory.join("torrents.json"), serde_json::to_vec(&Ledger {
            migrated_legacy: true, records: vec![record],
        }).unwrap()).unwrap();
        let cfg = Config { save_path: Some(downloads.to_string_lossy().into()), ..Default::default() };
        let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
        let app = start_in(&cfg, Lifecycle::default(), directory.clone(), instance, SessionOptions {
            dht: None, disable_trackers: true, disable_local_service_discovery: true,
            concurrent_init_limit: Some(1), ..Default::default()
        }).unwrap();
        (app, directory, hash, length)
    }

    #[test]
    fn stop_during_hashing_is_paused_and_can_resume() {
        use std::time::Instant;
        for saved_paused in [false, true] {
            let (app, directory, hash, length) = hashing_fixture(saved_paused);
            let dispatcher = app.dispatcher();
            let row = || -> Torrent {
                let rows: Vec<Torrent> = serde_json::from_str(&dispatcher.dispatch(&request("torrents/info", &[])).unwrap().unwrap()).unwrap();
                rows.into_iter().find(|row| row.hash == hash).unwrap()
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let checking = row();
                if checking.state == "checkingDL" && checking.completed > 0 && checking.completed < length as i64 {
                    println!("active hashing: state={} checked_bytes={}", checking.state, checking.completed);
                    break;
                }
                assert!(Instant::now() < deadline, "must stop during real hashing: {checking:?}");
                std::thread::sleep(Duration::from_millis(1));
            }
            dispatcher.dispatch(&request("torrents/stop", &[("hashes", &hash)])).unwrap().unwrap();
            let stopped = row();
            println!("stopped hashing (saved_paused={saved_paused}): state={} checked_bytes={}", stopped.state, stopped.completed);
            assert_eq!(stopped.state, "stoppedDL");
            assert!(stopped.is_stopped(), "the shared client must offer Resume");
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(row().completed, stopped.completed, "Stop must drain the real check, including a saved-paused check");
            let saved = read_ledger(&directory.join("torrents.json")).unwrap();
            assert!(saved.records[0].paused);
            dispatcher.dispatch(&request("torrents/start", &[("hashes", &hash)])).unwrap().unwrap();
            let handle = dispatcher.inner.handle(&hash).unwrap();
            dispatcher.runtime.block_on(async {
                time::timeout(dispatcher.runtime.cx(), Duration::from_secs(15),
                    handle.wait_until_initialized(dispatcher.runtime.cx())).await.unwrap().unwrap();
            });
            let resumed = row();
            println!("resumed hashing: state={} verified_bytes={}", resumed.state, resumed.completed);
            assert_eq!(resumed.state, "stalledDL");
            assert!(resumed.completed > 0 && resumed.completed < length as i64);
            assert!(!read_ledger(&directory.join("torrents.json")).unwrap().records[0].paused);
            drop(app);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn saved_paused_check_reports_activity_until_complete() {
        use std::time::Instant;
        let (app, directory, hash, length) = hashing_fixture(true);
        let dispatcher = app.dispatcher();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut active_observations = 0;
        loop {
            let rows: Vec<Torrent> = serde_json::from_str(&dispatcher.dispatch(&request("torrents/info", &[])).unwrap().unwrap()).unwrap();
            let row = rows.iter().find(|row| row.hash == hash).unwrap();
            assert!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].paused);
            match row.state.as_str() {
                "checkingDL" => {
                    if row.completed > 0 && row.completed < length as i64 {
                        active_observations += 1;
                    }
                }
                "stoppedDL" => {
                    assert!(active_observations > 0, "saved-paused verification must be shown as checking while active: {row:?}");
                    assert!(matches!(dispatcher.inner.handle(&hash).unwrap().stats().state, TorrentStatsState::Paused));
                    println!("saved-paused verification: {active_observations} active checking observations, then state={} verified_bytes={}", row.state, row.completed);
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
        std::fs::write(directory.join("torrents.json"), serde_json::to_vec(&Ledger { migrated_legacy: true, records }).unwrap()).unwrap();
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
        let list = || -> Vec<Torrent> {
            serde_json::from_str(&dispatcher.dispatch(&request("torrents/info", &[])).unwrap().unwrap()).unwrap()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let rows = loop {
            let rows = list();
            assert_eq!(rows.len(), 3);
            let resolving = rows.iter().find(|row| row.hash == magnet.hash).unwrap();
            assert_eq!(resolving.state, "metaDL");
            assert!(resolving.error_message.is_none());
            if rows.iter().any(|row| row.hash == bad.hash && row.state == "error") &&
                dispatcher.restore_outcomes().get(&valid.hash) == Some(&RestoreOutcome::Restored) { break rows; }
            assert!(Instant::now() < deadline, "rows: {rows:?}");
            std::thread::sleep(Duration::from_millis(20));
        };
        for row in &rows { println!("{} state={} error_message={:?}", row.hash, row.state, row.error_message); }
        let failed = rows.iter().find(|row| row.hash == bad.hash).unwrap();
        assert!(failed.error_message.as_ref().is_some_and(|message| message.contains("could not restore")));
        assert!(matches!(dispatcher.restore_outcomes().get(&bad.hash), Some(RestoreOutcome::Failed { reason: "restore_failed", .. })));
        // Verify the cached pending mapping without racing a five-byte hash check.
        let mut pending = dispatcher.inner.restores.lock().unwrap();
        let entry = pending.get_mut(&bad.hash).unwrap();
        let failed_outcome = std::mem::replace(&mut entry.outcome, RestoreOutcome::Checking);
        drop(pending);
        assert_eq!(dispatcher.inner.unrestored_snapshot(&bad).state, "checkingDL");
        assert_eq!(dispatcher.inner.unrestored_snapshot(&bad).error_message, failed.error_message);
        dispatcher.inner.restores.lock().unwrap().get_mut(&bad.hash).unwrap().outcome = failed_outcome;

        // A retry may spend another 90 seconds resolving metadata. Keep the
        // corruption diagnostic while it waits, and replace it only on a new
        // failure or clear it after successful restoration.
        std::fs::remove_file(&bad_path).unwrap();
        dispatcher.dispatch(&request("torrents/start", &[("hashes", &bad.hash)])).unwrap().unwrap();
        let retry = list().into_iter().find(|row| row.hash == bad.hash).unwrap();
        assert_eq!(retry.state, "metaDL");
        assert_eq!(retry.error_message, failed.error_message);
        println!("pending retry state={} error_message={:?}", retry.state, retry.error_message);
        dispatcher.dispatch(&request("torrents/stop", &[("hashes", &bad.hash)])).unwrap().unwrap();
        std::fs::write(&bad_path, &valid_bytes).unwrap(); // Real but mismatched metainfo.
        dispatcher.dispatch(&request("torrents/start", &[("hashes", &bad.hash)])).unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let row = list().into_iter().find(|row| row.hash == bad.hash).unwrap();
            assert!(row.error_message.is_some(), "a retry must retain a diagnostic: {row:?}");
            if row.state == "error" {
                assert!(row.error_message.as_ref().unwrap().contains("does not match saved record"));
                println!("failed retry state={} error_message={:?}", row.state, row.error_message);
                break;
            }
            assert!(Instant::now() < deadline, "{row:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&bad_path, repaired_bytes).unwrap();
        dispatcher.dispatch(&request("torrents/start", &[("hashes", &bad.hash)])).unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while dispatcher.restore_outcomes().get(&bad.hash) != Some(&RestoreOutcome::Restored) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(list().iter().find(|row| row.hash == bad.hash).unwrap().error_message.is_none());
        println!("repaired cache retried and restored");

        dispatcher.dispatch(&request("torrents/stop", &[("hashes", &magnet.hash)])).unwrap().unwrap();
        assert_eq!(list().iter().find(|row| row.hash == magnet.hash).unwrap().state, "stoppedDL");
        let deadline = Instant::now() + Duration::from_secs(1);
        while dispatcher.inner.restores.lock().unwrap()[&magnet.hash].task.as_ref().is_some_and(|task| !task.is_finished()) {
            assert!(Instant::now() < deadline, "stopped resolver must finish its cleanup");
            std::thread::sleep(Duration::from_millis(10));
        }
        dispatcher.dispatch(&request("torrents/start", &[("hashes", &magnet.hash)])).unwrap().unwrap();
        assert_eq!(list().iter().find(|row| row.hash == magnet.hash).unwrap().state, "metaDL");
        dispatcher.dispatch(&request("torrents/delete", &[("hashes", &magnet.hash)])).unwrap().unwrap();
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
