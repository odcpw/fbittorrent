//! The torrent engine belongs to the open application, including its runtime.
//!
//! We persist add options ourselves: librqbit's session JSON does not preserve
//! per-torrent upload limits. Every restore therefore applies the cap before
//! peers can exchange payload, and existing files are hash checked on opening.

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
use crate::daemon::lifecycle::{Lifecycle, StopReason};
use crate::ipc::{InstanceLock, Request, Server};
use crate::qbt::{Torrent, TorrentFile, TransferInfo, NO_UPLOAD_LIMIT, SEED_TAG};

mod ledger;
mod queue;
pub mod ops;
mod restore;
pub mod view;
mod parked;
mod legacy_import;

use ledger::{Record, Ledger, read_ledger, save_parked_ledger};
use ops::{mutation_hashes, json, now};
#[cfg(test)]
use parked::dispatch_parked_in;
#[allow(unused_imports)] // Preserve the old compatibility entry point for P1-10.
pub use parked::{dispatch_parked, ParkedLedger};
pub use ledger::LedgerGeneration;
pub(crate) use legacy_import::migrate_legacy;
#[cfg(test)]
pub(crate) use legacy_import::LegacyMigration;

static RESTORE_GENERATION: AtomicUsize = AtomicUsize::new(0);

/// In-memory restore results, available independently of live torrent handles.
/// P1-03 can use these to publish engine changes without interpreting legacy JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
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
    ledger: Arc<Mutex<Ledger>>,
    additions: Arc<Mutex<()>>,
    directory: PathBuf,
    default_folder: PathBuf,
    stopping: AtomicBool,
    max_active_downloads: AtomicUsize,
    network_blocked: AtomicBool,
    restores: std::sync::Mutex<HashMap<String, RestoreTask>>,
    // Initializing.paused is transfer intent, not check activity. Only a
    // successfully drained pause cancels an initial check. Track the handle
    // identity so rebuilding the same hash cannot inherit an old cancellation.
    cancelled_checks: std::sync::Mutex<HashMap<String, Weak<ManagedTorrent>>>,
}

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
    server: Option<Server>,
    // Released only after the socket, workers, session and runtime have stopped.
    instance: InstanceLock,
}

pub fn start(cfg: &Config, lifecycle: Lifecycle) -> Result<AppEngine> {
    cfg.validate()?;
    let instance = InstanceLock::acquire()?;
    start_in(cfg, lifecycle, crate::config::data_dir().join("engine"), instance, SessionOptions {
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
    })
}

// The production startup path, with explicit paths/options for local-only tests.
fn start_in(cfg: &Config, lifecycle: Lifecycle, directory: PathBuf, instance: InstanceLock, options: SessionOptions) -> Result<AppEngine> {
    std::fs::create_dir_all(&directory)?;
    let ledger_path = directory.join("torrents.json");
    let ledger = read_ledger(&ledger_path)?;
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
    let generation = ledger.generation.clone();
    let inner = Arc::new(Inner {
        session,
        ledger: Arc::new(Mutex::new(ledger)),
        additions: Arc::new(Mutex::new(())),
        directory,
        default_folder,
        stopping: AtomicBool::new(false),
        max_active_downloads: AtomicUsize::new(cfg.max_active_downloads),
        network_blocked: AtomicBool::new(false),
        restores: std::sync::Mutex::new(HashMap::new()),
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
            if let Err(error) = inner.maintain(cx).await {
                // Keep the failure available without writing over a running TUI.
                let _ = inner.write_store(cx, "last-error.txt", format!("{error:#}\n").as_bytes()).await;
            }
        }
    });
    Ok(AppEngine {
        dispatcher,
        runtime: Some(runtime),
        server: None,
        instance,
    })
}

impl AppEngine {
    pub fn dispatcher(&self) -> EngineDispatcher {
        self.dispatcher.clone()
    }
    pub fn serve(
        &mut self,
        cfg: &Config,
        handler: impl Fn(&Request) -> Result<String> + Send + Sync + 'static,
    ) -> Result<()> {
        if self.server.is_some() {
            bail!("native engine socket is already open");
        }
        self.server = Some(Server::start(cfg, &self.instance, handler)?);
        Ok(())
    }
}

impl Drop for AppEngine {
    fn drop(&mut self) {
        let began = std::time::Instant::now();
        self.dispatcher
            .inner
            .stopping
            .store(true, Ordering::Release);
        for restore in self.dispatcher.inner.restores.lock().unwrap().values() {
            restore.cancel.cancel();
        }
        if let Some(server) = self.server.as_mut() {
            server.stop_accepting();
        }
        if let Some(runtime) = self.runtime.take() {
            let bound = Duration::from_secs(4);
            let cx = runtime.cx();
            let mut drained = false;
            match runtime.block_on(time::timeout(
                cx,
                bound.saturating_sub(began.elapsed()),
                self.dispatcher.inner.session.stop(cx),
            )) {
                Ok(Ok(())) => drained = true,
                Ok(Err(error)) => eprintln!("closing native torrent session: {error:#}"),
                Err(_) => eprintln!("native torrent session did not drain before its deadline"),
            }
            self.server.take();
            if let Err(error) = runtime.shutdown_timeout(bound.saturating_sub(began.elapsed())) {
                drained = false;
                eprintln!("closing native runtime: {error:#}");
            }
            if drained { eprintln!("INFO engine drained in {} ms", began.elapsed().as_millis()); }
        }
    }
}

#[cfg(test)]
mod tests;
