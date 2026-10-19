use ops::{EngineOp, EngineResult, HashSelection};
use view::{TorrentState, TorrentView};

fn dormant_fixture(
    label: &str,
    completion: i64,
    seed: bool,
    missing: bool,
) -> (AppEngine, PathBuf, Vec<Record>) {
    let directory = review_directory(label);
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let mut records = Vec::new();
    for index in 0..5 {
        let name = format!("completed-{index}.txt");
        let mut bytes = format!(
            "d4:infod6:lengthi5e4:name{}:{}12:piece lengthi16384e6:pieces20:",
            name.len(),
            name
        )
        .into_bytes();
        // SHA-1(hello). These are real complete files, checked against these
        // same piece hashes when admitted by seed or recheck below.
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        let mut record = review_record(&hash, &downloads);
        record.name = name.clone();
        record.completion_on = completion;
        if seed {
            record.ratio_limit = -2.0;
        }
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        if !missing {
            std::fs::write(downloads.join(name), b"hello").unwrap();
        }
        records.push(record);
    }
    save_parked_ledger(
        &directory,
        &Ledger {
            generation: Default::default(),
            migrated_legacy: true,
            records: records.clone(),
        },
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
            disable_trackers: true,
            disable_local_service_discovery: true,
            concurrent_init_limit: Some(1),
            fastresume: false,
            ..Default::default()
        },
    )
    .unwrap();
    (app, directory, records)
}

fn dormant_rows(dispatcher: &EngineDispatcher) -> Vec<TorrentView> {
    let EngineResult::Torrents(rows) = dispatcher.execute(EngineOp::Info).unwrap() else {
        panic!("expected torrents")
    };
    rows
}

fn wait_for_check(dispatcher: &EngineDispatcher, hash: &str) {
    let handle = dispatcher.inner.handle(hash).unwrap();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        time::timeout(
            cx,
            Duration::from_secs(10),
            handle.wait_until_initialized(cx),
        )
        .await
        .unwrap()
        .unwrap();
        dispatcher.inner.maintain(cx).await.unwrap();
    });
}

fn dormant_diagnostic(action: &str, observed: serde_json::Value) {
    println!(
        "FBT-P116-STEP {}",
        serde_json::json!({"action": action, "observed": observed})
    );
}

#[test]
fn restore_dormant_record() {
    let started = std::time::Instant::now();
    let (app, directory, records) = dormant_fixture("lazy-no-checks", 123, false, false);
    let dispatcher = app.dispatcher();
    let rows = dormant_rows(&dispatcher);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(rows.len(), 5);
    for row in &rows {
        assert_eq!(row.state, TorrentState::Done);
        assert!(!row.checked);
        assert_eq!(row.progress, Some(1.0));
        assert_eq!(row.size, Some(5));
        assert_eq!(row.completed_bytes, Some(5));
        assert_eq!(row.completed_at, Some(123));
        assert!(row.peers.is_none() && row.dl_bps.is_none() && row.ul_bps.is_none());
        assert_eq!(
            dispatcher.restore_outcomes()[&row.hash],
            RestoreOutcome::Dormant
        );
        assert!(dispatcher.inner.handle(&row.hash).is_err());
        let EngineResult::Files(files) = dispatcher
            .execute(EngineOp::Files {
                hash: row.hash.clone(),
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 5);
        assert!(!files[0].progress_known);
    }
    dispatcher
        .runtime
        .block_on(dispatcher.inner.maintain(dispatcher.runtime.cx()))
        .unwrap();
    assert!(records
        .iter()
        .all(|r| dispatcher.inner.handle(&r.hash).is_err()));
    dormant_diagnostic(
        "list five complete records without any admitted handles",
        serde_json::json!({"elapsed_ms": started.elapsed().as_secs_f64()*1000.0,"rows":rows}),
    );
    // Pausing a dormant torrent must not schedule a check or lose its completion.
    dispatcher
        .execute(EngineOp::Stop {
            hashes: HashSelection::All,
        })
        .unwrap();
    assert!(dormant_rows(&dispatcher)
        .iter()
        .all(|r| r.state == TorrentState::Done && !r.checked));
    let error = dispatcher
        .execute(EngineOp::Start {
            hashes: HashSelection::Hashes(vec![records[0].hash.clone()]),
            skip_complete: false,
        })
        .unwrap_err();
    assert!(error.downcast_ref::<crate::agent::AppError>().is_some());
    dispatcher
        .execute(EngineOp::SetSeed {
            hashes: HashSelection::Hashes(vec![records[0].hash.clone()]),
            on: false,
        })
        .unwrap();
    assert!(dispatcher.inner.handle(&records[0].hash).is_err());
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_seed_checks() {
    let (app, directory, records) = dormant_fixture("lazy-seed", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = &records[0].hash;
    dispatcher
        .execute(EngineOp::SetSeed {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
            on: true,
        })
        .unwrap();
    wait_for_check(&dispatcher, hash);
    let rows = dormant_rows(&dispatcher);
    let row = rows.iter().find(|r| &r.hash == hash).unwrap();
    assert_eq!(row.state, TorrentState::Seeding);
    assert!(row.checked && row.seed);
    assert_eq!(row.completed_bytes, Some(5));
    assert_eq!(row.upload_limit_bps, Some(0));
    assert!(rows
        .iter()
        .filter(|r| &r.hash != hash)
        .all(|r| r.state == TorrentState::Done && !r.checked));
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 1);
    dormant_diagnostic(
        "seed one restores and verifies only that torrent",
        serde_json::json!(rows),
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_missing_files() {
    let (app, directory, records) = dormant_fixture("lazy-missing", 123, false, true);
    let dispatcher = app.dispatcher();
    assert!(dormant_rows(&dispatcher)
        .iter()
        .all(|r| r.state == TorrentState::Error && r.reason.as_deref()==Some("missing_files") && r.progress.is_none() && !r.checked));
    let hash = &records[0].hash;
    let error = dispatcher
        .execute(EngineOp::SetSeed {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
            on: true,
        })
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<InvalidTorrentState>().unwrap().reason,
        "missing_files"
    );
    let rows = dormant_rows(&dispatcher);
    let row = rows.iter().find(|r| &r.hash == hash).unwrap();
    assert_eq!(row.state, TorrentState::Error);
    assert_eq!(row.reason.as_deref(), Some("missing_files"));
    assert!(!row.checked);
    assert!(row.completed_at.is_none());
    assert!(dispatcher.inner.handle(hash).is_err());
    assert!(!Path::new(&records[0].output_folder)
        .join(&records[0].name)
        .exists());
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records[0]
            .completion_on,
        0
    );
    assert!(rows
        .iter()
        .filter(|r| &r.hash != hash)
        .all(|r| r.state == TorrentState::Error && r.reason.as_deref()==Some("missing_files") && r.progress.is_none()));
    dormant_diagnostic(
        "missing files become an error only on seed",
        serde_json::json!(rows),
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_open_no_check() {
    let (app, directory, records) = dormant_fixture("lazy-open", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = &records[0].hash;
    let began = std::time::Instant::now();
    let EngineResult::OpenTarget(path) = dispatcher
        .execute(EngineOp::OpenTarget { hash: hash.clone() })
        .unwrap()
    else {
        panic!()
    };
    let elapsed = began.elapsed();
    assert!(elapsed < Duration::from_millis(100));
    assert_eq!(
        path,
        Path::new(&records[0].output_folder).join(&records[0].name)
    );
    let row = dormant_rows(&dispatcher)
        .into_iter()
        .find(|r| &r.hash == hash)
        .unwrap();
    assert_eq!(row.state, TorrentState::Done);
    assert!(!row.checked && dispatcher.inner.handle(hash).is_err());
    dormant_diagnostic(
        "open target stats without hashing",
        serde_json::json!({"roundtrip_ms":elapsed.as_secs_f64()*1000.0,"row":row}),
    );
    std::fs::remove_file(path).unwrap();
    let error = dispatcher
        .execute(EngineOp::OpenTarget { hash: hash.clone() })
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<InvalidTorrentState>().unwrap().reason,
        "missing_files"
    );
    let row = dormant_rows(&dispatcher)
        .into_iter()
        .find(|r| &r.hash == hash)
        .unwrap();
    assert_eq!(row.state, TorrentState::Error);
    assert_eq!(row.reason.as_deref(), Some("missing_files"));
    assert!(!row.checked);
    dormant_diagnostic(
        "missing open target records structured failure",
        serde_json::json!(row),
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_recheck_clears_corrupt_completion() {
    let (app, directory, records) = dormant_fixture("lazy-corrupt", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = &records[0].hash;
    std::fs::write(
        Path::new(&records[0].output_folder).join(&records[0].name),
        b"wrong",
    )
    .unwrap();
    dispatcher
        .execute(EngineOp::Recheck { hash: hash.clone() })
        .unwrap();
    wait_for_check(&dispatcher, hash);
    let row = dormant_rows(&dispatcher)
        .into_iter()
        .find(|r| &r.hash == hash)
        .unwrap();
    assert!(row.checked);
    assert_eq!(row.completed_bytes, Some(0));
    assert!(row.completed_at.is_none());
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records[0]
            .completion_on,
        0
    );
    dormant_diagnostic(
        "recheck actually hashes corrupt data and clears completion",
        serde_json::json!(row),
    );
    drop(app);
    // Production restart must check this incomplete record again, not mark done.
    let cfg = Config {
        save_path: Some(directory.join("downloads").to_string_lossy().into()),
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
            disable_trackers: true,
            disable_local_service_discovery: true,
            ..Default::default()
        },
    )
    .unwrap();
    let dispatcher = app.dispatcher();
    assert_ne!(
        dispatcher.restore_outcomes().get(hash),
        Some(&RestoreOutcome::Dormant)
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_file_selection_validates_before_admission() {
    let (app, directory, records) = dormant_fixture("lazy-files", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = &records[0].hash;
    assert!(dispatcher
        .execute(EngineOp::SetFiles {
            hash: hash.clone(),
            include: vec![]
        })
        .is_err());
    assert!(dispatcher
        .execute(EngineOp::SetFiles {
            hash: hash.clone(),
            include: vec![1]
        })
        .is_err());
    assert!(dispatcher.inner.handle(hash).is_err());
    dispatcher
        .execute(EngineOp::SetFiles {
            hash: hash.clone(),
            include: vec![0],
        })
        .unwrap();
    wait_for_check(&dispatcher, hash);
    assert!(
        dormant_rows(&dispatcher)
            .into_iter()
            .find(|r| &r.hash == hash)
            .unwrap()
            .checked
    );
    assert_eq!(
        dispatcher.inner.handle(hash).unwrap().only_files(),
        Some(vec![0])
    );
    // The legacy file-priority route uses the same dormant admission policy.
    let hash = &records[1].hash;
    dispatcher
        .execute(EngineOp::FilePriority {
            hash: hash.clone(),
            ids: vec![0],
            priority: 1,
        })
        .unwrap();
    wait_for_check(&dispatcher, hash);
    assert!(
        dormant_rows(&dispatcher)
            .into_iter()
            .find(|r| &r.hash == hash)
            .unwrap()
            .checked
    );
    dormant_diagnostic(
        "nonempty file selections check dormant data",
        serde_json::json!(dormant_rows(&dispatcher)),
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unknown_completion_and_seeding_restore_normally() {
    for (completion, seed) in [(-1, false), (0, false), (123, true)] {
        let (app, directory, records) = dormant_fixture("lazy-exclusions", completion, seed, false);
        let dispatcher = app.dispatcher();
        assert!(records
            .iter()
            .all(|record| dispatcher.restore_outcomes().get(&record.hash)
                != Some(&RestoreOutcome::Dormant)));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while dispatcher.inner.handle(&records[0].hash).is_err() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        wait_for_check(&dispatcher, &records[0].hash);
        dormant_diagnostic(
            "unknown completion and saved seed intent still check",
            serde_json::json!({"completion":completion,"seed":seed,"rows":dormant_rows(&dispatcher)}),
        );
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn dormant_restart_preserves_selected_piece_totals() {
    let directory = review_directory("lazy-selected-pieces");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    // The first two selected files share a piece with first-skip.bin. The
    // selected last.bin also shares the short final piece with last-skip.bin.
    // Neither of middle-skip.bin's two complete pieces is selected.
    let files = [
        ("first-a.bin", 8192, b'a'),
        ("first-b.bin", 4096, b'a'),
        ("first-skip.bin", 4096, b'a'),
        ("middle-skip.bin", 32768, b'b'),
        ("last.bin", 8192, b'c'),
        ("last-skip.bin", 3, b'c'),
        ("empty.bin", 0, b'c'),
    ];
    let mut bytes = b"d4:infod5:filesl".to_vec();
    for (name, length, value) in files {
        bytes.extend_from_slice(
            format!("d6:lengthi{length}e4:pathl{}:{name}ee", name.len()).as_bytes(),
        );
        std::fs::write(downloads.join(name), vec![value; length]).unwrap();
    }
    bytes.extend_from_slice(b"e4:name8:selected12:piece lengthi16384e6:pieces80:");
    // Actual SHA-1 hashes of 16384 'a' bytes, twice 16384 'b' bytes,
    // and the shorter final piece of 8195 'c' bytes.
    bytes.extend_from_slice(&[
        0x12, 0xff, 0x34, 0x7b, 0x4f, 0x27, 0xd6, 0x9e, 0x1f, 0x32, 0x8e, 0x6f, 0x4b, 0x55, 0x73,
        0xe3, 0x66, 0x6e, 0x12, 0x2f,
    ]);
    bytes.extend_from_slice(
        &[
            0x76, 0x02, 0x42, 0x43, 0x18, 0x7a, 0x14, 0xd9, 0xcf, 0xa7, 0xed, 0x6f, 0x21, 0x2c,
            0x81, 0x51, 0xb7, 0x2f, 0x7b, 0x92,
        ]
        .repeat(2),
    );
    bytes.extend_from_slice(&[
        0x1c, 0x64, 0xf9, 0xc6, 0x59, 0x9a, 0x4f, 0x32, 0xc3, 0x50, 0x17, 0xde, 0xcf, 0x8a, 0x45,
        0xaa, 0xbf, 0xd7, 0xd2, 0x52,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
    let mut record = review_record(&hash, &downloads);
    record.name = "selected".into();
    record.only_files = Some(vec![0, 1, 4]);
    let app = test_app(
        &directory,
        &downloads,
        Ledger {
            generation: Default::default(),
            migrated_legacy: true,
            records: vec![record],
        },
        None,
    );
    let dispatcher = app.dispatcher();
    wait_for_check(&dispatcher, &hash);
    let stats = dispatcher.inner.handle(&hash).unwrap().stats();
    assert!(stats.finished);
    assert_eq!(stats.total_bytes, 16384 + 8195);
    assert_eq!(stats.progress_bytes, stats.total_bytes);
    let live = dormant_rows(&dispatcher).pop().unwrap();
    assert!(live.checked);
    assert_eq!(live.size, Some(stats.total_bytes));
    assert_eq!(live.completed_bytes, Some(stats.progress_bytes));
    let saved = read_ledger(&directory.join("torrents.json")).unwrap();
    assert!(saved.records[0].completion_on > 0);
    assert_eq!(saved.records[0].only_files, Some(vec![0, 1, 4]));
    drop(app);

    // A real production restart with the unselected middle data now missing.
    // It must keep the selected completion claim without admitting any storage.
    let skipped = downloads.join("middle-skip.bin");
    std::fs::remove_file(&skipped).unwrap();
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
            disable_trackers: true,
            disable_local_service_discovery: true,
            fastresume: false,
            ..Default::default()
        },
    )
    .unwrap();
    let dispatcher = app.dispatcher();
    let dormant = dormant_rows(&dispatcher).pop().unwrap();
    assert_eq!(dormant.state, TorrentState::Done);
    assert_eq!(dormant.progress, Some(1.0));
    assert!(!dormant.checked);
    assert_eq!(dormant.size, Some(stats.total_bytes));
    assert_eq!(dormant.completed_bytes, Some(stats.progress_bytes));
    assert_eq!(dormant.completed_at, live.completed_at);
    assert_eq!(
        dispatcher.restore_outcomes()[&hash],
        RestoreOutcome::Dormant
    );
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 0);
    assert!(!skipped.exists(), "restart must not recreate skipped data");
    let EngineResult::Files(saved_files) = dispatcher
        .execute(EngineOp::Files { hash: hash.clone() })
        .unwrap()
    else {
        panic!("expected saved files")
    };
    assert_eq!(saved_files.iter().map(|file| file.size).sum::<i64>(), 57347);
    assert!(saved_files.iter().all(|file| !file.progress_known));
    for file in &saved_files {
        assert_eq!(
            file.priority,
            if [0, 1, 4].contains(&file.index) {
                1
            } else {
                0
            }
        );
    }
    assert_eq!(saved_files[3].size, 32768);
    // The compatibility view uses the corrected counts with its existing wire
    // shape; all ten existing wire fixtures remain unchanged.
    let legacy = dispatcher.inner.unrestored_snapshot(&saved.records[0]);
    assert_eq!(legacy.size, stats.total_bytes as i64);
    assert_eq!(legacy.completed, stats.progress_bytes as i64);

    // Metadata-only edges: absent/empty selection, an overlapping selection,
    // just the short last piece, and an empty file (which selects no pieces).
    for (selection, expected) in [
        (None, 57347),
        (Some(vec![]), 0),
        (Some(vec![0, 1]), 16384),
        (Some(vec![4]), 8195),
        (Some(vec![6]), 0),
    ] {
        let mut record = saved.records[0].clone();
        record.only_files = selection;
        assert_eq!(
            dispatcher
                .inner
                .saved_torrent(&record)
                .unwrap()
                .selected_bytes,
            expected
        );
    }
    dormant_diagnostic(
        "restart retains the checked selected-piece totals and skips missing unselected data",
        serde_json::json!({"live": live, "dormant": dormant, "files": saved_files,
            "selected_file_bytes": 20480, "selected_piece_bytes": stats.total_bytes,
            "unselected_whole_piece_bytes": 32768, "session_handles": 0}),
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
