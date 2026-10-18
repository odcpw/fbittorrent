//! Deadline-only observations. Inspectors retain state, never RuntimeInner.
use asupersync::{
    observability::task_inspector::{SnapshotLimits, TaskInspector, TaskInspectorConfig},
    runtime::{blocking_pool::BlockingPoolHandle, Runtime},
    time::{TimeSource, WallClock},
    types::RegionId,
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::{mpsc, OnceLock},
    time::{Duration, Instant},
};

const SNAPSHOT_BYTES: usize = 3072;
const OUTPUT_WAIT: Duration = Duration::from_millis(10);
struct Output {
    detail: String,
    fallback: String,
    finished: mpsc::SyncSender<()>,
}
static OUTPUT: OnceLock<Option<mpsc::SyncSender<Output>>> = OnceLock::new();

// Initialize before shutdown, never wait for a logger thread to start after a
// deadline. One worker and one queued record bound retained memory even if an
// output device stalls in open/write. No runtime/state ownership reaches it.
pub(super) fn prepare_output() {
    OUTPUT.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Output>(1);
        std::thread::Builder::new()
            .name("fbt-deadline-log".into())
            .spawn(move || {
                for record in rx {
                    // Journald/socket stderr cannot be reopened through procfs.
                    // MSG_DONTWAIT bounds a socket send without changing the
                    // shared file-description flags used by other loggers.
                    let line = if record.detail.len() <= libc::PIPE_BUF as usize {
                        &record.detail
                    } else {
                        &record.fallback
                    };
                    let sent = unsafe {
                        libc::send(
                            libc::STDERR_FILENO,
                            line.as_ptr().cast(),
                            line.len(),
                            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                        )
                    };
                    if sent >= 0
                        || std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOTSOCK)
                    {
                        if sent < 0 {
                            let _ = unsafe {
                                libc::send(
                                    libc::STDERR_FILENO,
                                    record.fallback.as_ptr().cast(),
                                    record.fallback.len(),
                                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                                )
                            };
                        }
                        let _ = record.finished.try_send(());
                        continue;
                    }
                    // A fresh file description avoids changing stderr's O_NONBLOCK
                    // flag for the rest of the process. Append preserves log files.
                    if let Ok(mut file) = OpenOptions::new()
                        .append(true)
                        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                        .open("/proc/self/fd/2")
                    {
                        // <= PIPE_BUF: a nonblocking full pipe writes nothing. A
                        // short failure line is attempted if detail cannot fit.
                        if record.detail.len() > libc::PIPE_BUF as usize
                            || file.write(record.detail.as_bytes()).ok()
                                != Some(record.detail.len())
                        {
                            let _ = file.write(record.fallback.as_bytes());
                        }
                    }
                    let _ = record.finished.try_send(());
                }
            })
            .ok()
            .map(|_| tx)
    });
}

pub(super) fn emit(detail: String, fallback: String) {
    let Some(Some(output)) = OUTPUT.get() else {
        return;
    };
    let (finished, receipt) = mpsc::sync_channel(1);
    if output
        .try_send(Output {
            detail,
            fallback,
            finished,
        })
        .is_ok()
    {
        // Normal output is ordered before the caller's combined error line.
        // A stalled device/worker never extends this fixed wait.
        let _ = receipt.recv_timeout(OUTPUT_WAIT);
    }
}

struct Observer {
    tasks: TaskInspector,
    blocking_pool: Option<BlockingPoolHandle>,
    application: RegionId,
}

impl Observer {
    pub(super) fn new(runtime: &Runtime, application: RegionId) -> Self {
        Self {
            tasks: runtime.task_inspector(TaskInspectorConfig::default()),
            blocking_pool: runtime.blocking_handle(),
            application,
        }
    }

    fn snapshot(&self) -> Value {
        let began = Instant::now();
        // Native clocks share this concrete monotonic epoch. Sample outside
        // runtime locks; the bounded API never dispatches a user clock callback.
        let observed_at = Some(WallClock::new().now());
        let mut bounded = self.tasks.bounded_snapshot(
            SnapshotLimits {
                max_tasks: 16,
                max_regions: 16,
                max_bytes: SNAPSHOT_BYTES,
                time_cap: Duration::from_millis(5),
                observed_at,
            },
            &[self.application],
        );
        // A transient lock holder or preemption can exhaust one tiny sample.
        // Retry without waiting on locks, at most twice and within 10 ms total;
        // a permanently held lock still returns explicitly partial data. The
        // caller's independent 20 ms fuse remains the hard liveness bound.
        for _ in 0..2 {
            if !bounded.partial
                || !(bounded.json.contains("time_limit") || bounded.json.contains("contention"))
            {
                break;
            }
            let remaining = Duration::from_millis(10).saturating_sub(began.elapsed());
            if remaining.is_zero() {
                break;
            }
            let candidate = self.tasks.bounded_snapshot(
                SnapshotLimits {
                    max_tasks: 16,
                    max_regions: 16,
                    max_bytes: SNAPSHOT_BYTES,
                    time_cap: remaining.min(Duration::from_millis(5)),
                    observed_at,
                },
                &[self.application],
            );
            if !candidate.partial || candidate.json.len() > bounded.json.len() {
                bounded = candidate;
            }
        }
        let mut snapshot: Value = serde_json::from_str(&bounded.json).unwrap_or_else(|_| {
            json!({
                "tasks": [], "regions": [], "partial": true, "truncated": true,
                "reasons": ["snapshot_unavailable"],
            })
        });
        for region in snapshot["regions"].as_array_mut().unwrap() {
            region["name"] = if region["id"] == format!("{:?}", self.application) {
                "native_application"
            } else if region["parent"].is_null() {
                "native_runtime_root"
            } else {
                "child_region"
            }
            .into();
        }
        snapshot["blocking_pool"] = self
            .blocking_pool
            .as_ref()
            .map(|pool| {
                json!({
                    "name": "native_blocking_pool", "busy_threads": pool.busy_threads(),
                    "pending_jobs": pool.pending_count(), "active_threads": pool.active_threads(),
                })
            })
            .unwrap_or(Value::Null);
        snapshot["runtime_ownership"] = "not exposed by runtime API".into();
        snapshot
    }

    fn failure(&self, message: &str, began: Instant) -> String {
        // Error construction is independent of the logger and its receipt.
        // Fixed count/byte limits bound post-copy serialization as well.
        let snapshot = self.snapshot();
        let detail = format!(
            "{message} elapsed_ms={} outstanding={snapshot}",
            began.elapsed().as_millis()
        );
        detail
    }
}

struct Request {
    message: String,
    began: Instant,
    reply: mpsc::SyncSender<String>,
}

// A separate observer thread makes the caller's time ceiling hard even if
// allocation/formatting or an unforeseen observer implementation stalls. The
// worker also uses the try-lock API and its own small work/time ceilings; it
// never waits on a runtime-owned lock. One pending request bounds backlog.
pub(super) struct ShutdownDiagnostics {
    requests: mpsc::SyncSender<Request>,
}
impl ShutdownDiagnostics {
    pub(super) fn new(runtime: &Runtime, application: RegionId) -> anyhow::Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let (ready, started) = mpsc::sync_channel(1);
        let runtime = runtime.clone();
        std::thread::Builder::new()
            .name("fbt-deadline-snapshot".into())
            .spawn(move || {
                let observer = Observer::new(&runtime, application);
                // Startup rendezvous proves that the observer holds only state,
                // not a strong Runtime which would itself prevent final teardown.
                drop(runtime);
                if ready.send(()).is_err() {
                    return;
                }
                for request in receiver {
                    let detail = observer.failure(&request.message, request.began);
                    let _ = request.reply.try_send(detail);
                }
            })?;
        started.recv()?;
        Ok(Self { requests })
    }

    pub(super) fn failure(&self, message: &str, began: Instant) -> String {
        let (reply, receipt) = mpsc::sync_channel(1);
        let detail = if self
            .requests
            .try_send(Request {
                message: message.into(),
                began,
                reply,
            })
            .is_ok()
        {
            receipt.recv_timeout(Duration::from_millis(20)).ok()
        } else {
            None
        };
        // Error propagation never depends on observer/logging success. Do not
        // wait for or join the worker after its fuse, nor queue unlimited jobs.
        let detail = detail.unwrap_or_else(|| format!("{message} elapsed_ms={} outstanding={{\"tasks\":[],\"regions\":[],\"partial\":true,\"truncated\":true,\"reasons\":[\"observer_deadline\"]}}", began.elapsed().as_millis()));
        // Exactly one output attempt per phase. A late observer reply is
        // dropped; it cannot emit duplicate or reordered deadline records.
        let fallback = format!("ERROR engine {message} elapsed_ms={} outstanding={{\"tasks\":[],\"regions\":[],\"partial\":true,\"reasons\":[\"output_unavailable\"]}}\n", began.elapsed().as_millis());
        emit(format!("ERROR engine {detail}\n"), fallback);
        detail
    }
}
