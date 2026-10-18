//! App-owned Asupersync runtime. Cloned dispatch handles borrow this owner's
//! runtime; shutdown closes admissions and drains the application child region.
use anyhow::{Result, anyhow};
use asupersync::{
    Cx,
    cx::{ChildRegion, ChildRegionSpec},
    runtime::{Runtime, RuntimeBuilder},
    types::{Budget, CancelReason},
};
use std::{
    future::Future,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

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
        let shared = self.shared.read().unwrap();
        let runtime = shared.as_ref().expect("native runtime is closed");
        let mut future = std::pin::pin!(future);
        runtime.block_on(std::future::poll_fn(|task| {
            // Native filesystem effects consult the ambient Cx. Mirror our
            // runtime-owned application principal for every request poll so
            // those effects belong to the same application child region.
            let _guard = Cx::set_current(Some(self.cx.clone()));
            future.as_mut().poll(task)
        }))
    }
}
pub struct NativeRuntime {
    handle: Handle,
    region: Option<ChildRegion>,
}
impl NativeRuntime {
    pub fn new() -> Result<Self> {
        let runtime = RuntimeBuilder::new()
            .worker_threads(1)
            .thread_name_prefix("fbittorrent-engine")
            .blocking_threads(1, 2)
            .enable_time()
            .build()?;
        let root = runtime.request_cx_with_budget(Budget::INFINITE);
        let region = runtime.block_on(root.open_child_region(ChildRegionSpec::inherit()))?;
        let cx = region.cx().clone();
        Ok(Self {
            handle: Handle {
                shared: Arc::new(RwLock::new(Some(runtime))),
                cx,
            },
            region: Some(region),
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
        #[cfg(feature = "chunk-diagnostics")]
        let emit_chunk_diagnostics = self.region.is_some();
        #[cfg(feature = "epoch-diagnostics")]
        let before_close = epoch_receipt::capture(&self.handle.shared, "before_app_region_close");
        let began = Instant::now();
        let mut failure = None;
        if let Some(region) = self.region.take() {
            if let Err(error) = region.cancel(CancelReason::user("application stopped")) {
                failure = Some(anyhow!(error));
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
                    timeout,
                    region.close(),
                ))
            };
            match drained {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failure = Some(anyhow!(error)),
                Err(_) => {
                    failure = Some(anyhow!(
                        "native application region did not drain before its deadline"
                    ))
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
            failure = Some(anyhow!("native runtime teardown exceeded its deadline"));
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
                    "teardown_succeeded": failure.is_none(),
                    "counters": librqbit::chunk_diagnostic_snapshot()
                })
            );
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for NativeRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.drain_and_shutdown(Duration::from_secs(2)) {
            eprintln!("closing native runtime: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Poll;

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

#[cfg(feature = "epoch-diagnostics")]
#[path = "native_runtime/epoch_receipt.rs"]
mod epoch_receipt;
