#[test]
fn queue_maintenance_shutdown_is_quiet() {
    const CHILD: &str = "FBT_MAINTENANCE_SHUTDOWN_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "engine::tests::queue_maintenance_shutdown_is_quiet", "--nocapture"])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("maintenance shutdown hung: {}", String::from_utf8_lossy(&output.stderr));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "shutdown child failed: {stdout}\n{stderr}");
        assert!(!stderr.lines().any(|line| line.starts_with("ERROR ")),
            "ordinary shutdown emitted an error: {stderr}");
        assert!(stdout.contains("maintenance_keep_running=false"), "{stdout}");
        assert!(stderr.contains("INFO engine drained"), "{stderr}");
        println!("queue maintenance blocked on the ledger, shutdown retired it, and the engine drained without ERROR");
        return;
    }

    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let directory = review_directory("maintenance-shutdown");
    let downloads = directory.join("downloads");
    let cfg = Config {save_path:Some(downloads.to_string_lossy().into()), ..Default::default()};
    let instance = InstanceLock::acquire_in(directory.join("runtime")).unwrap();
    let mut app = start_in(&cfg, Lifecycle::default(), directory.clone(), instance, SessionOptions {
        dht:None, disable_trackers:true, disable_local_service_discovery:true, ..Default::default()
    }).unwrap();
    let dispatcher = app.dispatcher();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx).await.unwrap();
        let mut tick = std::pin::pin!(dispatcher.inner.maintenance_tick(cx));
        // Prove the real tick passed its admission and is waiting for the
        // ledger before quiescing. No sleeps or probabilistic racing are used.
        poll_fn(|task| {
            assert!(tick.as_mut().poll(task).is_pending());
            Poll::Ready(())
        }).await;
        app.quiesce();
        drop(ledger);
        println!("maintenance_keep_running={}", tick.await);
    });
    assert!(app.drain(Duration::from_secs(2)));
    drop(app);
    drop(InstanceLock::acquire_in(directory.join("runtime")).unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn quiesce_releases_observers_while_the_ledger_is_still_held() {
    use std::{future::{poll_fn, Future}, task::Poll};
    let directory = review_directory("observer-shutdown");
    let mut app = test_app(&directory, &directory.join("downloads"), Ledger::default(), None);
    let dispatcher = app.dispatcher();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx).await.unwrap();
        let mut read = std::pin::pin!(dispatcher.observation_ledger());
        poll_fn(|task| {
            assert!(read.as_mut().poll(task).is_pending(), "observer must be waiting for the ledger");
            Poll::Ready(())
        }).await;
        app.quiesce();
        // Retain the ledger until AFTER the cancellation receipt. The old
        // unconditional lock would remain pending forever at this boundary.
        poll_fn(|task| match read.as_mut().poll(task) {
            Poll::Ready(Err(error)) => {
                assert!(error.to_string().contains("native engine is closing"));
                Poll::Ready(())
            }
            _ => panic!("quiesced observer must resolve before the ledger is released"),
        }).await;
        drop(ledger);
    });
    assert!(app.drain(Duration::from_secs(2)));
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
