//! App-owned Asupersync runtime. Cloned dispatch handles borrow this owner's
//! runtime; shutdown closes admissions and drains the application child region.
use anyhow::{anyhow, Result};
use asupersync::{
    cx::{ChildRegion, ChildRegionSpec},
    runtime::{Runtime, RuntimeBuilder},
    types::{Budget, CancelReason},
    Cx,
};
use std::{
    future::Future,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

/// Teardown logging must not wait on stderr's process lock or output device.
pub(crate) fn log_teardown(line: String) {
    let fallback = format!("{}\n", line.split(" error=").next().unwrap_or(&line));
    shutdown_diagnostics::emit(format!("{line}\n"), fallback);
}

#[derive(Clone)]
pub struct Handle {
    shared: Arc<RwLock<Option<Runtime>>>,
    cx: Cx,
}
impl Handle {
    pub fn cx(&self) -> &Cx {
        &self.cx
    }
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.try_block_on(future).expect("native runtime is closed")
    }
    /// A bounded external reader can race engine-slot teardown between polls.
    /// Report a closed owner instead of panicking after its final read guard.
    pub(crate) fn try_block_on<F: Future>(&self, future: F) -> Result<F::Output> {
        let shared = self.shared.read().unwrap();
        let runtime = shared.as_ref().ok_or_else(|| anyhow!("native runtime is closed"))?;
        let mut future = std::pin::pin!(future);
        Ok(runtime.block_on(std::future::poll_fn(|task| {
            // Native filesystem effects consult the ambient Cx. Mirror our
            // runtime-owned application principal for every request poll so
            // those effects belong to the same application child region.
            let _guard = Cx::set_current(Some(self.cx.clone()));
            future.as_mut().poll(task)
        })))
    }
}
pub(crate) const MAX_BLOCKING_THREADS: usize = 3;

pub struct NativeRuntime {
    handle: Handle,
    region: Option<ChildRegion>,
    diagnostics: shutdown_diagnostics::ShutdownDiagnostics,
}
impl NativeRuntime {
    pub fn new() -> Result<Self> {
        shutdown_diagnostics::prepare_output();
        let runtime = RuntimeBuilder::new()
            .worker_threads(1)
            .thread_name_prefix("fbittorrent-engine")
            // Initial validation admits one torrent with at most two checksum
            // workers. Keep control I/O (notably durable Stop intent) runnable
            // while both workers are occupied, so it can cancel that check.
            .blocking_threads(1, MAX_BLOCKING_THREADS)
            .enable_time()
            .build()?;
        // The daemon may settle millions of file I/O obligations without
        // restarting. Keep recent history rather than every completed record.
        runtime.set_resolved_obligation_history_limit(1024);
        let root = runtime.request_cx_with_budget(Budget::INFINITE);
        let region = runtime.block_on(root.open_child_region(ChildRegionSpec::inherit()))?;
        let cx = region.cx().clone();
        let diagnostics = shutdown_diagnostics::ShutdownDiagnostics::new(&runtime, cx.region_id())?;
        Ok(Self {
            handle: Handle {
                shared: Arc::new(RwLock::new(Some(runtime))),
                cx,
            },
            region: Some(region),
            diagnostics,
        })
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub fn cx(&self) -> &Cx {
        self.handle.cx()
    }
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.handle.block_on(future)
    }
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Result<()> {
        self.drain_and_shutdown(timeout)
    }
    fn drain_and_shutdown(&mut self, timeout: Duration) -> Result<()> {
        let began = Instant::now();
        if self.handle.shared.read().unwrap().is_none() {
            return Ok(());
        }
        let diagnostics = &self.diagnostics;
        #[cfg(feature = "chunk-diagnostics")]
        let emit_chunk_diagnostics = self.region.is_some();
        #[cfg(feature = "epoch-diagnostics")]
        let before_close = epoch_receipt::capture(&self.handle.shared, "before_app_region_close");
        let mut failures = Vec::new();
        if let Some(region) = self.region.take() {
            if let Err(error) = region.cancel(CancelReason::user("application stopped")) {
                failures.push(diagnostics.failure(
                    &format!("cancelling native application region: error={error:?}"),
                    began,
                ));
            }
            // A close receipt proves quiescence. The timeout is the liveness
            // fuse; expiry is an explicit failure, not a successful drain.
            let drained = {
                let shared = self.handle.shared.read().unwrap();
                let runtime = shared.as_ref().expect("native runtime is closed");
                let cleanup = runtime.request_cx_with_budget(Budget::INFINITE);
                // Cleanup keeps the runtime root principal: the application
                // principal is cancelled and its Sleep cannot enforce a fuse.
                runtime.block_on(rqbit_runtime::time::timeout(
                    &cleanup,
                    timeout.saturating_sub(began.elapsed()),
                    region.close(),
                ))
            };
            match drained {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(diagnostics.failure(
                    &format!("closing native application region: error={error:?}"),
                    began,
                )),
                Err(_) => {
                    failures.push(diagnostics.failure(
                        "native application region did not drain before its deadline",
                        began,
                    ));
                }
            }
        }
        #[cfg(feature = "epoch-diagnostics")]
        let after_close = epoch_receipt::capture(
            &self.handle.shared,
            "after_app_region_close_before_runtime_teardown",
        );
        let runtime = self.handle.shared.write().unwrap().take();
        if runtime.is_some_and(|runtime| {
            !runtime.shutdown_timeout(timeout.saturating_sub(began.elapsed()))
        }) {
            failures
                .push(diagnostics.failure("native runtime teardown exceeded its deadline", began));
        }
        #[cfg(feature = "epoch-diagnostics")]
        if let Some(before_close) = before_close {
            eprintln!(
                "{}",
                serde_json::json!({"schema": "fbittorrent.epoch_attribution.v1", "pid": std::process::id(), "before_close": before_close, "after_close": after_close})
            );
        }
        #[cfg(feature = "chunk-diagnostics")]
        if emit_chunk_diagnostics {
            eprintln!(
                "{}",
                serde_json::json!({
                    "schema": "fbittorrent.chunk_attribution.v1",
                    "pid": std::process::id(),
                    "scope": "process_cumulative",
                    "teardown_succeeded": failures.is_empty(),
                    "counters": librqbit::chunk_diagnostic_snapshot()
                })
            );
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(failures.join("; ")))
        }
    }
}
impl Drop for NativeRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.drain_and_shutdown(Duration::from_secs(2)) {
            shutdown_diagnostics::emit(
                format!("ERROR engine closing native runtime: error={:?}\n", format!("{error:#}")),
                "ERROR engine closing native runtime: teardown failed; diagnostic output unavailable\n".into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_than_one_thousand_owned_regions_admit_and_close() {
        const CHILD: &str = "FBT_REGION_CAPACITY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // A production engine owns one native runtime in its process.
            // Keep this capacity probe independent of the other tests' global
            // resource monitors and runtime startup/shutdown races. The same
            // 1,100 simultaneous regions and checked closes run in the child.
            let name = format!("{}::more_than_one_thousand_owned_regions_admit_and_close", module_path!().split_once("::").unwrap().1);
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"]).env(CHILD, "1")
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while child.try_wait().unwrap().is_none() {
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    let output = child.wait_with_output().unwrap();
                    panic!("native capacity probe hung: {output:?}");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "probe must execute: {output:?}");
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return;
        }
        let began = Instant::now();
        eprintln!("CAPACITY starting");
        let runtime = NativeRuntime::new().unwrap();
        eprintln!("CAPACITY runtime_created elapsed_ms={}", began.elapsed().as_millis());
        runtime.block_on(async {
            let mut regions = Vec::new();
            for index in 0..1100 {
                regions.push(runtime.cx().open_child_region(ChildRegionSpec::inherit())
                    .await.unwrap_or_else(|error| panic!("region {index}: {error}")));
                if index % 250 == 0 { eprintln!("CAPACITY admitted={} elapsed_ms={}", index + 1, began.elapsed().as_millis()); }
            }
            assert_eq!(regions.iter().map(ChildRegion::region_id).collect::<std::collections::HashSet<_>>().len(), 1100,
                "all admitted regions must coexist with distinct identities");
            eprintln!("CAPACITY admitted=1100 elapsed_ms={}", began.elapsed().as_millis());
            for (index, region) in regions.into_iter().enumerate() {
                region.cancel(CancelReason::user("capacity probe finished")).unwrap();
                region.close().await.unwrap();
                if index % 250 == 0 { eprintln!("CAPACITY closed={} elapsed_ms={}", index + 1, began.elapsed().as_millis()); }
            }
        });
        eprintln!("CAPACITY closed=1100 elapsed_ms={}", began.elapsed().as_millis());
        runtime.shutdown_timeout(Duration::from_secs(2)).unwrap();
        eprintln!("CAPACITY shutdown elapsed_ms={}", began.elapsed().as_millis());
    }

    use std::task::Poll;

    #[test]
    fn closed_runtime_refuses_external_reader_polls_without_panicking() {
        let runtime = NativeRuntime::new().unwrap();
        let reader = runtime.handle();
        runtime.shutdown_timeout(Duration::from_secs(2)).unwrap();
        assert!(reader.try_block_on(async { 42 }).unwrap_err().to_string().contains("closed"));
    }

    #[test]
    fn requests_use_the_application_region_for_native_effects() {
        let runtime = NativeRuntime::new().unwrap();
        runtime.block_on(async {
            let ambient = Cx::current().unwrap();
            assert_eq!(ambient.region_id(), runtime.cx().region_id());
            rqbit_runtime::task::yield_now().await;
            assert_eq!(Cx::current().unwrap().region_id(), runtime.cx().region_id());
        });
        runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn cancelling_a_parked_native_file_task_closes_its_descriptor() {
        let path = std::env::temp_dir().join(format!(
            "fbittorrent-native-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"native file owner").unwrap();
        let runtime = NativeRuntime::new().unwrap();
        let (started_tx, mut started_rx) = rqbit_runtime::channel::oneshot::channel();
        let file_path = path.clone();
        let task = rqbit_runtime::task::spawn_with(runtime.cx(), move |cx| async move {
            let file = rqbit_runtime::fs::File::open(file_path).await?;
            let mut sleeping =
                std::pin::pin!(rqbit_runtime::time::sleep(&cx, Duration::from_secs(120)));
            let mut started = Some(started_tx);
            std::future::poll_fn(|poll| match sleeping.as_mut().poll(poll) {
                Poll::Pending => {
                    if let Some(sender) = started.take() {
                        sender.send(&cx, ()).unwrap();
                    }
                    Poll::Pending
                }
                Poll::Ready(()) => Poll::Ready(()),
            })
            .await;
            cx.checkpoint()?;
            drop(file);
            Ok::<(), anyhow::Error>(())
        });
        runtime.block_on(started_rx.recv(runtime.cx())).unwrap();
        let descriptor_is_open = || {
            std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == path))
        };
        assert!(
            descriptor_is_open(),
            "the parked task must own the real file"
        );
        runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
        assert!(task.is_finished());
        assert!(
            !descriptor_is_open(),
            "the drained region must release its file"
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
#[path = "native_runtime/storage_ownership_tests.rs"]
mod storage_ownership_tests;

#[cfg(test)]
#[path = "native_runtime/obligation_history_tests.rs"]
mod obligation_history_tests;

#[cfg(test)]
#[path = "native_runtime/shutdown_tests.rs"]
mod shutdown_tests;

#[path = "native_runtime_diagnostics.rs"]
mod shutdown_diagnostics;

#[cfg(feature = "epoch-diagnostics")]
#[path = "native_runtime/epoch_receipt.rs"]
mod epoch_receipt;
