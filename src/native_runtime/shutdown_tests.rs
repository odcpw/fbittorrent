//! Real native-worker/pool barriers, with expected failures captured by a parent.
use super::*;
use std::{process::Command, sync::mpsc};

const CHILD: &str = "FBT_P103C_PROBE";
const REGION_FAILURE: &str = "native application region did not drain before its deadline";
const RUNTIME_FAILURE: &str = "native runtime teardown exceeded its deadline";
const STORAGE_SITE: &str = "blocked_storage_read at src/native_runtime/shutdown_tests.rs";

fn probe(kind: &str) -> String {
    let runtime = NativeRuntime::new().unwrap();
    let raw = runtime
        .handle
        .shared
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut held_region = None;
    let mut task = None;
    match kind {
        "worker" => {
            // Put the stand-in in a grandchild, so the snapshot must traverse
            // child regions rather than only looking at the application root.
            let region = runtime
                .block_on(runtime.cx().open_child_region(ChildRegionSpec::inherit()))
                .unwrap();
            task = Some(
                region
                    .cx()
                    .spawn(move |cx| async move {
                        cx.checkpoint_with(STORAGE_SITE).unwrap();
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        finished_tx.send(()).unwrap();
                    })
                    .unwrap(),
            );
            held_region = Some(region);
        }
        "pool" => {
            // A physical blocking-pool job can survive its async wrapper.
            // There is no task name in the pool API: assert the pool's name
            // and live busy count instead of inventing per-closure attribution.
            raw.blocking_handle().unwrap().spawn(move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                finished_tx.send(()).unwrap();
            });
        }
        "owner" => {}
        _ => panic!("unknown shutdown probe"),
    }
    if kind != "owner" {
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    // Only the ownership control keeps a strong Runtime across shutdown.
    let retained = (kind == "owner").then(|| raw.clone());
    drop(raw);
    let budget = Duration::from_millis(50);
    let began = Instant::now();
    let result = runtime.shutdown_timeout(budget);
    let elapsed = began.elapsed();
    let still_blocked =
        kind == "owner" || matches!(finished_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    // Always release our work before an assertion can unwind the fixture.
    if kind != "owner" {
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    drop(retained);
    drop(held_region);
    if let Some(task) = task {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !task.is_finished() {
            assert!(Instant::now() < deadline, "released worker did not finish");
            std::thread::yield_now();
        }
    }
    let error = format!("{:#}", result.unwrap_err());
    println!(
        "probe={kind} still_blocked={still_blocked} elapsed_us={} returned={error}",
        elapsed.as_micros()
    );
    assert!(
        still_blocked,
        "the work must remain held until the fuse returns"
    );
    assert!(
        elapsed < Duration::from_millis(250),
        "50 ms fuse exceeded: {elapsed:?}"
    );
    assert!(error.contains(RUNTIME_FAILURE), "{error}");
    if kind == "worker" {
        assert!(
            error.contains(REGION_FAILURE),
            "region failure was overwritten: {error}"
        );
        assert!(
            error.contains(STORAGE_SITE),
            "missing blocked task attribution: {error}"
        );
        assert!(error.contains("age_ms"), "missing task age: {error}");
    } else {
        assert!(
            !error.contains(REGION_FAILURE),
            "invented region failure: {error}"
        );
    }
    error
}

fn assert_probe(kind: &str, name: &str) {
    if std::env::var(CHILD).is_ok_and(|value| value == kind) {
        probe(kind);
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, kind)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "shutdown probe hung: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    use base64::Engine;
    // Transport every child diagnostic to the local JSONL runner, even on red.
    println!(
        "FBT-P103C-PROBE-LOG {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{stdout}\n{stderr}"))
    );
    assert!(
        output.status.success(),
        "shutdown probe failed: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("probe={kind} still_blocked=true")),
        "{stdout}"
    );
    let lines: Vec<_> = stderr.lines().collect();
    let messages: &[&str] = if kind == "worker" {
        &[REGION_FAILURE, RUNTIME_FAILURE]
    } else {
        &[RUNTIME_FAILURE]
    };
    assert_eq!(
        lines.len(),
        messages.len(),
        "unexpected diagnostic: {stderr}"
    );
    for (line, message) in lines.iter().zip(messages) {
        let suffix = line
            .strip_prefix(&format!("ERROR engine {message} elapsed_ms="))
            .unwrap_or_else(|| panic!("wrong phase/format: {line}"));
        let (elapsed, snapshot) = suffix.split_once(" outstanding=").unwrap();
        assert!(elapsed.parse::<u128>().unwrap() < 250, "{line}");
        let snapshot: serde_json::Value = serde_json::from_str(snapshot).unwrap();
        assert!(snapshot["tasks"].is_array(), "{line}");
        assert!(snapshot["regions"].is_array(), "{line}");
        if kind == "worker" {
            let tasks = snapshot["tasks"].as_array().unwrap();
            assert!(
                tasks.iter().any(|task| task["checkpoint"] == STORAGE_SITE
                    && task["age_ms"].is_number()
                    && task["id"].is_string()
                    && task["region"].is_string()),
                "{line}"
            );
            assert!(
                snapshot["regions"].as_array().unwrap().len() >= 3,
                "grandchild not traversed: {line}"
            );
        } else if kind == "pool" {
            assert_eq!(
                snapshot["blocking_pool"]["name"], "native_blocking_pool",
                "{line}"
            );
            assert_eq!(snapshot["blocking_pool"]["busy_threads"], 1, "{line}");
        } else {
            assert!(snapshot["tasks"].as_array().unwrap().is_empty(), "{line}");
            assert_eq!(snapshot["blocking_pool"]["busy_threads"], 0, "{line}");
            assert_eq!(
                snapshot["runtime_ownership"], "not exposed by runtime API",
                "{line}"
            );
        }
    }
}

#[test]
fn blocked_storage_task_names_outstanding_work_and_preserves_both_failures() {
    assert_probe("worker", "native_runtime::shutdown_tests::blocked_storage_task_names_outstanding_work_and_preserves_both_failures");
}

#[test]
fn blocked_pool_is_attributed_after_application_region_closes() {
    assert_probe("pool", "native_runtime::shutdown_tests::blocked_pool_is_attributed_after_application_region_closes");
}

#[test]
fn retained_runtime_owner_does_not_invent_region_failure() {
    assert_probe(
        "owner",
        "native_runtime::shutdown_tests::retained_runtime_owner_does_not_invent_region_failure",
    );
}

#[test]
fn saturated_stderr_does_not_extend_shutdown_or_erase_phase_errors() {
    const NAME: &str = "native_runtime::shutdown_tests::saturated_stderr_does_not_extend_shutdown_or_erase_phase_errors";
    if std::env::var(CHILD).is_ok_and(|value| value == "saturated") {
        // Reuse every original worker assertion, including the 250 ms bound,
        // both returned errors and the checkpoint. Only the output sink differs.
        let flags = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_GETFL) };
        assert_eq!(flags & libc::O_NONBLOCK, 0);
        probe("worker");
        assert_eq!(
            unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_GETFL) },
            flags
        );
        return;
    }
    full_stderr_child(NAME, CHILD, "saturated", "probe=worker still_blocked=true");
    full_stderr_child(
        "engine::tests::join_timeout_preserves_unfinished_restore",
        "FBT_P103B_TIMEOUT_CHILD",
        "1",
        "drained=false present=true unfinished=true old_completion_assertion=false",
    );
}

fn full_stderr_child(name: &str, environment: &str, value: &str, expected: &str) {
    use std::{
        io::{Read, Write},
        os::fd::{AsRawFd, FromRawFd},
    };
    let mut descriptors = [-1; 2];
    // SAFETY: pipe2 initializes two owned descriptors on success.
    assert_eq!(
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    let mut reader = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
    let mut writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
    let fd = writer.as_raw_fd();
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) },
        0
    );
    let mut filled = 0;
    loop {
        match writer.write(&[b'x'; 4096]) {
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            other => panic!("filling pipe: {other:?}"),
        }
    }
    assert!(filled >= 4096);
    // The CHILD's stderr is blocking, exactly the old eprintln failure mode.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, 0) }, 0);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(environment, value)
        .stdout(std::process::Stdio::piped())
        .stderr(writer)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!(
                "shutdown waited on saturated stderr: {:?}",
                child.wait_with_output().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let mut sink = Vec::new();
    reader.read_to_end(&mut sink).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains(expected), "{stdout}");
    if environment == CHILD {
        assert!(
            stdout.contains(REGION_FAILURE) && stdout.contains(RUNTIME_FAILURE),
            "{stdout}"
        );
        assert!(stdout.contains(STORAGE_SITE), "{stdout}");
    }
    assert_eq!(
        sink.len(),
        filled,
        "a full pipe cannot accept even the fallback line"
    );
    assert!(sink.iter().all(|&byte| byte == b'x'));
    use base64::Engine;
    println!(
        "FBT-P103C-PROBE-LOG {}",
        base64::engine::general_purpose::STANDARD.encode(stdout)
    );
    println!("saturated_stderr_bytes={filled}; both phase failures retained without output");
}

#[test]
fn large_snapshot_with_held_task_lock_is_partial_and_bounded() {
    use asupersync::{
        observability::task_inspector::{SnapshotLimits, TaskInspector},
        runtime::state::RuntimeState,
        time::{TimeSource, TimerDriver, TimerDriverHandle},
    };
    #[derive(Debug)]
    struct Clock {
        armed: std::sync::atomic::AtomicBool,
    }
    impl TimeSource for Clock {
        fn now(&self) -> asupersync::types::Time {
            assert!(
                !self.armed.load(std::sync::atomic::Ordering::SeqCst),
                "bounded collection must not call an arbitrary runtime clock under locks"
            );
            asupersync::types::Time::ZERO
        }
    }
    let mut state = RuntimeState::new();
    let region = state.create_root_region(Budget::INFINITE);
    let mut handles = Vec::new();
    let mut held = None;
    for _ in 0..256 {
        let (id, handle) = state
            .create_task(region, Budget::INFINITE, std::future::pending::<()>())
            .unwrap();
        let task = state.task(id).unwrap();
        if held.is_none() {
            held = task.cx_inner.clone();
        }
        task.cx
            .as_ref()
            .unwrap()
            .checkpoint_with("large task checkpoint".repeat(200))
            .unwrap();
        handles.push(handle);
    }
    let clock = Arc::new(Clock {
        armed: std::sync::atomic::AtomicBool::new(false),
    });
    state.set_timer_driver(TimerDriverHandle::new(Arc::new(TimerDriver::with_clock(
        clock.clone(),
    ))));
    clock.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let inspector = TaskInspector::new(Arc::new(state), None);
    let lock = held.unwrap();
    // The real TaskRecord's CxInner write lock, not a diagnostic helper lock.
    let guard = lock.write();
    let limits = SnapshotLimits {
        max_tasks: 8,
        max_regions: 2,
        max_bytes: 2048,
        time_cap: Duration::from_millis(5),
        observed_at: None,
    };
    let began = Instant::now();
    let bounded = inspector.bounded_snapshot(limits, &[region]);
    let elapsed = began.elapsed();
    drop(guard);
    assert!(
        elapsed < limits.time_cap,
        "collection waited on task lock: {elapsed:?}"
    );
    assert!(bounded.partial);
    assert!(bounded.json.len() <= limits.max_bytes);
    let snapshot: serde_json::Value = serde_json::from_str(&bounded.json).unwrap();
    assert_eq!(snapshot["partial"], true);
    assert_eq!(snapshot["truncated"], true);
    assert!(snapshot["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason == "checkpoint_contention"));
    assert!(snapshot["tasks"].as_array().unwrap().len() <= limits.max_tasks);
    assert!(snapshot["regions"].as_array().unwrap().len() <= limits.max_regions);
    println!(
        "bounded_snapshot tasks={} regions={} bytes={} elapsed_us={} reasons={}",
        snapshot["tasks"].as_array().unwrap().len(),
        snapshot["regions"].as_array().unwrap().len(),
        bounded.json.len(),
        elapsed.as_micros(),
        snapshot["reasons"]
    );
    let tiny = inspector.bounded_snapshot(
        SnapshotLimits {
            max_bytes: 0,
            ..limits
        },
        &[region],
    );
    assert!(tiny.partial && tiny.json.is_empty());
    let expired = inspector.bounded_snapshot(
        SnapshotLimits {
            time_cap: Duration::ZERO,
            ..limits
        },
        &[region],
    );
    assert!(expired.partial && expired.json.contains("time_limit"));
    let counts = inspector.bounded_snapshot(
        SnapshotLimits {
            max_tasks: 2,
            max_regions: 0,
            max_bytes: 4096,
            ..limits
        },
        &[region],
    );
    let counts: serde_json::Value = serde_json::from_str(&counts.json).unwrap();
    assert_eq!(counts["tasks"].as_array().unwrap().len(), 2);
    assert!(counts["regions"].as_array().unwrap().is_empty());
    assert!(counts["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason == "task_limit"));
    assert!(counts["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason == "region_limit"));
    clock
        .armed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    drop(handles);
}

#[test]
fn socket_stderr_keeps_attribution_and_shutdown_budget() {
    use std::{
        io::Read,
        os::{fd::OwnedFd, unix::net::UnixStream},
    };
    if std::env::var_os("FBT_P103C_PRESSURED_REPLAY").is_some() {
        pressured_socket_child();
        return;
    }
    let (mut reader, writer) = UnixStream::pair().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_runtime::shutdown_tests::blocked_storage_task_names_outstanding_work_and_preserves_both_failures", "--nocapture"])
        .env(CHILD, "worker").stdout(std::process::Stdio::piped())
        .stderr(OwnedFd::from(writer)).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!(
                "socket diagnostic hung: {:?}",
                child.wait_with_output().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut stderr = String::new();
    reader.read_to_string(&mut stderr).unwrap();
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "{stderr}");
    for (line, failure) in lines.iter().zip([REGION_FAILURE, RUNTIME_FAILURE]) {
        assert!(
            line.starts_with(&format!("ERROR engine {failure}")),
            "{line}"
        );
        assert!(
            line.contains(STORAGE_SITE) && line.contains("age_ms"),
            "{line}"
        );
        let snapshot: serde_json::Value =
            serde_json::from_str(line.split_once(" outstanding=").unwrap().1).unwrap();
        assert!(snapshot["regions"].as_array().unwrap().len() >= 3, "{line}");
    }
    use base64::Engine;
    println!(
        "FBT-P103C-PROBE-LOG {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{stdout}\n{stderr}"))
    );
    pressured_socket_replay();
    short_socket_write_resumes_from_offset();
}

fn pressured_socket_child() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // First obtain the actual, asserted two-phase error without filling the
    // replay socket. This redirection is confined to this test subprocess.
    let saved = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    assert!(saved >= 0);
    let saved = unsafe { OwnedFd::from_raw_fd(saved) };
    let null = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .unwrap();
    assert_eq!(
        unsafe { libc::dup2(null.as_raw_fd(), libc::STDERR_FILENO) },
        libc::STDERR_FILENO
    );
    let began = Instant::now();
    let error = probe("worker");
    // A FIFO output receipt proves all previous /dev/null records are done
    // before restoring stderr; no sleep-based redirection race.
    assert!(shutdown_diagnostics::emit(String::new(), String::new()));
    assert_eq!(
        unsafe { libc::dup2(saved.as_raw_fd(), libc::STDERR_FILENO) },
        libc::STDERR_FILENO
    );
    let detail = replay_record(&error);
    let fallback = "ERROR engine closing native runtime:\n".to_string();
    assert!(detail.len() > 2048 && detail.len() < libc::PIPE_BUF as usize);
    assert!(shutdown_diagnostics::emit(detail.clone(), fallback));
    assert!(
        began.elapsed() < Duration::from_millis(250),
        "replay exceeded original ceiling"
    );
    assert!(error.contains(REGION_FAILURE) && error.contains(RUNTIME_FAILURE));
    println!(
        "pressured_replay_bytes={} elapsed_us={}",
        detail.len(),
        began.elapsed().as_micros()
    );
}

fn small_send_buffer(writer: &std::os::unix::net::UnixStream) {
    use std::os::fd::AsRawFd;
    let size: libc::c_int = 1024;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                writer.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&size as *const libc::c_int).cast(),
                std::mem::size_of_val(&size) as libc::socklen_t,
            )
        },
        0
    );
}

fn replay_record(error: &str) -> String {
    let mut line = format!("ERROR engine closing native runtime: error={error:?}");
    // Match the reviewer's 2562-byte record despite variable IDs/ages. Padding
    // after the quoted error preserves the exact two returned phase errors.
    while line.len() < 2561 {
        line.push(' ');
    }
    line.push('\n');
    line
}

fn pressured_socket_replay() {
    use std::{
        io::{Read, Write},
        os::{fd::OwnedFd, unix::net::UnixStream},
    };
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    small_send_buffer(&writer);
    writer.write_all(b"x").unwrap(); // Reviewer setup: one queued byte, reader paused.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_runtime::shutdown_tests::socket_stderr_keeps_attribution_and_shutdown_budget",
            "--nocapture",
        ])
        .env("FBT_P103C_PRESSURED_REPLAY", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(OwnedFd::from(writer))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!(
                "pressured socket hung: {:?}",
                child.wait_with_output().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert!(output.status.success(), "{stdout} sink={bytes:?}");
    assert_eq!(bytes.first(), Some(&b'x'));
    let received = std::str::from_utf8(&bytes[1..]).unwrap();
    let error = stdout
        .lines()
        .find(|line| line.starts_with("probe=worker still_blocked=true"))
        .unwrap()
        .split_once(" returned=")
        .unwrap()
        .1;
    let expected = replay_record(error);
    assert!(expected.len() > 2048);
    assert!(
        received == expected || received == "ERROR engine closing native runtime:\n",
        "torn/unframed replay: {received:?}"
    );
    assert!(received.ends_with('\n'));
    assert_eq!(received.lines().count(), 1);
    println!(
        "pressured_socket sndbuf=1024 prequeued=1 replay_bytes={} received_bytes={} fallback={}",
        expected.len(),
        received.len(),
        received != expected
    );
    use base64::Engine;
    println!(
        "FBT-P103C-PROBE-LOG {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{stdout}\n{received}"))
    );
}

fn short_socket_write_resumes_from_offset() {
    use std::{
        io::{Read, Write},
        os::{fd::AsRawFd, unix::net::UnixStream},
    };
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    small_send_buffer(&writer);
    writer.write_all(b"x").unwrap();
    let line = format!("ERROR engine partial-write control {}\n", "z".repeat(2530));
    let sent = unsafe {
        libc::send(
            writer.as_raw_fd(),
            line.as_ptr().cast(),
            line.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    assert!(
        sent > 0 && (sent as usize) < line.len(),
        "control must produce a real short send: {sent}"
    );
    let blocked_at = Instant::now();
    let blocked = shutdown_diagnostics::send_socket_record(
        writer.as_raw_fd(),
        line.as_bytes(),
        sent as usize,
        blocked_at + Duration::from_millis(5),
    )
    .unwrap();
    assert_eq!(
        blocked, sent as usize,
        "a full socket must neither lose nor replay the prefix"
    );
    assert!(
        blocked_at.elapsed() < Duration::from_millis(250),
        "socket retry fuse overran"
    );
    let mut received = vec![0; sent as usize + 1];
    reader.read_exact(&mut received).unwrap(); // Release capacity before resuming.
    let began = Instant::now();
    let written = shutdown_diagnostics::send_socket_record(
        writer.as_raw_fd(),
        line.as_bytes(),
        sent as usize,
        began + Duration::from_millis(5),
    )
    .unwrap();
    assert_eq!(written, line.len());
    assert!(began.elapsed() < Duration::from_millis(5));
    drop(writer);
    reader.read_to_end(&mut received).unwrap();
    assert_eq!(&received[1..], line.as_bytes());
    println!("short_socket first_bytes={sent} total_bytes={written} blocked_retry_ms={} completed_with_offset=true", blocked_at.elapsed().as_millis());
}
