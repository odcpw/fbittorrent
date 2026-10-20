use super::*;
#[test]
fn shutdown_cache_needs_no_fsync_and_keeps_fast_process_restart() {
    const CHILD: &str = "FBT_NO_SHUTDOWN_FSYNC_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let name = format!("{}::shutdown_cache_needs_no_fsync_and_keeps_fast_process_restart", module_path!().split_once("::").unwrap().1);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env(CHILD, "1").stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()).spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                panic!("cache shutdown child exceeded its ten-second fuse");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = child.wait_with_output().unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "child must execute the regression: {output:?}");
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    let (mut app, directory, hash, _counts) = fixture(false);
    let dispatcher = app.dispatcher();
    set_seed(&dispatcher, &hash, true).unwrap();
    // Inject fsync failure only after durable torrent admission/intent writes.
    // TSYNC includes the real engine blocking workers, not just this thread.
    let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
    let mut filter = [
        instruction(0x20, 0, 0, 0), // BPF_LD W ABS: seccomp_data.nr
        instruction(0x15, 1, 0, libc::SYS_fsync as u32),
        instruction(0x15, 0, 1, libc::SYS_fdatasync as u32),
        instruction(0x06, 0, 0, 0x00050000 | libc::EIO as u32),
        instruction(0x06, 0, 0, 0x7fff0000), // ALLOW
    ];
    let program = libc::sock_fprog { len: filter.len() as u16, filter: filter.as_mut_ptr() };
    unsafe {
        assert_eq!(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
        assert_eq!(libc::syscall(libc::SYS_seccomp, 1, 1, &program), 0);
    }
    app.quiesce();
    let began = std::time::Instant::now();
    assert!(app.drain(Duration::from_secs(2)));
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(directory.join("resume/CLEAN").is_file(), "boot cache must survive without fsync");
    drop(dispatcher);
    let store = resume::Store::open(&directory).unwrap();
    let records: Vec<Record> = read_ledger(&directory.join("torrents.json")).unwrap().records;
    assert!(store.prepare(&directory, &records).unwrap().take(&hash).is_some());
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

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
