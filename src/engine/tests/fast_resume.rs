use super::*;
#[test]
fn clean_restart_trusts_real_storage_without_checksum_reads_and_recheck_invalidates_first() {
    let (app, directory, hash, counts) = fixture(false);
    let dispatcher = app.dispatcher();
    set_seed(&dispatcher, &hash, true).unwrap();
    let before = counts.snapshot();
    assert_eq!(before, (1, 1, 5));
    drop(dispatcher);
    drop(app);
    assert!(directory.join("resume/CLEAN").exists());
    assert!(directory.join(format!("resume/{hash}.bin")).exists());
    let cfg = Config {
        save_path: Some(directory.join("downloads").to_string_lossy().into()),
        ..Default::default()
    };
    let lock = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
    let app = start_in(
        &cfg,
        Lifecycle::default(),
        directory.clone(),
        lock,
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            default_storage_factory: Some(CountedFilesystemFactory(counts.clone()).boxed()),
            ..Default::default()
        },
    )
    .unwrap();
    let dispatcher = app.dispatcher();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let row = row(&dispatcher);
        assert_ne!(
            row.state,
            TorrentState::Checking,
            "trusted restore must never advertise hashing"
        );
        assert!(
            row.checked,
            "validated have snapshot already checked the data"
        );
        if dispatcher.inner.handle(&hash).is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    wait_for_check(&dispatcher, &hash);
    let after = counts.snapshot();
    assert_eq!(
        after.1, before.1,
        "trusted admission performs no payload reads"
    );
    assert_eq!(after.0, before.0 + 1);
    assert!(!directory.join("resume/CLEAN").exists());
    let EngineResult::Recheck(batch) = dispatcher
        .execute(EngineOp::RecheckBatch {
            hashes: vec![hash.clone()],
        })
        .unwrap()
    else {
        panic!("batch")
    };
    assert!(
        !directory.join(format!("resume/{hash}.bin")).exists(),
        "resume cache is deleted before pending acknowledgement"
    );
    while batch.result().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        counts.snapshot().1,
        after.1 + 1,
        "explicit recheck reads the real piece exactly once"
    );
    println!("physical oracle: first scan 1 read/5 bytes; trusted restart adds 0 reads; explicit recheck removes cache before ack and adds exactly 1 read/5 bytes");
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn periodic_snapshot_is_due_at_five_minutes_and_has_no_clean_authority() {
    let (app, directory, hash, _counts) = fixture(false);
    let dispatcher = app.dispatcher();
    dispatcher
        .runtime
        .block_on(dispatcher.inner.periodic_resume(dispatcher.runtime.cx()))
        .unwrap();
    assert!(!directory.join(format!("resume/{hash}.bin")).exists());
    dispatcher
        .inner
        .resume
        .periodic_elapsed(Duration::from_secs(301));
    dispatcher
        .runtime
        .block_on(dispatcher.inner.periodic_resume(dispatcher.runtime.cx()))
        .unwrap();
    assert!(directory.join(format!("resume/{hash}.bin")).exists());
    assert!(!directory.join("resume/CLEAN").exists());
    println!("real periodic snapshot path: before 300s no write; at 301s writes have snapshot without CLEAN; production timer remains 300s");
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn changed_files_or_missing_marker_each_force_a_real_checksum_read() {
    for changed in [true, false] {
        let (app, directory, hash, counts) = fixture(false);
        let dispatcher = app.dispatcher();
        set_seed(&dispatcher, &hash, true).unwrap();
        drop(dispatcher);
        drop(app);
        let before = counts.snapshot();
        if changed {
            std::fs::write(directory.join("downloads/hello.txt"), b"HELLO").unwrap();
        } else {
            std::fs::remove_file(directory.join("resume/CLEAN")).unwrap();
        }
        let cfg = Config {
            save_path: Some(directory.join("downloads").to_string_lossy().into()),
            ..Default::default()
        };
        let lock = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
        let app = start_in(
            &cfg,
            Lifecycle::default(),
            directory.clone(),
            lock,
            SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                default_storage_factory: Some(CountedFilesystemFactory(counts.clone()).boxed()),
                ..Default::default()
            },
        )
        .unwrap();
        let dispatcher = app.dispatcher();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while dispatcher.inner.handle(&hash).is_err() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        wait_for_check(&dispatcher, &hash);
        assert_eq!(
            counts.snapshot().1,
            before.1 + 1,
            "untrusted restore must physically read its piece"
        );
        println!("changed={changed}: cold restore adds exactly one real checksum read (5 bytes)");
        drop(dispatcher);
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
