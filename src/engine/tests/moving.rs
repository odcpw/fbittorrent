//! The payload read counter detects a recheck even if it finishes between polls.
use super::*;
#[test]
fn move_complete_preserves_verification_and_done_without_payload_reads() {
    let (app, directory, hash, counts) = fixture(false);
    let dispatcher = app.dispatcher();
    let original = row(&dispatcher);
    assert_eq!(original.state, TorrentState::Done);
    let before = counts.snapshot();
    let tracker_stats = dispatcher
        .inner
        .handle(&hash)
        .unwrap()
        .shared()
        .tracker_stats
        .clone();
    assert!(tracker_stats.announce_now());
    let target = directory.join("moved");
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: target.clone(),
        })
        .unwrap()
    else {
        panic!("move batch")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut states = Vec::new();
    while batch.result().is_none() {
        let current = row(&dispatcher);
        states.push(current.state);
        assert_ne!(current.state, TorrentState::Checking);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(batch.result().unwrap().is_ok());
    let after = row(&dispatcher);
    assert_eq!(after.state, TorrentState::Done);
    assert_eq!(after.save_path, target.to_string_lossy());
    assert_eq!(after.intent, original.intent);
    assert_eq!(after.seed, original.seed);
    assert!(after.checked);
    let retained = dispatcher
        .inner
        .handle(&hash)
        .unwrap()
        .shared()
        .tracker_stats
        .clone();
    assert!(Arc::ptr_eq(&tracker_stats, &retained));
    assert!(
        !retained.announce_now(),
        "move cannot reset the per-torrent forced announce limit"
    );
    assert_eq!(
        counts.snapshot().1,
        before.1,
        "move performs no checksum reads"
    );
    assert!(target.join("hello.txt").exists());
    assert!(!Path::new(&original.save_path).join("hello.txt").exists());
    assert!(directory.join(format!("resume/{hash}.bin")).exists());
    println!(
        "MOVE-VERIFIED original_reads={} final_reads={} states={states:?} save_path={}",
        before.1,
        counts.snapshot().1,
        after.save_path
    );
    drop(dispatcher);
    drop(app);
}
#[test]
fn move_preserves_explicit_pause_intent_and_rejects_missing_source_data() {
    let (app, directory, hash, counts) = fixture(false);
    let dispatcher = app.dispatcher();
    dispatcher
        .execute(EngineOp::Stop {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
        })
        .unwrap();
    let before = row(&dispatcher);
    let reads = counts.snapshot().1;
    let target = directory.join("paused-move");
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: target.clone(),
        })
        .unwrap()
    else {
        panic!("move")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(batch.result().unwrap().is_ok());
    let after = row(&dispatcher);
    assert_eq!(before.intent, after.intent);
    assert_eq!(after.state, TorrentState::Paused);
    assert!(after.checked);
    assert_eq!(counts.snapshot().1, reads);
    std::fs::remove_file(target.join("hello.txt")).unwrap();
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: directory.join("must-not-trust"),
        })
        .unwrap()
    else {
        panic!("move")
    };
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        batch.result().unwrap().unwrap_err().kind,
        crate::engine::moving::FailureKind::Source
    ));
    assert_eq!(row(&dispatcher).save_path, target.to_string_lossy());
    assert!(
        dispatcher.inner.handle(&hash).is_err(),
        "source failure discards the old verified handle before a retry"
    );
    assert_eq!(row(&dispatcher).state, TorrentState::Error);
    drop(dispatcher);
    drop(app);
}
#[test]
fn move_reads_remain_available_and_timed_out_drain_retains_join() {
    let (app, directory, hash, _) = fixture(false);
    let dispatcher = app.dispatcher();
    let serial = dispatcher
        .runtime
        .block_on(OwnedMutexGuard::lock(
            dispatcher.inner.operations.clone(),
            dispatcher.runtime.cx(),
        ))
        .unwrap();
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: directory.join("after-barrier"),
        })
        .unwrap()
    else {
        panic!("move")
    };
    assert_eq!(
        row(&dispatcher).state,
        TorrentState::Done,
        "reads do not take the mutation lock"
    );
    let refused = dispatcher
        .execute(EngineOp::Stop {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
        })
        .unwrap_err();
    let refusal = crate::api::ops::engine_error(refused);
    assert_eq!(refusal.code, crate::api::ErrorCode::Busy);
    assert_eq!(refusal.details.unwrap()["limit"], "dispatch");
    let timeout = dispatcher.runtime.block_on(time::timeout(
        dispatcher.runtime.cx(),
        Duration::from_millis(20),
        dispatcher.inner.moves.drain(),
    ));
    assert!(timeout.is_err());
    assert_eq!(
        dispatcher.inner.moves.retained_jobs(),
        1,
        "timed-out drain retains the owned join handle"
    );
    drop(serial);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(batch.result().unwrap().is_ok());
    dispatcher
        .runtime
        .block_on(dispatcher.inner.moves.drain())
        .unwrap();
    assert_eq!(dispatcher.inner.moves.retained_jobs(), 0);
    drop(dispatcher);
    drop(app);
}

#[test]
fn truncated_verified_payload_is_not_trusted_after_move() {
    let (app, directory, hash, _) = fixture(false);
    let dispatcher = app.dispatcher();
    let source = PathBuf::from(row(&dispatcher).save_path).join("hello.txt");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_len(1)
        .unwrap();
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: directory.join("reject-truncated"),
        })
        .unwrap()
    else {
        panic!("move")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        batch.result().unwrap().unwrap_err().kind,
        crate::engine::moving::FailureKind::Source
    ));
    assert_eq!(source.metadata().unwrap().len(), 1);
    assert!(dispatcher.inner.handle(&hash).is_err());
    assert_eq!(row(&dispatcher).state, TorrentState::Error);
    drop(dispatcher);
    drop(app);
}
#[test]
fn failed_ledger_save_rolls_back_payload_and_never_follows_temporary_symlink() {
    let (app, directory, hash, _) = fixture(false);
    let dispatcher = app.dispatcher();
    dispatcher
        .runtime
        .block_on(dispatcher.inner.maintain(dispatcher.runtime.cx()))
        .unwrap();
    let original = row(&dispatcher);
    let source = PathBuf::from(&original.save_path).join("hello.txt");
    let sentinel = directory.join("outside-ledger-sentinel");
    std::fs::write(&sentinel, b"do not overwrite").unwrap();
    let temporary = directory.join("torrents.json.tmp");
    std::os::unix::fs::symlink(&sentinel, &temporary).unwrap();
    let destination = directory.join("must-rollback");
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: destination.clone(),
        })
        .unwrap()
    else {
        panic!("move")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(batch.result().unwrap().is_err());
    assert_eq!(std::fs::read(source).unwrap(), b"hello");
    assert!(!destination.join("hello.txt").exists());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not overwrite");
    assert_eq!(row(&dispatcher).save_path, original.save_path);
    std::fs::remove_file(temporary).unwrap();
    drop(dispatcher);
    drop(app);
}
#[test]
fn queued_move_shutdown_does_not_wait_for_an_unrelated_mutation_lock() {
    let (app, directory, hash, _) = fixture(false);
    let dispatcher = app.dispatcher();
    let source = PathBuf::from(row(&dispatcher).save_path).join("hello.txt");
    let serial = dispatcher
        .runtime
        .block_on(OwnedMutexGuard::lock(
            dispatcher.inner.operations.clone(),
            dispatcher.runtime.cx(),
        ))
        .unwrap();
    let destination = directory.join("cancel-queued");
    let EngineResult::Move(batch) = dispatcher
        .execute(EngineOp::Move {
            hashes: vec![hash.clone()],
            save_path: destination.clone(),
        })
        .unwrap()
    else {
        panic!("move")
    };
    dispatcher.inner.stopping.store(true, Ordering::Release);
    dispatcher.inner.moves.cancel.cancel();
    let drain = dispatcher.runtime.block_on(time::timeout(
        dispatcher.runtime.cx(),
        Duration::from_millis(100),
        dispatcher.inner.moves.drain(),
    ));
    assert!(matches!(drain, Ok(Ok(()))));
    assert!(matches!(
        batch.result().unwrap().unwrap_err().kind,
        crate::engine::moving::FailureKind::Stopping
    ));
    assert_eq!(std::fs::read(source).unwrap(), b"hello");
    assert!(!destination.exists());
    drop(serial);
    drop(dispatcher);
    drop(app);
}
