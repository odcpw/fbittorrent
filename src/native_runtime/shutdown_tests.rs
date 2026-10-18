//! Real native-worker/pool barriers, with expected failures captured by a parent.
use super::*;
use std::{process::Command, sync::mpsc};

const CHILD: &str = "FBT_P103C_PROBE";
const REGION_FAILURE: &str = "native application region did not drain before its deadline";
const RUNTIME_FAILURE: &str = "native runtime teardown exceeded its deadline";
const STORAGE_SITE: &str = "blocked_storage_read at src/native_runtime/shutdown_tests.rs";

fn probe(kind: &str) {
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
