//! Record-only edits must not depend on an admitted torrent or hash any payload.
use super::*;

#[test]
fn dormant_caps_are_durable_without_storage_and_rollback_failed_saves() {
    let (app, directory, hash, counts) = fixture(true);
    let dispatcher = app.dispatcher();
    let original = row(&dispatcher);
    let cap = |download| {
        if download {
            EngineOp::SetDownloadLimit {
                hashes: HashSelection(vec![hash.clone()]),
                bps: Some(131072),
            }
        } else {
            EngineOp::SetUploadLimit {
                hashes: HashSelection(vec![hash.clone()]),
                limit: 65536,
            }
        }
    };
    for download in [true, false] {
        let saved = std::fs::read(directory.join("torrents.json")).unwrap();
        std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
        assert!(dispatcher.execute(cap(download)).is_err());
        std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            saved
        );
        let current = row(&dispatcher);
        assert_eq!(
            current.download_limit_bps,
            if download { None } else { Some(131072) }
        );
        assert_eq!(current.upload_limit_bps, original.upload_limit_bps);
        dispatcher.execute(cap(download)).unwrap();
    }
    for op in [
        EngineOp::SetDownloadLimit {
            hashes: HashSelection(vec![hash.clone()]),
            bps: Some(1),
        },
        EngineOp::SetUploadLimit {
            hashes: HashSelection(vec![hash.clone()]),
            limit: 1,
        },
        EngineOp::SetDownloadLimit {
            hashes: HashSelection(vec!["0".repeat(40)]),
            bps: Some(65536),
        },
    ] {
        assert!(dispatcher.execute(op).is_err());
    }
    let final_row = row(&dispatcher);
    assert_eq!(final_row.download_limit_bps, Some(131072));
    assert_eq!(final_row.upload_limit_bps, Some(65536));
    assert_eq!(final_row.completed_at, original.completed_at);
    assert_eq!(final_row.intent, original.intent);
    assert_eq!(final_row.state, TorrentState::Done);
    assert!(!final_row.checked && !final_row.upload_enabled);
    assert_eq!(counts.snapshot(), (0, 0, 0));
    assert!(dispatcher.inner.handle(&hash).is_err());
    let saved = read_ledger(&directory.join("torrents.json")).unwrap();
    assert_eq!(saved.records[0].download_limit, Some(131072));
    assert_eq!(saved.records[0].upload_limit, 65536);
    println!(
        "dormant caps: durable download=131072 upload=65536; no storage/read/permission change; failed saves rolled back"
    );
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_move_checks_once_then_preserves_completion_and_payload() {
    let (app, directory, hash, counts) = fixture(true);
    let dispatcher = app.dispatcher();
    let original = row(&dispatcher);
    let target = directory.join("dormant-moved");
    assert_eq!(counts.snapshot(), (0, 0, 0));
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: target.clone(),
        })
        .unwrap()
    else {
        panic!("move receipt")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(batch.result().unwrap().is_ok(), "{:?}", batch.result());
    let current = row(&dispatcher);
    assert_eq!(current.state, TorrentState::Done);
    assert!(current.checked && !current.upload_enabled);
    assert_eq!(current.completed_at, original.completed_at);
    assert_eq!(current.intent, original.intent);
    assert_eq!(current.save_path, target.to_string_lossy());
    assert_eq!(std::fs::read(target.join("hello.txt")).unwrap(), b"hello");
    assert!(!Path::new(&original.save_path).join("hello.txt").exists());
    assert_eq!(
        counts.snapshot().1,
        1,
        "one admission check, no second check after relocation"
    );
    assert_eq!(counts.snapshot().2, 5);
    let saved = read_ledger(&directory.join("torrents.json")).unwrap();
    assert_eq!(saved.records[0].output_folder, target.to_string_lossy());
    assert_eq!(saved.records[0].completion_on, 123);
    println!(
        "dormant move: real check reads=1 bytes=5; exact payload relocated, completion=123 and upload off preserved"
    );
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dormant_move_missing_source_never_creates_or_claims_completion() {
    let (app, directory, hash, counts) = fixture(true);
    let dispatcher = app.dispatcher();
    std::fs::remove_file(directory.join("downloads/hello.txt")).unwrap();
    let target = directory.join("missing-move");
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: target.clone(),
        })
        .unwrap()
    else {
        panic!("move receipt")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        batch.result().unwrap().unwrap_err().kind,
        crate::engine::moving::FailureKind::Source
    ));
    let current = row(&dispatcher);
    assert_eq!(current.state, TorrentState::Error);
    assert!(current.completed_at.is_none() && !current.checked);
    assert!(!directory.join("downloads/hello.txt").exists() && !target.join("hello.txt").exists());
    assert_eq!(counts.snapshot(), (0, 0, 0));
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
