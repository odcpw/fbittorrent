//! The torrent engine belongs to the open application, including its runtime.
//!
//! We persist add options ourselves: librqbit's session JSON does not preserve
//! per-torrent upload limits. Every restore therefore applies the cap before
//! peers can exchange payload. Completed stop-on-complete records stay dormant;
//! admitting one for transfer always checks its data first.

use crate::native_runtime::{Handle, NativeRuntime};
use anyhow::{anyhow, bail, Context, Result};
use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions,
    ListOnlyResponse, ManagedTorrent, Session, SessionOptions, TorrentStatsState,
};
use rqbit_runtime::{
    io::AsyncWriteExt,
    sync::{CancellationToken, Mutex, OwnedMutexGuard},
    task, time, Cx,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::daemon::lifecycle::Lifecycle;
use crate::ipc::InstanceLock;
use crate::qbt::{TorrentFile, TransferInfo};

mod ledger;
pub(crate) mod rename;
pub(crate) mod staging;
pub(crate) mod pieces;
mod bus;
mod watch;
pub(crate) mod playback;
pub(crate) mod peers;
mod tracker_edit;
mod queue;
mod queue_policy;
mod queue_ops;
mod trackers;
pub(crate) mod moving;
pub mod recheck;
pub mod undo;
pub(crate) mod resume;
mod undo_ops;
mod settings;
pub(crate) mod disk;
pub mod ops;
mod restore;
mod dormant;
mod saved;
mod dormant_delete;
pub mod view;
mod parked;

use ledger::{Record, Ledger, read_ledger, save_parked_ledger};
use ops::{mutation_hashes, now};
pub use parked::{ParkedLedger, ParkedRefusal};
pub use ledger::LedgerGeneration;
pub use dormant::InvalidTorrentState;

static RESTORE_GENERATION: AtomicUsize = AtomicUsize::new(0);

/// In-memory restore results, available independently of live torrent handles.
/// P1-03 can use these to publish engine changes without interpreting legacy JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Dormant,
    Checking,
    Metadata,
    Restored,
    Stopped,
    Failed { reason: &'static str, message: String },
}

struct RestoreTask {
    generation: usize,
    outcome: RestoreOutcome,
    // Retain the last diagnostic while another attempt is in flight. A
    // pending retry is not evidence that the previous fault has been fixed.
    last_error: Option<String>,
    task: Option<task::JoinHandle<()>>,
    cancel: CancellationToken,
}

struct Inner {
    session: Arc<Session>,
    bus_changed: rqbit_runtime::sync::Notify,
    completed_count: std::sync::atomic::AtomicU64,
    completions: std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>,
    ledger: Arc<Mutex<Ledger>>,
    additions: Arc<Mutex<()>>,
    operations: Arc<Mutex<()>>,
    moves: moving::Moves,
    directory: PathBuf,
    default_folder: PathBuf,
    rechecks: recheck::Rechecks,
    /// Parsed saved metainfo and sampled payload presence (see saved.rs).
    saved: saved::SavedCache,
    resume: Arc<resume::Store>,
    resume_prepared: resume::Prepared,
    undo: std::sync::Mutex<Arc<undo::History>>,
    /// The owner's rate-history sink, fed from this engine's 1 s tick.
    rates: std::sync::Mutex<Option<RateSink>>,
    stopping: AtomicBool,
    max_active_downloads: AtomicUsize,
    queue_pins: std::sync::Mutex<HashSet<String>>,
    network_blocked: AtomicBool,
    restores: std::sync::Mutex<HashMap<String, RestoreTask>>,
    // Keep background restores from filling the ledger's FIFO with hundreds
    // of admissions ahead of foreground snapshots and user mutations.
    restore_admission: rqbit_runtime::sync::Semaphore,
    // Removing/retrying an intent does not end the previous task's cleanup.
    retired_restores: std::sync::Mutex<Vec<task::JoinHandle<()>>>,
    #[cfg(test)]
    restore_join_started: std::sync::Mutex<Option<CancellationToken>>,
    // Initializing.paused is transfer intent, not check activity. Only a
    // successfully drained pause cancels an initial check. Track the handle
    // identity so rebuilding the same hash cannot inherit an old cancellation.
    cancelled_checks: std::sync::Mutex<HashMap<String, Weak<ManagedTorrent>>>,
}

/// Receives (download, upload) bytes/s once per engine tick.
pub(crate) type RateSink = Arc<dyn Fn(u64, u64) + Send + Sync>;

#[derive(Clone)]
pub struct EngineDispatcher {
    generation: Arc<LedgerGeneration>,
    lifecycle: Lifecycle,
    inner: Arc<Inner>,
    runtime: Handle,
}

pub struct AppEngine {
    dispatcher: EngineDispatcher,
    runtime: Option<NativeRuntime>,
    // Released only after the socket, workers, session and runtime have stopped.
    _instance: Arc<InstanceLock>,
}

/// Start one replaceable session under the daemon owner's existing lock. The
/// listener and the owner's Arc stay alive while this engine drains/restarts.
pub(crate) fn start_with_lock(cfg: &Config, lifecycle: Lifecycle, instance: Arc<InstanceLock>, resume: Option<Arc<resume::Store>>) -> Result<AppEngine> {
    cfg.validate()?;
    start_shared(cfg, lifecycle, crate::config::data_dir().join("engine"), instance, SessionOptions {
        peer_encryption: match cfg.peer_encryption.as_str() {
            "off" => librqbit::PeerEncryption::Off,
            "require" => librqbit::PeerEncryption::Require,
            _ => librqbit::PeerEncryption::Prefer,
        },
        bind_device_name: cfg.network_interface.clone(),
        listen: Some(ListenerOptions {
            mode: ListenerMode::TcpOnly,
            listen_addr: ([0, 0, 0, 0], cfg.peer_port).into(),
            enable_upnp_port_forwarding: false,
            ..Default::default()
        }),
        concurrent_init_limit: Some(1),
        runtime_worker_threads: Some(2),
        peer_limit: Some(96),
        persistence: None,
        fastresume: false,
        ..Default::default()
    }, resume)
}

// The production startup path, with explicit paths/options for local-only tests.
#[cfg(test)]
fn start_in(cfg: &Config, lifecycle: Lifecycle, directory: PathBuf, instance: InstanceLock, options: SessionOptions) -> Result<AppEngine> {
    start_shared(cfg, lifecycle, directory, Arc::new(instance), options, None)
}

fn start_shared(cfg: &Config, lifecycle: Lifecycle, directory: PathBuf, instance: Arc<InstanceLock>, mut options: SessionOptions, resume: Option<Arc<resume::Store>>) -> Result<AppEngine> {
    options.upload_enabled = cfg.upload == "on";
    std::fs::create_dir_all(&directory)?;
    let ledger_path = directory.join("torrents.json");
    let ledger = read_ledger(&ledger_path)?;
    let resume=resume.map(Ok).unwrap_or_else(||resume::Store::open(&directory).map(Arc::new))?;
    let resume_prepared=resume.prepare(&directory,&ledger.records)?;
    // Both TUI start() and daemon start_with_lock() enter here. Capture the
    // same generation before moving the ledger or starting background restores.
    let generation = ledger.generation.clone();
    let default_folder = cfg
        .save_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::download_dir().unwrap_or_else(|| crate::config::data_dir().join("downloads"))
        });
    std::fs::create_dir_all(&default_folder)?;
    let runtime = NativeRuntime::new()?;
    let cx = runtime.cx();
    let session = runtime
        .block_on(Session::new_with_opts(
            cx,
            default_folder.clone(),
            options,
        ))
        .context("starting the native torrent engine")?;
    settings::apply_limits(&session, cfg);
    let inner = Arc::new(Inner {
        session,
        bus_changed: rqbit_runtime::sync::Notify::new(),
        completed_count: Default::default(),
        completions: Default::default(),
        ledger: Arc::new(Mutex::new(ledger)),
        additions: Arc::new(Mutex::new(())),
        operations: Default::default(), moves: Default::default(),
        restore_admission: rqbit_runtime::sync::Semaphore::new(4),
        directory,
        default_folder,
        rechecks: Default::default(),
        saved: Default::default(),
        resume, resume_prepared,
        undo: Default::default(),
        rates: Default::default(),
        stopping: AtomicBool::new(false),
        max_active_downloads: AtomicUsize::new(cfg.max_active_downloads),
        queue_pins: Default::default(),
        network_blocked: AtomicBool::new(false),
        restores: std::sync::Mutex::new(HashMap::new()),
        retired_restores: std::sync::Mutex::new(Vec::new()),
        #[cfg(test)]
        restore_join_started: std::sync::Mutex::new(None),
        cancelled_checks: std::sync::Mutex::new(HashMap::new()),
    });
    let records = runtime.block_on(async {
        Ok::<_, anyhow::Error>(OwnedMutexGuard::lock(inner.ledger.clone(), cx).await?.records.clone())
    })?;
    inner.begin_restores(cx, records)?;
    let dispatcher = EngineDispatcher {
        generation,
        lifecycle,
        inner: inner.clone(),
        runtime: runtime.handle(),
    };
    task::spawn_with(cx, move |cx| async move {
        let cx = &cx;
        let mut interval = time::interval(cx, Duration::from_secs(1));
        loop {
            if cx.checkpoint().is_err() {
                break;
            }
            interval.tick(cx).await;
            if inner.stopping.load(Ordering::Acquire) {
                break;
            }
            // Before resume/queue work, which can wait for the ledger.
            inner.record_rates();
            if let Err(error) = inner.periodic_resume(cx).await {
                eprintln!("WARN resume periodic snapshot error={error:?}");
            }
            if !inner.maintenance_tick(cx).await { break; }
        }
    });
    Ok(AppEngine {
        dispatcher,
        runtime: Some(runtime),
        _instance: instance,
    })
}

impl AppEngine {
    pub(crate) fn is_quiescing(&self) -> bool {
        self.dispatcher.inner.stopping.load(Ordering::Acquire)
    }
    /// Interrupt in-flight legacy operations before waiting for the slot writer.
    /// Their dispatch select observes this token, so an unresolved add cannot
    /// hold shutdown behind its 90-second metadata timeout.
    pub(crate) fn quiesce(&self) {
        self.dispatcher.inner.stopping.store(true, Ordering::Release);
        self.dispatcher.inner.session.cancellation_token().cancel();
        self.dispatcher.inner.moves.cancel.cancel();
        self.dispatcher.inner.rechecks.cancel.cancel();
        self.dispatcher.inner.rechecks.cancel.cancel();
        for restore in self.dispatcher.inner.restores.lock().unwrap().values() {
            restore.cancel.cancel();
        }
    }
    pub fn dispatcher(&self) -> EngineDispatcher {
        self.dispatcher.clone()
    }
}

// Debug quoting escapes newline/control characters in chained error details.
// Retain the old diagnostic substrings consumed by the worker-failure scans.
fn drain_failure(message: &str, detail: Option<String>) -> String {
    match detail {
        Some(detail) => format!("ERROR engine {message} error={detail:?}"),
        None => format!("ERROR engine {message}"),
    }
}

impl AppEngine {
    // Drop uses the production budget; tests exercise the same path with a short fuse.
    pub(crate) fn drain(&mut self, bound: Duration) -> bool {
        let began = std::time::Instant::now();
        self.dispatcher
            .inner
            .stopping
            .store(true, Ordering::Release);
        self.dispatcher.inner.rechecks.cancel.cancel();
        for restore in self.dispatcher.inner.restores.lock().unwrap().values() {
            restore.cancel.cancel();
        }
        let mut drained = true;
        if let Some(runtime) = self.runtime.take() {
            let cx = runtime.cx();
            match runtime.block_on(time::timeout(cx,bound.saturating_sub(began.elapsed()),self.dispatcher.inner.moves.drain())) {
                Ok(Ok(()))=>{},
                result=>{drained=false;crate::native_runtime::log_teardown(drain_failure("joining payload moves",Some(format!("{result:?}"))));}
            }
            // Resolve the protocol stop and observe every restore's completion
            // while its application context and the session are still alive.
            // Cancelling their regions first races the resolver's close epilogue.
            match runtime.block_on(time::timeout(
                cx,
                bound.saturating_sub(began.elapsed()),
                self.dispatcher.inner.drain_restores(),
            )) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    drained = false;
                    crate::native_runtime::log_teardown(drain_failure(
                        "closing native torrent session:",
                        Some(format!("joining restore tasks: {error:#}")),
                    ));
                }
                Err(_) => {
                    drained = false;
                    crate::native_runtime::log_teardown(drain_failure(
                        "native torrent session did not drain before its deadline",
                        Some("joining restore tasks".into()),
                    ));
                }
            }
            match runtime.block_on(time::timeout(
                cx,
                bound.saturating_sub(began.elapsed()),
                self.dispatcher.inner.session.stop(cx),
            )) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    drained = false;
                    crate::native_runtime::log_teardown(drain_failure(
                        "closing native torrent session:",
                        Some(format!("{error:#}")),
                    ));
                }
                Err(_) => {
                    drained = false;
                    crate::native_runtime::log_teardown(drain_failure(
                        "native torrent session did not drain before its deadline",
                        None,
                    ));
                }
            }
            if drained {
                match runtime.block_on(time::timeout(cx,bound.saturating_sub(began.elapsed()),self.dispatcher.inner.save_resume(cx,true))) {
                    Ok(Ok(()))=>{},
                    result=>{self.dispatcher.inner.resume.abandon_clean();eprintln!("WARN resume clean snapshot unavailable result={result:?}");},
                }
            }
            if let Err(error) = runtime.shutdown_timeout(bound.saturating_sub(began.elapsed())) {
                drained = false;
                crate::native_runtime::log_teardown(drain_failure(
                    "closing native runtime:",
                    Some(format!("{error:#}")),
                ));
            }
            if !drained {self.dispatcher.inner.resume.abandon_clean();}
        }
        drained
    }
}

impl Drop for AppEngine {
    fn drop(&mut self) {
        self.drain(Duration::from_secs(4));
    }
}

#[cfg(test)]
mod tests;

// Integration regressions for the shared production startup path. The P1-19
// typed operation/fixture tests remain unchanged.
#[cfg(test)]
mod startup_reconciliation_tests {
    use super::*;

    #[test]
    fn drain_failures_use_single_line_engine_format() {
        for message in ["closing native torrent session:", "closing native runtime:"] {
            let error = anyhow!("disk error\nsecond line\rthird line").context("drain failed");
            let line = drain_failure(message, Some(format!("{error:#}")));
            assert_eq!(line, format!("ERROR engine {message} error=\"drain failed: disk error\\nsecond line\\rthird line\""));
            assert_eq!(line.lines().count(), 1);
            assert!(line.contains(message), "existing failure scanner must still match");
        }
        assert_eq!(drain_failure("native torrent session did not drain before its deadline", None),
            "ERROR engine native torrent session did not drain before its deadline");
        println!("verified level/source, escaped details and scanner compatibility for all three drain diagnostics");
    }

    #[test]
    fn shared_startup_retains_generation_and_owner_lock() {
        let directory = std::env::temp_dir().join(format!("fbt-shared-start-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let downloads = directory.join("downloads");
        let cfg = Config { save_path: Some(downloads.to_string_lossy().into()), ..Default::default() };
        let owner = Arc::new(InstanceLock::acquire_in(directory.join("runtime")).unwrap());
        let lifecycle = Lifecycle::default();
        for _ in 0..2 {
            let app = start_shared(&cfg, lifecycle.clone(), directory.clone(), owner.clone(), SessionOptions {
                dht: None, disable_trackers: true, disable_local_service_discovery: true, ..Default::default()
            }, None).unwrap();
            let dispatcher = app.dispatcher();
            let generation = dispatcher.ledger_generation();
            let before = generation.current();
            dispatcher.runtime.block_on(async {
                let cx = dispatcher.runtime.cx();
                let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx).await.unwrap();
                assert!(Arc::ptr_eq(&generation, &ledger.generation));
                dispatcher.inner.save(cx, &ledger).await.unwrap();
            });
            assert_eq!(generation.wait_after(before, Duration::ZERO), Some(before + 1));
            app.quiesce();
            assert!(app.is_quiescing());
            drop(app);
            assert!(InstanceLock::acquire_in(directory.join("runtime")).is_err());
            println!("shared startup generation advanced to {}; owner lock survives quiesce/drop", generation.current());
        }
        drop(owner);
        drop(InstanceLock::acquire_in(directory.join("runtime")).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
