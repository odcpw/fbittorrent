    #[test]
    fn parked_history_preserves_order_unknown_progress_and_durable_manual_intent() {
        let directory = review_directory("parked-queue");
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = b"d4:infod6:lengthi1e4:name11:offline.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let hash = librqbit::torrent_from_bytes(bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        let mut record = review_record(&hash, &directory.join("downloads"));
        record.completion_on = 10;
        record.ratio_limit = -2.0; // Completed records can resume when seeding is enabled.
        let ledger = Ledger { generation: Default::default(),
            migrated_legacy: false,
            records: vec![record],
        };
        save_parked_ledger(&directory, &ledger).unwrap();
        let info: serde_json::Value = serde_json::from_str(
            &dispatch_parked_in(&request("torrents/info", &[]), &directory)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(info[0]["state"], "waitingVPN");
        assert_eq!(info[0]["size"], 1);
        assert_eq!(info[0]["progress_known"], false);
        let files: serde_json::Value = serde_json::from_str(
            &dispatch_parked_in(&request("torrents/files", &[("hash", &hash)]), &directory)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(files[0]["name"], "offline.bin");
        assert_eq!(files[0]["progress_known"], false);
        dispatch_parked_in(&request("torrents/stop", &[("hashes", &hash)]), &directory)
            .unwrap()
            .unwrap();
        assert!(
            read_ledger(&directory.join("torrents.json"))
                .unwrap()
                .records[0]
                .paused
        );
        dispatch_parked_in(&request("torrents/start", &[("hashes", &hash)]), &directory)
            .unwrap()
            .unwrap();
        assert!(
            !read_ledger(&directory.join("torrents.json"))
                .unwrap()
                .records[0]
                .paused
        );
        assert!(dispatch_parked_in(
            &request(
                "torrents/add",
                &[(
                    "urls",
                    "magnet:?xt=urn:btih:0000000000000000000000000000000000000000"
                )]
            ),
            &directory
        )
        .unwrap()
        .is_err());
        assert!(dispatch_parked_in(
            &request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")]
            ),
            &directory
        )
        .unwrap()
        .is_err());
        let saved = std::fs::read(directory.join("torrents.json")).unwrap();
        std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
        assert!(
            dispatch_parked_in(&request("torrents/stop", &[("hashes", &hash)]), &directory)
                .unwrap()
                .is_err()
        );
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            saved
        );
        std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
        dispatch_parked_in(
            &request("torrents/delete", &[("hashes", &hash)]),
            &directory,
        )
        .unwrap()
        .unwrap();
        assert!(read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .is_empty());
        assert!(!directory.join(format!("{hash}.torrent")).exists());
        assert!(
            !directory.join("downloads").exists(),
            "offline mutations do not create a session or touch payload"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_records_preserve_the_original_ledger() {
        let directory = review_directory("invalid-ledger");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("torrents.json");
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let record = review_record(hash, &directory.join("downloads"));
        let mut invalid_hash = record.clone();
        invalid_hash.hash = "../outside".into();
        let samples = [
            b"{\"records\":[".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&Ledger { generation: Default::default(),
                migrated_legacy: false,
                records: vec![invalid_hash],
            })
            .unwrap(),
            serde_json::to_vec(&Ledger { generation: Default::default(),
                migrated_legacy: false,
                records: vec![record.clone(), record.clone()],
            })
            .unwrap(),
        ];
        for bytes in samples {
            std::fs::write(&path, &bytes).unwrap();
            assert!(read_ledger(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&Ledger { generation: Default::default(),
                migrated_legacy: false,
                records: vec![record],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read_ledger(&path).unwrap().records.len(), 1);
        assert!(!directory.join("torrents.json.tmp").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mismatched_saved_metadata_never_opens_download_files() {
        let directory = review_directory("metadata-admission");
        let store = directory.join("store");
        let downloads = directory.join("downloads");
        let app = test_app(&store, &downloads, Ledger::default(), None);
        let bytes = b"d4:infod6:lengthi1e4:name11:foreign.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let expected = "0000000000000000000000000000000000000000";
        assert_ne!(
            librqbit::torrent_from_bytes(bytes)
                .unwrap()
                .info_hash
                .as_string(),
            expected
        );
        std::fs::write(store.join(format!("{expected}.torrent")), bytes).unwrap();
        let record = review_record(expected, &downloads);
        app.dispatcher.runtime.block_on(async {
            let error = app
                .dispatcher
                .inner
                .restore_record(&test_cx(), &record, false)
                .await
                .err()
                .unwrap();
            assert!(error.to_string().contains("hash does not match"));
            assert_eq!(
                app.dispatcher
                    .inner
                    .session
                    .with_torrents(|torrents| torrents.count()),
                0
            );
            assert!(!downloads.join("foreign.bin").exists());
            let mut invalid = record;
            invalid.hash = "../outside".into();
            assert!(app
                .dispatcher
                .inner
                .restore_record(&test_cx(), &invalid, false)
                .await
                .is_err());
            assert_eq!(
                app.dispatcher
                    .inner
                    .session
                    .with_torrents(|torrents| torrents.count()),
                0
            );
        });
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restoring_downloads_preserves_upload_policy() {
        let record = Record {
            hash: "abc".into(),
            name: "fixture".into(),
            source: String::new(),
            output_folder: "/tmp/fixture".into(),
            only_files: Some(vec![1]),
            upload_limit: NO_UPLOAD_LIMIT,
            download_limit: None,
            ratio_limit: 0.0,
            category: String::new(),
            tags: String::new(),
            added_on: 1,
            completion_on: 0,
            paused: false,
            resume_after_migration: false, deferred: false,
        };
        let stored = serde_json::to_vec(&record).unwrap();
        let restored: Record = serde_json::from_slice(&stored).unwrap();
        let options = restored.options(restored.paused).unwrap();
        assert_eq!(
            options.ratelimits.upload_bps.unwrap().get(),
            NO_UPLOAD_LIMIT as u32
        );
        assert_eq!(options.only_files, Some(vec![1]));
        assert!(!options.paused);
        assert!(restored.stop_on_complete());
    }

    #[test]
    fn native_lifecycle_hashes_existing_data_stops_restores_and_deletes_durably() {
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-native-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("fixture.txt"), b"hello").unwrap();
        // A real five-byte v1 torrent, whose only piece is SHA1("hello").
        let mut torrent =
            b"d4:infod6:lengthi5e4:name11:fixture.txt12:piece lengthi16384e6:pieces20:".to_vec();
        torrent.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        torrent.extend_from_slice(b"ee");
        let make_app = |ledger: Ledger| test_app(&directory, &downloads, ledger, None);
        let app = make_app(Ledger::default());
        let dispatcher = app.dispatcher();
        dispatcher
            .dispatch(&Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: vec![],
                file: Some(crate::ipc::Upload {
                    name: "fixture.torrent".into(),
                    bytes: torrent,
                }),
            })
            .unwrap()
            .unwrap();
        let (hash, mut saved) = dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                handle.stats().finished,
                "existing bytes must pass real torrent verification"
            );
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
            assert!(
                handle.is_paused(),
                "manual download must stop at completion"
            );
            let duplicate = Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: vec![(
                    "urls".into(),
                    format!("magnet:?xt=urn:btih:{}", record.hash),
                )],
                file: None,
            };
            time::timeout(
                &test_cx(),
                Duration::from_secs(2),
                dispatcher.add(&test_cx(), &duplicate),
            )
            .await
            .expect("a saved magnet must not need peer metadata lookup")
            .unwrap();
            let saved: Ledger = serde_json::from_slice(
                &rqbit_runtime::fs::read(directory.join("torrents.json"))
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                saved.records.len(),
                1,
                "duplicate magnet must keep one durable record"
            );
            assert!(
                !saved.records[0].paused,
                "completion suspension preserves running intent"
            );
            assert!(saved.records[0].completion_on > 0);
            (record.hash, saved)
        });
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        // Restoring an unknown legacy date may mark it complete only after
        // the existing payload has passed the real hash check again.
        saved.records[0].completion_on = -1;
        save_parked_ledger(&directory, &saved).unwrap();
        let app = make_app(saved);
        app.dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(app.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = app.dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            app.dispatcher.inner.maintain(&test_cx()).await.unwrap();
            assert_eq!(handle.info_hash().as_string(), hash);
            assert!(handle.is_paused());
            assert!(OwnedMutexGuard::lock(app.dispatcher.inner.ledger.clone(), &test_cx())
                .await.unwrap().records[0].completion_on > 0);
            assert!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].completion_on > 0);
            assert_eq!(
                record
                    .options(record.paused)
                    .unwrap()
                    .ratelimits
                    .upload_bps
                    .unwrap()
                    .get(),
                32 * 1024
            );
            assert_eq!(
                rqbit_runtime::fs::read(downloads.join("fixture.txt"))
                    .await
                    .unwrap(),
                b"hello"
            );
        });
        let ledger_before = std::fs::read(directory.join("torrents.json")).unwrap();
        std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
        assert!(app
            .dispatcher
            .dispatch(&request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")],
            ))
            .unwrap()
            .is_err());
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            ledger_before
        );
        assert_eq!(
            std::fs::read(downloads.join("fixture.txt")).unwrap(),
            b"hello"
        );
        assert!(app.dispatcher.inner.handle(&hash).is_ok());
        std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
        app.dispatcher
            .dispatch(&request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")],
            ))
            .unwrap()
            .unwrap();
        assert!(!downloads.join("fixture.txt").exists());
        assert!(!app.dispatcher.inner.torrent_path(&hash).unwrap().exists());
        let deleted = read_ledger(&directory.join("torrents.json")).unwrap();
        assert!(deleted.records.is_empty());
        drop(app);
        let reopened = make_app(deleted);
        assert!(reopened.dispatcher.inner.handle(&hash).is_err());
        assert!(!downloads.join("fixture.txt").exists());
        drop(reopened);
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_import_preserves_files_and_refuses_renamed_layout_before_writes() {
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-migrate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = directory.join("store");
        let downloads = directory.join("downloads");
        let backup = directory.join("backup");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(downloads.join("original.txt"), b"hello").unwrap();
        let app = test_app(&store, &downloads, Ledger::default(), None);
        let bytes = app.dispatcher.runtime.block_on(async {
            librqbit::create_torrent(
                &test_cx(),
                &downloads.join("original.txt"),
                librqbit::CreateTorrentOptions::default(),
                &librqbit::spawn_utils::BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap()
        });
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(backup.join(format!("{hash}.torrent")), &bytes).unwrap();
        std::fs::rename(
            downloads.join("original.txt"),
            downloads.join("renamed.txt"),
        )
        .unwrap();
        let mut torrent = Torrent {
            hash: hash.clone(),
            name: "renamed.txt".into(),
            state: "stoppedUP".into(),
            progress: 1.0,
            size: 5,
            completed: 5,
            save_path: downloads.to_string_lossy().into_owned(),
            content_path: downloads.join("renamed.txt").to_string_lossy().into_owned(),
            category: "fixture".into(),
            tags: "read".into(),
            up_limit: NO_UPLOAD_LIMIT,
            ratio_limit: 0.0,
            ..Default::default()
        };
        let mut file = TorrentFile {
            name: "renamed.txt".into(),
            size: 5,
            progress: 1.0,
            priority: 1,
            ..Default::default()
        };
        let rejected = app
            .dispatcher
            .import_legacy(
                vec![torrent.clone()],
                HashMap::from([(hash.clone(), vec![file.clone()])]),
                backup.clone(),
            )
            .unwrap_err();
        assert!(rejected
            .to_string()
            .contains("renamed or unsupported layout"));
        assert!(!store.join("torrents.json").exists());
        assert!(!store.join(format!("{hash}.torrent")).exists());
        assert!(!downloads.join("original.txt").exists());
        assert_eq!(
            std::fs::read(downloads.join("renamed.txt")).unwrap(),
            b"hello"
        );
        std::fs::rename(
            downloads.join("renamed.txt"),
            downloads.join("original.txt"),
        )
        .unwrap();
        torrent.content_path = downloads
            .join("original.txt")
            .to_string_lossy()
            .into_owned();
        file.name = "original.txt".into();
        assert_eq!(
            app.dispatcher
                .import_legacy(
                    vec![torrent],
                    HashMap::from([(hash.clone(), vec![file])]),
                    backup
                )
                .unwrap(),
            1
        );
        assert!(app.dispatcher.legacy_migration_completed().unwrap());
        let record = app.dispatcher.runtime.block_on(async {
            let handle = app.dispatcher.inner.handle(&hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(handle.stats().finished);
            assert!(handle.is_paused());
            OwnedMutexGuard::lock(app.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone()
        });
        assert_eq!(record.upload_limit, NO_UPLOAD_LIMIT);
        assert_eq!(record.only_files, Some(vec![0]));
        assert_eq!(record.category, "fixture");
        assert_eq!(record.tags, "read");
        let error = app
            .dispatcher
            .dispatch(&request(
                "torrents/setUploadLimit",
                &[("hashes", &hash), ("limit", "1024")],
            ))
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("at least 16 KiB/s"));
        let error = app
            .dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hash), ("id", "0"), ("priority", "7")],
            ))
            .unwrap()
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("high and maximum are unavailable"));
        assert_eq!(
            std::fs::read(downloads.join("original.txt")).unwrap(),
            b"hello"
        );
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }
