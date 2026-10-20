use super::*;
use crate::engine::tests::{review_directory, review_record, test_app};

fn options(folder: &Path) -> ops::AddRequest {
    let mut bytes =
        b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    ops::AddRequest {
        sources: vec![],
        torrent: Some(("hello.torrent".into(), bytes)),
        save_path: Some(folder.to_string_lossy().into()),
        upload_limit: 0,
        seed: false,
        stopped: false,
        category: String::new(),
        tags: String::new(),
        origin: None,
    }
}

#[test]
fn staged_has_no_handle_or_payload_access_until_commit() {
    let directory = review_directory("staged-commit");
    let folder = directory.join("data");
    let app = test_app(&directory, &folder, Ledger::default(), None);
    let dispatcher = app.dispatcher();
    let EngineResult::Added(rows) = dispatcher
        .execute(EngineOp::Stage {
            options: options(&folder),
            files: Some(vec![0]),
        })
        .unwrap()
    else {
        panic!()
    };
    let hash = rows[0].hash.clone();
    std::thread::sleep(Duration::from_millis(30));
    assert!(dispatcher.inner.handle(&hash).is_err());
    assert!(
        !folder.join("hello.txt").exists(),
        "staging must not create/open the payload"
    );
    let EngineResult::Torrents(rows) = dispatcher.execute(EngineOp::Info).unwrap() else {
        panic!()
    };
    assert!(rows[0].staged && !rows[0].checked);
    assert_eq!(rows[0].state, view::TorrentState::Paused);
    assert_eq!(rows[0].size, Some(5));
    let before = std::fs::read(directory.join("torrents.json")).unwrap();
    let invalid = dispatcher
        .execute(EngineOp::Commit {
            hash: hash.clone(),
            files: Some(vec![1]),
            seed: None,
            save_path: None,
        })
        .unwrap_err();
    assert!(invalid.to_string().contains("out of range"));
    assert_eq!(
        before,
        std::fs::read(directory.join("torrents.json")).unwrap()
    );
    dispatcher
        .execute(EngineOp::Commit {
            hash: hash.clone(),
            files: Some(vec![0]),
            seed: Some(true),
            save_path: None,
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while dispatcher.inner.handle(&hash).is_err() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let saved = read_ledger(&directory.join("torrents.json")).unwrap();
    assert!(
        !saved.records[0].staged
            && !saved.records[0].paused
            && !saved.records[0].stop_on_complete()
    );
    for op in [
        EngineOp::Commit {
            hash: hash.clone(),
            files: None,
            seed: None,
            save_path: None,
        },
        EngineOp::Discard { hash: hash.clone() },
    ] {
        assert_eq!(
            dispatcher
                .execute(op)
                .unwrap_err()
                .downcast_ref::<InvalidTorrentState>()
                .unwrap()
                .reason,
            "not_staged"
        );
    }
    println!("staging: no live handle or payload file; invalid selection leaves durable bytes unchanged; commit alone admits storage, persists seed/run intent; non-staged operations refuse");
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn staged_discard_and_daemon_start_preserve_existing_bytes() {
    let directory = review_directory("staged-discard");
    let folder = directory.join("data");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("hello.txt"),
        b"never change these existing bytes",
    )
    .unwrap();
    let mut ledger = Ledger::default();
    let EngineResult::Added(rows) = execute(
        &directory,
        &folder,
        &mut ledger,
        &EngineOp::Stage {
            options: options(&folder),
            files: Some(vec![0]),
        },
    )
    .unwrap() else {
        panic!()
    };
    let hash = rows[0].hash.clone();
    let EngineResult::Added(rows) = execute(
        &directory,
        &folder,
        &mut ledger,
        &EngineOp::Stage {
            options: options(&folder),
            files: Some(vec![0]),
        },
    )
    .unwrap() else {
        panic!()
    };
    assert!(rows[0].duplicate);
    execute(
        &directory,
        &folder,
        &mut ledger,
        &EngineOp::Discard { hash: hash.clone() },
    )
    .unwrap();
    assert!(ledger.records.is_empty() && !directory.join(format!("{hash}.torrent")).exists());
    execute(
        &directory,
        &folder,
        &mut ledger,
        &EngineOp::Stage {
            options: options(&folder),
            files: Some(vec![0]),
        },
    )
    .unwrap();
    ledger.records.push(review_record(&"a".repeat(40), &folder));
    save_parked_ledger(&directory, &ledger).unwrap();
    discard_at_start(&directory).unwrap();
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(!directory.join(format!("{hash}.torrent")).exists());
    assert_eq!(
        std::fs::read(folder.join("hello.txt")).unwrap(),
        b"never change these existing bytes"
    );
    discard_at_start(&directory).unwrap();
    println!("discard/startup removed staged record+metainfo twice safely; unrelated record and original payload bytes survived");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn staged_timeout_stays_paused_and_outside_queue() {
    let directory = review_directory("staged-timeout");
    let mut record = review_record(&"b".repeat(40), &directory);
    record.staged = true;
    record.paused = true;
    let mut old = serde_json::to_value(&record).unwrap();
    old.as_object_mut().unwrap().remove("staged");
    assert!(!serde_json::from_value::<Record>(old).unwrap().staged);
    let restore = RestoreTask {
        generation: 1,
        outcome: RestoreOutcome::Failed {
            reason: "restore_failed",
            message: "torrent metadata lookup timed out".into(),
        },
        last_error: None,
        task: None,
        cancel: CancellationToken::new(),
    };
    let row = view::TorrentView::unrestored(&record, Some(&restore));
    assert_eq!(row.state, view::TorrentState::Paused);
    assert_eq!(row.reason.as_deref(), Some("metadata_timeout"));
    assert!(row.staged && !row.checked);
    assert!(!queue_policy::positioned(&record));
    assert_eq!(
        queue_policy::admit(&[record], &[Some(queue_policy::Stats::default())], 3),
        [false]
    );
    println!("90s resolver timeout diagnostic keeps staged paused intent; queue position and transfer admission absent; pre-upgrade records default staged=false");
}
