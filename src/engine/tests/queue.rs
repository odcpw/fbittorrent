    fn add_queue_fixture(dispatcher: &EngineDispatcher, index: usize, seed: bool) -> String {
        let name = format!("queue-{index}.txt");
        if seed {
            std::fs::write(dispatcher.inner.default_folder.join(&name), b"hello").unwrap();
        }
        let mut bytes = format!(
            "d4:infod6:lengthi5e4:name{}:{}12:piece lengthi16384e6:pieces20:",
            name.len(),
            name
        )
        .into_bytes();
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"ee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        dispatcher
            .dispatch(&Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: if seed {
                    vec![
                        ("ratioLimit".into(), "-2".into()),
                        ("upLimit".into(), "0".into()),
                    ]
                } else {
                    vec![]
                },
                file: Some(crate::ipc::Upload {
                    name: format!("queue-{index}.torrent"),
                    bytes,
                }),
            })
            .unwrap()
            .unwrap();
        dispatcher.runtime.block_on(async {
            let handle = dispatcher.inner.handle(&hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        hash
    }

    fn active_queue(dispatcher: &EngineDispatcher) -> Vec<String> {
        dispatcher.runtime.block_on(async {
            let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap();
            ledger
                .records
                .iter()
                .filter_map(|record| {
                    let handle = dispatcher.inner.handle(&record.hash).unwrap();
                    (!handle.is_paused() && !handle.stats().finished).then(|| record.hash.clone())
                })
                .collect()
        })
    }

    #[test]
    fn queue_caps_downloads_preserves_fifo_pause_delete_completion_and_restart() {
        let directory = review_directory("queue-lifecycle");
        let downloads = directory.join("downloads");
        let app = test_app(&directory, &downloads, Ledger::default(), None);
        let dispatcher = app.dispatcher();
        let hashes: Vec<_> = (0..6)
            .map(|index| add_queue_fixture(&dispatcher, index, false))
            .collect();
        assert_eq!(active_queue(&dispatcher), hashes[..3]);
        let info: Vec<Torrent> = serde_json::from_str(
            &dispatcher
                .dispatch(&request("torrents/info", &[]))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            info.iter()
                .filter(|torrent| torrent.state == "queuedDL")
                .count(),
            3
        );
        let status: serde_json::Value = serde_json::from_str(
            &dispatcher
                .dispatch(&request("app/queue", &[]))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["active_downloads"], 3);
        assert_eq!(status["queued_downloads"], 3);
        assert!(dispatcher
            .dispatch(&request("app/setQueue", &[("max_active_downloads", "0")]))
            .unwrap()
            .is_err());
        assert_eq!(active_queue(&dispatcher), hashes[..3]);

        dispatcher
            .dispatch(&request("torrents/stop", &[("hashes", &hashes[0])]))
            .unwrap()
            .unwrap();
        assert_eq!(active_queue(&dispatcher), hashes[1..4]);
        dispatcher
            .dispatch(&request("torrents/start", &[("hashes", &hashes[0])]))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&dispatcher),
            hashes[..3],
            "resume keeps the original FIFO position"
        );
        dispatcher
            .dispatch(&request("torrents/delete", &[("hashes", &hashes[1])]))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&dispatcher),
            vec![hashes[0].clone(), hashes[2].clone(), hashes[3].clone()]
        );
        dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hashes[0]), ("id", "0"), ("priority", "0")],
            ))
            .unwrap()
            .unwrap();
        assert!(
            dispatcher
                .inner
                .handle(&hashes[0])
                .unwrap()
                .stats()
                .finished
        );
        assert!(dispatcher.inner.handle(&hashes[0]).unwrap().is_paused());
        assert_eq!(
            active_queue(&dispatcher),
            hashes[2..5],
            "selected-file completion must admit the next download"
        );
        dispatcher
            .dispatch(&request("torrents/stop", &[("hashes", &hashes[5])]))
            .unwrap()
            .unwrap();
        let saved = read_ledger(&directory.join("torrents.json")).unwrap();
        assert_eq!(
            saved
                .records
                .iter()
                .map(|record| &record.hash)
                .collect::<Vec<_>>(),
            vec![&hashes[0], &hashes[2], &hashes[3], &hashes[4], &hashes[5]]
        );
        assert!(
            !saved.records[0].paused,
            "automatic completion suspension must retain running intent"
        );
        assert!(
            saved.records.last().unwrap().paused,
            "manual pause must be durable"
        );
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        let app = test_app(&directory, &downloads, saved, None);
        app.dispatcher.runtime.block_on(async {
            for hash in [&hashes[0], &hashes[2], &hashes[3], &hashes[4], &hashes[5]] {
                let handle = app.dispatcher.inner.handle(hash).unwrap();
                time::timeout(
                    &test_cx(),
                    Duration::from_secs(5),
                    handle.wait_until_initialized(&test_cx()),
                )
                .await
                .unwrap()
                .unwrap();
            }
            app.dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&app.dispatcher), hashes[2..5]);
        assert!(app.dispatcher.inner.handle(&hashes[5]).unwrap().is_paused());
        app.dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hashes[0]), ("id", "0"), ("priority", "1")],
            ))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&app.dispatcher),
            vec![hashes[0].clone(), hashes[2].clone(), hashes[3].clone()],
            "expanding completed selection must respect the cap"
        );
        drop(app);
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_limit_changes_and_network_suspension_keep_intent_and_exclude_seeders() {
        let directory = review_directory("queue-limit");
        let downloads = directory.join("downloads");
        let app = test_app(&directory, &downloads, Ledger::default(), None);
        let dispatcher = app.dispatcher();
        let seeder = add_queue_fixture(&dispatcher, 10, true);
        let hashes: Vec<_> = (0..5)
            .map(|index| add_queue_fixture(&dispatcher, index, false))
            .collect();
        assert!(!dispatcher.inner.handle(&seeder).unwrap().is_paused());
        assert_eq!(active_queue(&dispatcher), hashes[..3]);
        dispatcher.runtime.block_on(async {
            dispatcher
                .inner
                .max_active_downloads
                .store(1, Ordering::Release);
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&dispatcher), hashes[..1]);
        assert!(
            !dispatcher.inner.handle(&seeder).unwrap().is_paused(),
            "finished seeders do not consume download slots"
        );
        let before = std::fs::read(directory.join("torrents.json")).unwrap();
        dispatcher.set_network_blocked(true).unwrap();
        assert!(active_queue(&dispatcher).is_empty());
        assert!(dispatcher.inner.handle(&seeder).unwrap().is_paused());
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            before,
            "network suspension must not rewrite intent"
        );
        dispatcher.set_network_blocked(false).unwrap();
        assert_eq!(active_queue(&dispatcher), hashes[..1]);
        dispatcher.runtime.block_on(async {
            dispatcher
                .inner
                .max_active_downloads
                .store(4, Ordering::Release);
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&dispatcher), hashes[..4]);
        let closing = std::time::Instant::now();
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        assert!(closing.elapsed() < Duration::from_secs(4));
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resume_unknown_completion_timestamp_explicitly_and_in_bulk() {
        let directory = review_directory("resume-unknown-completion");
        let hash = "0000000000000000000000000000000000000004";
        for completion_on in [-1, 0] {
            let mut record = review_record(hash, &directory);
            record.completion_on = completion_on;
            record.paused = true;
            save_parked_ledger(&directory, &Ledger {
                migrated_legacy: false, records: vec![record],
            }).unwrap();
            for selection in [hash, "all"] {
                let result: serde_json::Value = serde_json::from_str(&dispatch_parked_in(
                    &request("torrents/start", &[("hashes", selection)]), &directory)
                    .unwrap().unwrap()).unwrap();
                assert_eq!(result, serde_json::json!({"started": [hash], "skipped": []}));
                let saved = read_ledger(&directory.join("torrents.json")).unwrap();
                assert!(!saved.records[0].paused);
                assert_eq!(saved.records[0].completion_on, completion_on);
                dispatch_parked_in(&request("torrents/stop", &[("hashes", hash)]), &directory)
                    .unwrap().unwrap();
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resume_done_is_typed_and_bulk_start_reports_skips() {
        let directory = review_directory("resume-done");
        std::fs::create_dir_all(&directory).unwrap();
        let done_hash = "0000000000000000000000000000000000000001";
        let download_hash = "0000000000000000000000000000000000000002";
        let seed_hash = "0000000000000000000000000000000000000003";
        let mut done = review_record(done_hash, &directory);
        done.completion_on = 10;
        done.paused = true;
        let mut download = review_record(download_hash, &directory);
        download.paused = true;
        let mut seeder = done.clone();
        seeder.hash = seed_hash.into();
        seeder.ratio_limit = -2.0;
        save_parked_ledger(&directory, &Ledger {
            migrated_legacy: false, records: vec![done, download, seeder],
        }).unwrap();
        let before = std::fs::read(directory.join("torrents.json")).unwrap();
        let selected = format!("{download_hash}|{done_hash}");
        let error = dispatch_parked_in(&request("torrents/start", &[("hashes", &selected)]), &directory)
            .unwrap().unwrap_err();
        let typed = error.downcast_ref::<crate::agent::AppError>().unwrap_or_else(|| panic!("typed refusal: {error:#}"));
        assert_eq!(typed.code, crate::agent::Code::Failure);
        assert_eq!(typed.message, "already complete; seeding is off for this torrent");
        assert_eq!(typed.hint, Some(format!("fbittorrent seed {done_hash}")));
        assert_eq!(std::fs::read(directory.join("torrents.json")).unwrap(), before);
        let selected = format!("{done_hash}|{download_hash}|{seed_hash}");
        for args in [vec![("hashes", "all")], vec![("hashes", &selected), ("skip_complete", "true")]] {
            let result: serde_json::Value = serde_json::from_str(&dispatch_parked_in(
                &request("torrents/start", &args), &directory).unwrap().unwrap()).unwrap();
            assert_eq!(result, serde_json::json!({"started": [download_hash, seed_hash],
                "skipped": [{"hash": done_hash, "reason": "complete_no_seed"}]}));
            let saved = read_ledger(&directory.join("torrents.json")).unwrap();
            assert!(saved.records[0].paused);
            assert!(!saved.records[1].paused && !saved.records[2].paused);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
