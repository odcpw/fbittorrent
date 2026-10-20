#[test]
fn ledger_tracker_add_remove_and_shutdown_survive_ring_setup_failure() {
    crate::durable_sync::fault::isolated(&[libc::ENOMEM, libc::EPERM], |errno| {
        crate::durable_sync::fault::deny_setup(errno);
        // Exercise the exact tracker mutation which failed in the parallel gate,
        // including its credential-alias and durable removal safety assertions.
        redacted_alias_removes_only_its_tracker_and_cannot_be_added_or_retargeted();

        let directory = review_directory("durable-fallback");
        let downloads = directory.join("downloads");
        let mut app = test_app(&directory, &downloads, Ledger::default(), None);
        std::fs::write(downloads.join("fallback.txt"), b"hello").unwrap();
        let mut torrent =
            b"d4:infod6:lengthi5e4:name12:fallback.txt12:piece lengthi16384e6:pieces20:".to_vec();
        torrent.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        torrent.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&torrent)
            .unwrap()
            .info_hash
            .as_string();
        let mut add = add_request(Vec::new(), Some(torrent));
        add.stopped = true;
        let dispatcher = app.dispatcher();
        dispatcher.execute(ops::EngineOp::Add(add.clone())).unwrap();
        dispatcher.runtime.block_on(async {
            let cx = dispatcher.runtime.cx();
            let handle = dispatcher.inner.handle(&hash).unwrap();
            time::timeout(
                cx,
                Duration::from_secs(2),
                handle.wait_until_initialized(cx),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(handle.stats().finished);
        });
        let tracker = "http://127.0.0.1:1/announce?key=private-fixture";
        let before = dispatcher.ledger_generation().current();
        dispatcher
            .execute(ops::EngineOp::EditTracker {
                hash: hash.clone(),
                url: tracker.into(),
                add: true,
            })
            .unwrap();
        assert!(dispatcher.ledger_generation().current() > before);
        let saved = read_ledger(&directory.join("torrents.json")).unwrap();
        assert_eq!(saved.records.len(), 1);
        assert!(saved.records[0].trackers.iter().any(|url| url == tracker));
        assert!(directory.join(format!("{hash}.torrent")).exists());
        let began = std::time::Instant::now();
        assert!(
            app.drain(Duration::from_secs(2)),
            "fallback durability must drain successfully"
        );
        assert!(began.elapsed() < Duration::from_secs(2));
        drop((dispatcher, app));

        // Re-open from the acknowledged ledger and saved metadata; no in-memory
        // state from the first engine can satisfy these operations.
        let mut app = test_app(&directory, &downloads, saved, None);
        let dispatcher = app.dispatcher();
        assert_eq!(rows(&dispatcher).len(), 1);
        dispatcher
            .execute(ops::EngineOp::Delete {
                hashes: hashes(&[&hash]),
                files: false,
            })
            .unwrap();
        assert!(read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .is_empty());
        assert_eq!(
            std::fs::read(downloads.join("fallback.txt")).unwrap(),
            b"hello"
        );
        dispatcher.execute(ops::EngineOp::Add(add)).unwrap();
        dispatcher
            .execute(ops::EngineOp::Delete {
                hashes: hashes(&[&hash]),
                files: true,
            })
            .unwrap();
        assert!(read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .is_empty());
        assert!(!downloads.join("fallback.txt").exists());
        assert!(!directory.join(format!("{hash}.torrent")).exists());
        let began = std::time::Instant::now();
        assert!(app.drain(Duration::from_secs(2)));
        assert!(began.elapsed() < Duration::from_secs(2));
        println!(
            "errno={errno}: durable add/tracker/reopen/remove-keep/remove-files; drained in {:?}",
            began.elapsed()
        );
        drop((dispatcher, app));
        std::fs::remove_dir_all(directory).unwrap();
    });
}
