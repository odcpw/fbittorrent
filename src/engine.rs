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
    ManagedTorrent, Session, SessionOptions, TorrentStatsState,
};
use rqbit_runtime::{
    io::AsyncWriteExt,
    sync::{Mutex, OwnedMutexGuard},
    task, time, Cx,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::ipc::{InstanceLock, Request, Server};
use crate::qbt::{Torrent, TorrentFile, TransferInfo, NO_UPLOAD_LIMIT, SEED_TAG};

static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub fn quit_requested() -> bool {
    QUIT_REQUESTED.load(Ordering::Acquire)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    hash: String,
    name: String,
    source: String,
    output_folder: String,
    only_files: Option<Vec<usize>>,
    upload_limit: i64,
    ratio_limit: f64,
    category: String,
    tags: String,
    added_on: i64,
    completion_on: i64,
    // Durable user intent. Queue and completion suspension never rewrite this.
    paused: bool,
    #[serde(default)]
    resume_after_migration: bool,
}

impl Record {
    fn options(&self, paused: bool) -> Result<AddTorrentOptions> {
        if self.upload_limit < 0 {
            bail!("upload limit cannot be negative");
        }
        // rqbit reserves an entire protocol chunk from its rate limiter. A smaller
        // bucket would make a normal 16 KiB request impossible to serve.
        if self.upload_limit > 0 && self.upload_limit < 16 * 1024 {
            bail!("native upload limits must be zero (unlimited) or at least 16 KiB/s");
        }
        let upload_bps = if self.upload_limit > 0 {
            Some(
                NonZeroU32::new(
                    u32::try_from(self.upload_limit)
                        .context("upload limit exceeds the native engine maximum")?,
                )
                .context("invalid upload limit")?,
            )
        } else {
            None
        };
        Ok(AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: Some(self.output_folder.clone()),
            only_files: self.only_files.clone(),
            ratelimits: LimitsConfig {
                upload_bps,
                download_bps: None,
            },
            peer_limit: Some(96),
            ..Default::default()
        })
    }
    fn stop_on_complete(&self) -> bool {
        self.ratio_limit == 0.0
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    #[serde(default)]
    migrated_legacy: bool,
    records: Vec<Record>,
}

fn validate_info_hash(hash: &str) -> Result<()> {
    if hash.len() != 40
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("saved torrent hash must be 40 lowercase hexadecimal characters");
    }
    Ok(())
}

fn read_ledger(path: &Path) -> Result<Ledger> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let ledger: Ledger = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "reading saved torrent records {}; the file has been preserved",
            path.display()
        )
    })?;
    let mut hashes = HashSet::new();
    for record in &ledger.records {
        validate_info_hash(&record.hash).with_context(|| {
            format!(
                "invalid saved torrent record {} in {}",
                record.name,
                path.display()
            )
        })?;
        if !hashes.insert(&record.hash) {
            bail!(
                "duplicate saved torrent hash {} in {}; the file has been preserved",
                record.hash,
                path.display()
            );
        }
        record.options(true).with_context(|| {
            format!(
                "invalid saved options for {} in {}",
                record.name,
                path.display()
            )
        })?;
    }
    Ok(ledger)
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
}

impl Inner {
    fn torrent_path(&self, hash: &str) -> Result<PathBuf> {
        validate_info_hash(hash)?;
        Ok(self.directory.join(format!("{hash}.torrent")))
    }
    async fn save(&self, cx: &Cx, ledger: &Ledger) -> Result<()> {
        cx.checkpoint()?;
        let bytes = serde_json::to_vec_pretty(ledger)?;
        let temporary = self.directory.join("torrents.json.tmp");
        let final_path = self.directory.join("torrents.json");
        let mut file = rqbit_runtime::fs::File::create(&temporary).await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        drop(file);
        rqbit_runtime::fs::rename(&temporary, &final_path).await?;
        // Make both the replacement ledger and preceding metadata renames durable.
        rqbit_runtime::fs::File::open(&self.directory)
            .await?
            .sync_all()
            .await?;
        Ok(())
    }
    fn handle(&self, hash: &str) -> Result<Arc<ManagedTorrent>> {
        self.session
            .with_torrents(|torrents| {
                for (_, handle) in torrents {
                    if handle.info_hash().as_string() == hash {
                        return Some(handle.clone());
                    }
                }
                None
            })
            .ok_or_else(|| anyhow!("torrent not found: {hash}"))
    }
    async fn restore_record(
        &self,
        cx: &Cx,
        record: &Record,
        paused: bool,
    ) -> Result<Arc<ManagedTorrent>> {
        let path = self.torrent_path(&record.hash)?;
        let mut options = record.options(true)?;
        options.list_only = true;
        let add = if path.exists() {
            AddTorrent::from_bytes(rqbit_runtime::fs::read(path).await?)
        } else if !record.source.is_empty() {
            AddTorrent::from_url(record.source.clone())
        } else {
            bail!("missing torrent metadata for {}", record.name)
        };
        let response = time::timeout(
            cx,
            Duration::from_secs(90),
            self.session.add_torrent(cx, add, Some(options)),
        )
        .await
        .context("torrent metadata lookup timed out")??;
        let AddTorrentResponse::ListOnly(metadata) = response else {
            bail!("torrent metadata preflight returned an unexpected managed torrent");
        };
        if metadata.info_hash.as_string() != record.hash {
            bail!("restored torrent hash does not match saved record");
        }
        // A paused torrent still opens and verifies its files. Validate the hash
        // before admitting any storage, then apply the saved cap at admission.
        let mut options = record.options(paused)?;
        options.initial_peers = Some(metadata.seen_peers);
        let handle = self
            .session
            .add_torrent(
                cx,
                AddTorrent::from_bytes(metadata.torrent_bytes),
                Some(options),
            )
            .await?
            .into_handle()
            .context("torrent admission did not return a handle")?;
        Ok(handle)
    }
    async fn cache_metadata(&self, cx: &Cx, handle: &Arc<ManagedTorrent>) -> Result<()> {
        let metadata = handle
            .metadata
            .load_full()
            .context("torrent metadata is not available")?;
        self.write_metadata(cx, &handle.info_hash().as_string(), &metadata.torrent_bytes)
            .await?;
        Ok(())
    }
    async fn write_metadata(&self, cx: &Cx, hash: &str, bytes: &[u8]) -> Result<()> {
        cx.checkpoint()?;
        let final_path = self.torrent_path(hash)?;
        let temporary = self.directory.join(format!("{hash}.torrent.tmp"));
        let mut file = rqbit_runtime::fs::File::create(&temporary).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        rqbit_runtime::fs::rename(temporary, final_path).await?;
        Ok(())
    }
    async fn rebuild(&self, cx: &Cx, old: &Record, changed: &Record) -> Result<()> {
        let handle = self.handle(&old.hash)?;
        self.cache_metadata(cx, &handle).await?;
        self.session
            .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
            .await?;
        if let Err(error) = self.restore_record(cx, changed, true).await {
            // Restore the previous record if changing the cap cannot be applied.
            let rollback = self.restore_record(cx, old, true).await;
            if let Err(rollback) = rollback {
                bail!("changing upload cap failed: {error:#}; restoring previous torrent failed: {rollback:#}");
            }
            return Err(error).context("changing upload cap; previous torrent restored");
        }
        Ok(())
    }
    /// Apply durable running intent in FIFO order. Pause displaced torrents
    /// before admitting replacements, including checking downloads in the cap.
    async fn reconcile_queue(
        &self,
        cx: &Cx,
        ledger: &mut Ledger,
        retry_errors: &HashSet<String>,
    ) -> Result<()> {
        cx.checkpoint()?;
        if self.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let limit = self.max_active_downloads.load(Ordering::Acquire);
        let blocked = self.network_blocked.load(Ordering::Acquire);
        let mut slots = 0;
        let mut decisions = Vec::new();
        let mut changed = false;
        for record in &mut ledger.records {
            cx.checkpoint()?;
            let Ok(handle) = self.handle(&record.hash) else {
                continue;
            };
            let stats = handle.stats();
            if stats.finished && record.completion_on == 0 {
                record.completion_on = now();
                changed = true;
            }
            let retry = retry_errors.contains(&record.hash);
            let running = if blocked || record.paused {
                false
            } else if stats.finished {
                !record.stop_on_complete()
            } else if matches!(stats.state, TorrentStatsState::Error) && !retry {
                false
            } else if slots < limit {
                slots += 1;
                true
            } else {
                false
            };
            decisions.push((handle, running));
        }
        if changed {
            self.save(cx, ledger).await?;
        }
        // Stop first: reducing the limit or restarting an earlier queue entry
        // must never create a transient extra payload download.
        for (handle, running) in &decisions {
            cx.checkpoint()?;
            if !running
                && !handle.is_paused()
                && !matches!(handle.stats().state, TorrentStatsState::Error)
            {
                self.session.pause(cx, handle).await?;
            }
        }
        for (handle, running) in decisions {
            cx.checkpoint()?;
            if running
                && (handle.is_paused() || matches!(handle.stats().state, TorrentStatsState::Error))
            {
                self.session.unpause(cx, &handle).await?;
            }
        }
        Ok(())
    }
    async fn maintain(&self, cx: &Cx) -> Result<()> {
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await
    }
    async fn set_download_limit(&self, cx: &Cx, limit: usize) -> Result<()> {
        if limit == 0 {
            bail!("max_active_downloads must be at least 1");
        }
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        let old = self.max_active_downloads.load(Ordering::Acquire);
        let mut cfg = Config::load()?;
        let saved = cfg.max_active_downloads;
        let previous = cfg.clone();
        cfg.max_active_downloads = limit;
        if let Err(error) = cfg.save() {
            // A directory fsync can fail after rename. Restore the previous
            // setting even when the staged writer reported a failed save.
            if let Err(rollback) = previous.save() {
                bail!("saving download limit failed: {error:#}; restoring previous settings failed: {rollback:#}");
            }
            return Err(error).context("saving download limit; previous settings restored");
        }
        self.max_active_downloads.store(limit, Ordering::Release);
        if let Err(error) = self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await {
            self.max_active_downloads.store(old, Ordering::Release);
            cfg.max_active_downloads = saved;
            let persisted = cfg.save();
            let resumed = self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await;
            if let Err(rollback) = persisted.and(resumed) {
                bail!("changing download limit failed: {error:#}; restoring previous limit failed: {rollback:#}");
            }
            return Err(error).context("changing download limit; previous limit restored");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct EngineDispatcher {
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

pub fn start(cfg: &Config) -> Result<AppEngine> {
    cfg.validate()?;
    let instance = InstanceLock::acquire()?;
    QUIT_REQUESTED.store(false, Ordering::Release);
    let directory = crate::config::data_dir().join("engine");
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
            SessionOptions {
                peer_encryption: match cfg.peer_encryption.as_str() {
                    "off" => librqbit::PeerEncryption::Off,
                    "require" => librqbit::PeerEncryption::Require,
                    _ => librqbit::PeerEncryption::Prefer,
                },
                // The outer application resolves symbolic VPN selections before
                // admitting a session; only that exact device reaches sockets.
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
                // The native ledger applies every per-torrent cap before starting peers.
                persistence: None,
                fastresume: false,
                ..Default::default()
            },
        ))
        .context("starting the native torrent engine")?;
    let inner = Arc::new(Inner {
        session,
        ledger: Arc::new(Mutex::new(ledger)),
        additions: Arc::new(Mutex::new(())),
        directory,
        default_folder,
        stopping: AtomicBool::new(false),
        max_active_downloads: AtomicUsize::new(cfg.max_active_downloads),
        network_blocked: AtomicBool::new(false),
    });
    // Resolve from saved .torrent files first; downloads resume only in this runtime.
    runtime.block_on(async {
        let records = OwnedMutexGuard::lock(inner.ledger.clone(), cx)
            .await?
            .records
            .clone();
        for record in records {
            inner
                .restore_record(cx, &record, true)
                .await
                .with_context(|| format!("restoring {}", record.name))?;
        }
        inner.maintain(cx).await?;
        Ok::<_, anyhow::Error>(())
    })?;
    let dispatcher = EngineDispatcher {
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
                let _ = rqbit_runtime::fs::write(
                    inner.directory.join("last-error.txt"),
                    format!("{error:#}\n"),
                )
                .await;
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
        self.dispatcher
            .inner
            .stopping
            .store(true, Ordering::Release);
        if let Some(server) = self.server.as_mut() {
            server.stop_accepting();
        }
        if let Some(runtime) = self.runtime.take() {
            let began = std::time::Instant::now();
            let bound = Duration::from_secs(4);
            let cx = runtime.cx();
            match runtime.block_on(time::timeout(
                cx,
                bound,
                self.dispatcher.inner.session.stop(cx),
            )) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("closing native torrent session: {error:#}"),
                Err(_) => eprintln!("native torrent session did not drain before its deadline"),
            }
            self.server.take();
            if let Err(error) = runtime.shutdown_timeout(bound.saturating_sub(began.elapsed())) {
                eprintln!("closing native runtime: {error:#}");
            }
        }
    }
}

impl EngineDispatcher {
    /// Returns None for routes owned by other client features.
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        if !matches!(
            request.path.as_str(),
            "app/version"
                | "app/shutdown"
                | "app/setPreferences"
                | "app/queue"
                | "app/setQueue"
                | "transfer/info"
                | "torrents/info"
                | "torrents/add"
                | "torrents/start"
                | "torrents/stop"
                | "torrents/delete"
                | "torrents/files"
                | "torrents/filePrio"
                | "torrents/setUploadLimit"
                | "torrents/setShareLimits"
                | "torrents/addTags"
                | "torrents/removeTags"
                | "torrents/toggleSequentialDownload"
                | "torrents/toggleFirstLastPiecePrio"
        ) {
            return None;
        }
        if self.inner.stopping.load(Ordering::Acquire) {
            return Some(Err(anyhow!("native engine is closing")));
        }
        let cx = self.runtime.cx();
        Some(self.runtime.block_on(async {
            rqbit_runtime::select! {
                result = self.dispatch_async(cx, request) => result,
                _ = self.inner.session.cancellation_token().cancelled() => Err(anyhow!("native engine is closing")),
            }
        }))
    }

    async fn dispatch_async(&self, cx: &Cx, request: &Request) -> Result<String> {
        match request.path.as_str() {
            "app/version" => return Ok(format!("native librqbit {}", librqbit::version())),
            "app/shutdown" => { QUIT_REQUESTED.store(true, Ordering::Release); return Ok("Ok.".into()); },
            "app/queue" => {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                let mut active = 0;
                let mut queued = 0;
                for record in &ledger.records {
                    if let Ok(handle) = self.inner.handle(&record.hash) {
                        let stats = handle.stats();
                        if !stats.finished && !matches!(stats.state, TorrentStatsState::Error) {
                            if !handle.is_paused() { active += 1; }
                            else if !record.paused { queued += 1; }
                        }
                    }
                }
                return json(&serde_json::json!({
                    "max_active_downloads": self.inner.max_active_downloads.load(Ordering::Acquire),
                    "active_downloads": active,
                    "queued_downloads": queued,
                    "network_blocked": self.inner.network_blocked.load(Ordering::Acquire),
                }));
            },
            "app/setQueue" => {
                let limit = required(request, "max_active_downloads")?.parse()?;
                self.inner.set_download_limit(cx, limit).await?;
                return Ok("Ok.".into());
            },
            "app/setPreferences" => bail!("qBittorrent preferences do not apply to the native engine; edit fbittorrent configuration"),
            "transfer/info" => {
                let stats = self.inner.session.stats_snapshot();
                return json(&TransferInfo { dl_info_speed: stats.download_speed.as_bytes() as i64,
                    up_info_speed: stats.upload_speed.as_bytes() as i64,
                    dl_info_data: stats.counters.fetched_bytes as i64,
                    up_info_data: stats.counters.uploaded_bytes as i64,
                    connection_status: "connected".into() });
            },
            "torrents/info" => {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                let mut torrents = Vec::new();
                for record in &ledger.records {
                    if let Ok(handle) = self.inner.handle(&record.hash) { torrents.push(snapshot(record, &handle)); }
                }
                torrents.sort_by(|a, b| b.added_on.cmp(&a.added_on));
                return json(&torrents);
            },
            "torrents/files" => {
                let handle = self.inner.handle(required(request, "hash")?)?;
                let metadata = handle.metadata.load_full().context("torrent metadata is still loading")?;
                let stats = handle.stats();
                let selected = handle.only_files();
                let files: Vec<TorrentFile> = metadata.file_infos.iter().enumerate().map(|(index, file)| {
                    let progress = stats.file_progress.get(index).copied().unwrap_or(0);
                    TorrentFile { index: index as i64, name: file.relative_filename.to_string_lossy().into_owned(),
                        size: file.len as i64, progress: if file.len == 0 { 1.0 } else { progress as f64 / file.len as f64 },
                        progress_known: true,
                        priority: if selected.as_ref().map(|ids| ids.contains(&index)).unwrap_or(true) { 1 } else { 0 } }
                }).collect();
                return json(&files);
            },
            "torrents/add" => return self.add(cx, request).await,
            "torrents/toggleSequentialDownload" => bail!("the native engine always downloads each file sequentially; this setting cannot be disabled"),
            "torrents/toggleFirstLastPiecePrio" => bail!("the native engine always prioritizes the first and last piece of each file; this setting cannot be disabled"),
            _ => {},
        }
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        if request.path == "torrents/filePrio" {
            let hash = required(request, "hash")?;
            let priority: i64 = required(request, "priority")?.parse()?;
            if !matches!(priority, 0 | 1) {
                bail!("the native engine supports skip (0) and normal (1) file priorities; high and maximum are unavailable");
            }
            let handle = self.inner.handle(hash)?;
            let metadata = handle
                .metadata
                .load_full()
                .context("torrent metadata is still loading")?;
            let mut selected: HashSet<usize> = handle
                .only_files()
                .unwrap_or_else(|| (0..metadata.file_infos.len()).collect())
                .into_iter()
                .collect();
            for value in required(request, "id")?.split('|') {
                let index: usize = value.parse()?;
                if index >= metadata.file_infos.len() {
                    bail!("file index {index} is out of range");
                }
                if priority == 0 {
                    selected.remove(&index);
                } else {
                    selected.insert(index);
                }
            }
            // A finished seeder can become an unfinished download when another
            // file is selected. Stop its run before changing the selected set.
            if !handle.is_paused() && !matches!(handle.stats().state, TorrentStatsState::Error) {
                self.inner.session.pause(cx, &handle).await?;
            }
            self.inner
                .session
                .update_only_files(cx, &handle, &selected)
                .await?;
            let record = ledger
                .records
                .iter_mut()
                .find(|record| record.hash == hash)
                .context("torrent record not found")?;
            let mut selected: Vec<_> = selected.into_iter().collect();
            selected.sort_unstable();
            record.only_files = Some(selected);
            record.completion_on = 0;
            self.inner.save(cx, &ledger).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok("Ok.".into());
        }
        let hashes = requested_hashes(request, &ledger)?;
        // Validate the complete selection before any mutation.
        for hash in &hashes {
            self.inner.handle(hash)?;
        }
        if request.path == "torrents/delete" {
            let delete_files = request.arg("deleteFiles") == Some("true");
            // Persist the removal before closing handles or deleting payload:
            // a write failure must leave the existing download recoverable.
            let retained = ledger
                .records
                .iter()
                .filter(|record| !hashes.contains(&record.hash))
                .cloned()
                .collect();
            let replacement = Ledger {
                migrated_legacy: ledger.migrated_legacy,
                records: retained,
            };
            self.inner.save(cx, &replacement).await?;
            ledger.records = replacement.records;
            for hash in hashes {
                let handle = self.inner.handle(&hash)?;
                self.inner
                    .session
                    .delete(cx, TorrentIdOrHash::Id(handle.id()), delete_files)
                    .await?;
                let _ = rqbit_runtime::fs::remove_file(self.inner.torrent_path(&hash)?).await;
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok("Ok.".into());
        }
        let retry_errors = if request.path == "torrents/start" {
            hashes.iter().cloned().collect()
        } else {
            HashSet::new()
        };
        for hash in hashes {
            let index = ledger
                .records
                .iter()
                .position(|record| record.hash == hash)
                .context("torrent record not found")?;
            let old = ledger.records[index].clone();
            let mut record = old.clone();
            let handle = self.inner.handle(&hash)?;
            match request.path.as_str() {
                "torrents/start" => {
                    record.paused = false;
                    record.resume_after_migration = false;
                    if matches!(handle.stats().state, TorrentStatsState::Error) {
                        // Recheck a manually retried error while paused. Its
                        // future queue admission then survives a full queue.
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                "torrents/stop" => {
                    record.paused = true;
                    record.resume_after_migration = false;
                }
                "torrents/setUploadLimit" => {
                    let limit: i64 = required(request, "limit")?.parse()?;
                    if limit < 0 {
                        bail!("upload limit must be zero (unlimited) or a positive byte rate");
                    }
                    record.upload_limit = limit;
                    record.options(record.paused)?;
                    if record.upload_limit != old.upload_limit {
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                "torrents/setShareLimits" => {
                    let ratio: f64 = required(request, "ratioLimit")?.parse()?;
                    let action = request.arg("shareLimitAction").unwrap_or("Default");
                    if ratio != -2.0 && (ratio != 0.0 || !action.eq_ignore_ascii_case("stop")) {
                        bail!("native share limits support unlimited (-2) or stop on completion (ratio 0, Stop)");
                    }
                    record.ratio_limit = ratio;
                    if ratio == 0.0 && handle.stats().finished && !handle.is_paused() {
                        self.inner.session.pause(cx, &handle).await?;
                    }
                }
                "torrents/addTags" | "torrents/removeTags" => {
                    let mut tags: HashSet<String> = record
                        .tags
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_owned)
                        .collect();
                    for tag in required(request, "tags")?
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                    {
                        if request.path == "torrents/addTags" {
                            tags.insert(tag.into());
                        } else {
                            tags.remove(tag);
                        }
                    }
                    let mut tags: Vec<_> = tags.into_iter().collect();
                    tags.sort();
                    record.tags = tags.join(",");
                }
                _ => bail!("unsupported native torrent operation {}", request.path),
            }
            ledger.records[index] = record;
            if let Err(error) = self.inner.save(cx, &ledger).await {
                ledger.records[index] = old;
                let _ = self.inner.save(cx, &ledger).await;
                return Err(error).context("saving torrent intent; previous intent restored");
            }
        }
        self.inner
            .reconcile_queue(cx, &mut ledger, &retry_errors)
            .await?;
        Ok("Ok.".into())
    }

    async fn add(&self, cx: &Cx, request: &Request) -> Result<String> {
        // Serialize additions without holding the ledger while resolving magnets.
        // A duplicate add must observe the first add's durable record.
        let _addition = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
        if self.inner.network_blocked.load(Ordering::Acquire) {
            bail!("connect ProtonVPN before adding a download");
        }
        let upload_limit = request
            .arg("upLimit")
            .map(str::parse::<i64>)
            .transpose()?
            .unwrap_or_else(|| {
                if request
                    .arg("tags")
                    .unwrap_or("")
                    .split(',')
                    .any(|tag| tag.trim() == SEED_TAG)
                {
                    0
                } else {
                    NO_UPLOAD_LIMIT
                }
            });
        if upload_limit < 0 {
            bail!("upload limit cannot be negative");
        }
        let paused =
            request.arg("stopped") == Some("true") || request.arg("paused") == Some("true");
        let mut record = Record {
            hash: String::new(),
            name: String::new(),
            source: String::new(),
            output_folder: request
                .arg("savepath")
                .map(str::to_owned)
                .unwrap_or_else(|| self.inner.default_folder.to_string_lossy().into_owned()),
            only_files: None,
            upload_limit,
            ratio_limit: request
                .arg("ratioLimit")
                .map(str::parse::<f64>)
                .transpose()?
                .unwrap_or(if upload_limit == NO_UPLOAD_LIMIT {
                    0.0
                } else {
                    -2.0
                }),
            category: request.arg("category").unwrap_or("").into(),
            tags: request.arg("tags").unwrap_or("").into(),
            added_on: now(),
            completion_on: 0,
            paused,
            resume_after_migration: false,
        };
        if !matches!(record.ratio_limit, -2.0 | 0.0) {
            bail!("native share limits support unlimited (-2) or stop on completion (ratio 0)");
        }
        let sources: Vec<String> = if let Some(file) = &request.file {
            record.name = file.name.clone();
            vec![String::new()]
        } else {
            required(request, "urls")?
                .lines()
                .filter(|source| !source.trim().is_empty())
                .map(|source| source.trim().to_owned())
                .collect()
        };
        if sources.is_empty() {
            bail!("no torrent was supplied");
        }
        for source in sources {
            record.source = source.clone();
            let add = if let Some(file) = &request.file {
                AddTorrent::from_bytes(file.bytes.clone())
            } else if Path::new(&source).is_file() {
                AddTorrent::from_local_filename(&source)?
            } else {
                AddTorrent::from_url(source)
            };
            // Persist metadata and policy before allowing payload exchange.
            let mut options = record.options(true)?;
            // Let rqbit create a named subdirectory for new multi-file torrents.
            if request.arg("savepath").is_none() {
                options.output_folder = None;
            }
            let response = time::timeout(
                cx,
                Duration::from_secs(90),
                self.inner.session.add_torrent(cx, add, Some(options)),
            )
            .await
            .context("torrent metadata lookup timed out")??;
            let existed = matches!(&response, AddTorrentResponse::AlreadyManaged(_, _));
            let handle = response.into_handle().context("torrent was only listed")?;
            if existed {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                if !ledger
                    .records
                    .iter()
                    .any(|saved| saved.hash == handle.info_hash().as_string())
                {
                    bail!("torrent is being restored; retry the add shortly");
                }
                // Also completes a prior paused add whose record fsync failed.
                self.inner.save(cx, &ledger).await?;
                continue;
            }
            record.hash = handle.info_hash().as_string();
            record.name = handle.name().unwrap_or_else(|| record.hash.clone());
            record.output_folder = handle.output_folder().to_string_lossy().into_owned();
            if let Err(error) = self.inner.cache_metadata(cx, &handle).await {
                let _ = self
                    .inner
                    .session
                    .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
                    .await;
                return Err(error)
                    .context("saving torrent metadata; torrent stopped without deleting files");
            }
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            // The first durable record is paused. A failed first save must never
            // leave an acknowledged-running record for the next app opening.
            record.paused = true;
            ledger.records.push(record.clone());
            if let Err(error) = self.inner.save(cx, &ledger).await {
                // Retain the paused record in memory: rename may have completed
                // before an fsync failed, so deleting it could contradict disk.
                return Err(error).context("saving torrent records; torrent remains paused");
            }
            if !paused {
                let saved = ledger
                    .records
                    .iter_mut()
                    .find(|saved| saved.hash == record.hash)
                    .unwrap();
                saved.paused = false;
                if let Err(error) = self.inner.save(cx, &ledger).await {
                    ledger
                        .records
                        .iter_mut()
                        .find(|saved| saved.hash == record.hash)
                        .unwrap()
                        .paused = true;
                    let _ = self.inner.save(cx, &ledger).await;
                    return Err(error).context("saving requested torrent; torrent remains paused");
                }
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
        }
        Ok("Ok.".into())
    }

    pub fn import_legacy(
        &self,
        torrents: Vec<Torrent>,
        files: HashMap<String, Vec<TorrentFile>>,
        backup: PathBuf,
    ) -> Result<usize> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            let _addition = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            if ledger.migrated_legacy {
                return Ok(0);
            }
            if !ledger.records.is_empty() {
                bail!("native torrents already exist; legacy migration requires an empty native library");
            }
            let mut staged = Vec::new();
            for torrent in torrents {
                if torrent.hash.len() != 40 || !torrent.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    bail!("legacy torrent {} has an unsupported hash; native migration requires v1 torrents", torrent.name);
                }
                let seed_allowed = torrent.seed_allowed();
                let was_running = !torrent.is_stopped();
                let path = backup.join(format!("{}.torrent", torrent.hash));
                let bytes = rqbit_runtime::fs::read(&path)
                    .await
                    .with_context(|| format!("reading old torrent metadata {}", path.display()))?;
                let metadata = librqbit::torrent_from_bytes(&bytes)
                    .with_context(|| format!("reading metainfo for {}", torrent.name))?;
                if metadata.info_hash.as_string() != torrent.hash {
                    bail!("legacy metadata hash does not match {}", torrent.name);
                }
                let info = metadata.info.data.validate()?;
                let list = files.get(&torrent.hash).context("legacy torrent file list is missing")?;
                let selected = Some(list.iter().filter(|file| file.priority != 0).map(|file| file.index as usize).collect());
                let content = PathBuf::from(&torrent.content_path);
                let multi = info.info().files.is_some();
                let output_folder = if !torrent.content_path.is_empty() {
                    if multi {
                        content
                    } else {
                        content
                            .parent()
                            .unwrap_or(Path::new(&torrent.save_path))
                            .to_path_buf()
                    }
                } else {
                    PathBuf::from(&torrent.save_path)
                };
                let details: Vec<_> = info.iter_file_details().collect();
                if details.len() != list.len() {
                    bail!("legacy file list does not match metainfo for {}", torrent.name);
                }
                let mut indices = HashSet::new();
                for file in list {
                    let index = usize::try_from(file.index).context("legacy file index is negative")?;
                    let detail = details.get(index).context("legacy file index is out of range")?;
                    if !indices.insert(index) || file.size < 0 || file.size as u64 != detail.len {
                        bail!("legacy file sizes or indices do not match metainfo for {}", torrent.name);
                    }
                    let native_path = output_folder.join(detail.filename.to_pathbuf());
                    let legacy_path = PathBuf::from(&torrent.save_path).join(&file.name);
                    if native_path != legacy_path {
                        bail!("legacy file {} has a renamed or unsupported layout; native import would use {}; keep the legacy engine for this library", legacy_path.display(), native_path.display());
                    }
                    if file.progress > 0.0 && !native_path.is_file() {
                        bail!("legacy downloaded file {} is missing or uses an incomplete-file suffix; native migration cannot preserve its bytes", native_path.display());
                    }
                }
                let record = Record {
                    hash: torrent.hash.clone(),
                    name: torrent.name,
                    source: String::new(),
                    output_folder: output_folder.to_string_lossy().into_owned(),
                    only_files: selected,
                    upload_limit: if seed_allowed {
                        torrent.up_limit.max(0)
                    } else {
                        NO_UPLOAD_LIMIT
                    },
                    ratio_limit: if seed_allowed { -2.0 } else { 0.0 },
                    category: torrent.category,
                    tags: torrent.tags,
                    added_on: torrent.added_on,
                    completion_on: torrent.completion_on,
                    paused: true,
                    resume_after_migration: was_running,
                };
                record.options(true)?;
                staged.push((record, bytes));
            }
            let mut restored = Vec::new();
            for (record, bytes) in &staged {
                self.inner.write_metadata(cx, &record.hash, bytes).await?;
                match self.inner.restore_record(cx, record, true).await {
                    Ok(handle) => restored.push(handle),
                    Err(error) => {
                        for handle in restored {
                            let _ = self.inner.session.delete(cx, TorrentIdOrHash::Id(handle.id()), false).await;
                        }
                        return Err(error).context("importing old downloads; legacy records remain available for retry");
                    }
                }
            }
            let count = staged.len();
            ledger.records.extend(staged.into_iter().map(|(record, _)| record));
            ledger.migrated_legacy = true;
            self.inner.save(cx, &ledger).await?;
            Ok(count)
        })
    }

    /// True once legacy torrent records have been imported durably.
    /// Source service retirement and resuming imported torrents are separate steps.
    pub fn legacy_migration_completed(&self) -> Result<bool> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            Ok(OwnedMutexGuard::lock(self.inner.ledger.clone(), cx)
                .await?
                .migrated_legacy)
        })
    }

    pub fn resume_imported(&self) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            for record in &mut ledger.records {
                if record.resume_after_migration {
                    record.paused = false;
                    record.resume_after_migration = false;
                }
            }
            self.inner.save(cx, &ledger).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await
        })
    }

    /// Persist and apply a live download queue limit through the app runtime.
    #[cfg(test)]
    pub fn set_download_limit(&self, limit: usize) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        self.runtime
            .block_on(self.inner.set_download_limit(self.runtime.cx(), limit))
    }

    /// Suspend or admit network work without changing durable torrent intent.
    #[cfg(test)]
    pub fn set_network_blocked(&self, blocked: bool) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        self.runtime.block_on(async {
            let mut ledger =
                OwnedMutexGuard::lock(self.inner.ledger.clone(), self.runtime.cx()).await?;
            self.inner.network_blocked.store(blocked, Ordering::Release);
            self.inner
                .reconcile_queue(self.runtime.cx(), &mut ledger, &HashSet::new())
                .await
        })
    }
}

// A parked application has no native session or network work. Its IPC workers
// still serialize saved intent so simultaneous widget/CLI actions cannot lose
// another action's ledger update.
static PARKED_LEDGER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn save_parked_ledger(directory: &Path, ledger: &Ledger) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join("torrents.json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(ledger)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, directory.join("torrents.json"))?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn parked_metadata(record: &Record, directory: &Path) -> Result<Option<Vec<u8>>> {
    let path = directory.join(format!("{}.torrent", record.hash));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let metadata = librqbit::torrent_from_bytes(&bytes)?;
    if metadata.info_hash.as_string() != record.hash {
        bail!("saved torrent metadata does not match {}", record.name);
    }
    metadata.info.data.validate()?;
    Ok(Some(bytes))
}

fn parked_snapshot(record: &Record, directory: &Path) -> Result<serde_json::Value> {
    let mut torrent = Torrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        state: if record.paused {
            if record.completion_on > 0 {
                "stoppedUP"
            } else {
                "stoppedDL"
            }
        } else {
            "waitingVPN"
        }
        .into(),
        save_path: record.output_folder.clone(),
        content_path: record.output_folder.clone(),
        category: record.category.clone(),
        added_on: record.added_on,
        completion_on: record.completion_on,
        up_limit: record.upload_limit,
        ratio_limit: record.ratio_limit,
        tags: record.tags.clone(),
        seq_dl: true,
        f_l_piece_prio: true,
        eta: 8640000,
        ..Default::default()
    };
    if let Some(bytes) = parked_metadata(record, directory)? {
        let metadata = librqbit::torrent_from_bytes(&bytes)?;
        let info = metadata.info.data.validate()?;
        torrent.size = info.lengths().total_length() as i64;
        let files: Vec<_> = info.iter_file_details().collect();
        if files.len() == 1 {
            torrent.content_path = Path::new(&record.output_folder)
                .join(files[0].filename.to_pathbuf())
                .to_string_lossy()
                .into_owned();
        }
    }
    let mut value = serde_json::to_value(torrent)?;
    value["progress_known"] = serde_json::Value::Bool(false);
    Ok(value)
}

fn dispatch_parked_in(request: &Request, directory: &Path) -> Option<Result<String>> {
    if !matches!(
        request.path.as_str(),
        "app/version"
            | "app/shutdown"
            | "app/queue"
            | "app/setQueue"
            | "transfer/info"
            | "torrents/info"
            | "torrents/files"
            | "torrents/start"
            | "torrents/stop"
            | "torrents/delete"
    ) {
        return if request.path.starts_with("torrents/") || request.path == "app/setPreferences" {
            Some(Err(anyhow!("connect ProtonVPN before changing downloads; saved downloads and settings remain available")))
        } else {
            None
        };
    }
    Some((|| {
        let _guard = PARKED_LEDGER_LOCK
            .lock()
            .map_err(|_| anyhow!("saved torrent lock is unavailable"))?;
        match request.path.as_str() {
            "app/version" => {
                return Ok(format!(
                    "native librqbit {} (waiting for ProtonVPN)",
                    librqbit::version()
                ))
            }
            "app/shutdown" => {
                QUIT_REQUESTED.store(true, Ordering::Release);
                return Ok("Ok.".into());
            }
            "transfer/info" => {
                return json(&TransferInfo {
                    connection_status: "vpn_waiting".into(),
                    ..Default::default()
                })
            }
            "app/setQueue" => {
                let limit: usize = required(request, "max_active_downloads")?.parse()?;
                if limit == 0 {
                    bail!("max_active_downloads must be at least 1");
                }
                let mut cfg = Config::load()?;
                cfg.max_active_downloads = limit;
                cfg.save()?;
                return Ok("Ok.".into());
            }
            _ => {}
        }
        let mut ledger = read_ledger(&directory.join("torrents.json"))?;
        match request.path.as_str() {
            "app/queue" => {
                return json(&serde_json::json!({
                    "max_active_downloads": Config::load()?.max_active_downloads,
                    "active_downloads": 0,
                    "queued_downloads": ledger.records.iter().filter(|record| !record.paused && !(record.stop_on_complete() && record.completion_on > 0)).count(),
                    "network_blocked": true,
                }))
            }
            "torrents/info" => {
                ledger.records.sort_by(|a, b| b.added_on.cmp(&a.added_on));
                let torrents: Vec<_> = ledger
                    .records
                    .iter()
                    .map(|record| parked_snapshot(record, directory))
                    .collect::<Result<_>>()?;
                return json(&torrents);
            }
            "torrents/files" => {
                let hash = required(request, "hash")?;
                let record = ledger
                    .records
                    .iter()
                    .find(|record| record.hash == hash)
                    .context("torrent record not found")?;
                let bytes = parked_metadata(record, directory)?.context(
                    "saved torrent metadata is unavailable; connect ProtonVPN to resolve it",
                )?;
                let metadata = librqbit::torrent_from_bytes(&bytes)?;
                let info = metadata.info.data.validate()?;
                let files: Vec<_> = info.iter_file_details().enumerate().map(|(index, file)| serde_json::json!({
                    "index": index,
                    "name": file.filename.to_pathbuf().to_string_lossy(),
                    "size": file.len,
                    "progress": 0.0,
                    "progress_known": false,
                    "priority": if record.only_files.as_ref().map(|ids| ids.contains(&index)).unwrap_or(true) { 1 } else { 0 },
                })).collect();
                return json(&files);
            }
            _ => {}
        }
        let hashes = requested_hashes(request, &ledger)?;
        if request.path == "torrents/delete" {
            if request.arg("deleteFiles") == Some("true") {
                bail!("connect ProtonVPN before removing downloaded files; removing the saved torrent while keeping files is available now");
            }
            ledger
                .records
                .retain(|record| !hashes.contains(&record.hash));
            save_parked_ledger(directory, &ledger)?;
            for hash in hashes {
                let _ = std::fs::remove_file(directory.join(format!("{hash}.torrent")));
            }
        } else {
            for record in &mut ledger.records {
                if hashes.contains(&record.hash) {
                    record.paused = request.path == "torrents/stop";
                    record.resume_after_migration = false;
                }
            }
            save_parked_ledger(directory, &ledger)?;
        }
        Ok("Ok.".into())
    })())
}

/// Serve saved history and durable intent while no VPN-bound session exists.
pub fn dispatch_parked(request: &Request) -> Option<Result<String>> {
    dispatch_parked_in(request, &crate::config::data_dir().join("engine"))
}

fn required<'a>(request: &'a Request, key: &str) -> Result<&'a str> {
    request
        .arg(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing {key}"))
}
fn requested_hashes(request: &Request, ledger: &Ledger) -> Result<Vec<String>> {
    let value = required(request, "hashes")?;
    if value == "all" {
        return Ok(ledger
            .records
            .iter()
            .map(|record| record.hash.clone())
            .collect());
    }
    let hashes: Vec<String> = value
        .split('|')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    for hash in &hashes {
        if !ledger.records.iter().any(|record| &record.hash == hash) {
            bail!("torrent not found: {hash}");
        }
    }
    Ok(hashes)
}
fn json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn snapshot(record: &Record, handle: &Arc<ManagedTorrent>) -> Torrent {
    let stats = handle.stats();
    let progress = if stats.total_bytes > 0 {
        (stats.progress_bytes as f64 / stats.total_bytes as f64).min(1.0)
    } else if stats.finished {
        1.0
    } else {
        0.0
    };
    let (dlspeed, upspeed, peers) = stats
        .live
        .as_ref()
        .map(|live| {
            (
                live.download_speed.as_bytes() as i64,
                live.upload_speed.as_bytes() as i64,
                live.snapshot.peer_stats.live as i64,
            )
        })
        .unwrap_or_default();
    let state = if handle.is_paused()
        && !record.paused
        && !stats.finished
        && !matches!(stats.state, TorrentStatsState::Error)
    {
        "queuedDL"
    } else {
        match stats.state {
            TorrentStatsState::Initializing { paused: true } => {
                if stats.finished {
                    "stoppedUP"
                } else {
                    "stoppedDL"
                }
            }
            TorrentStatsState::Initializing { paused: false } => "checkingDL",
            TorrentStatsState::Paused => {
                if stats.finished {
                    "stoppedUP"
                } else {
                    "stoppedDL"
                }
            }
            TorrentStatsState::Live => {
                if stats.finished {
                    "uploading"
                } else if dlspeed > 0 {
                    "downloading"
                } else {
                    "stalledDL"
                }
            }
            TorrentStatsState::Error => "error",
        }
    };
    let content_path = handle
        .metadata
        .load_full()
        .map(|metadata| {
            if metadata.file_infos.len() == 1 {
                handle
                    .output_folder()
                    .join(&metadata.file_infos[0].relative_filename)
            } else {
                handle.output_folder().to_path_buf()
            }
        })
        .unwrap_or_else(|| PathBuf::from(&record.output_folder));
    Torrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        state: state.into(),
        progress,
        progress_known: true,
        size: stats.total_bytes as i64,
        completed: stats.progress_bytes as i64,
        dlspeed,
        upspeed,
        eta: if dlspeed > 0 {
            (stats.total_bytes.saturating_sub(stats.progress_bytes) / dlspeed as u64) as i64
        } else {
            8640000
        },
        // rqbit does not expose the swarm's seed/leecher split; peer count is available.
        num_seeds: 0,
        num_leechs: peers,
        ratio: if stats.progress_bytes > 0 {
            stats.uploaded_bytes as f64 / stats.progress_bytes as f64
        } else {
            0.0
        },
        seq_dl: true,
        f_l_piece_prio: true,
        save_path: record.output_folder.clone(),
        content_path: content_path.to_string_lossy().into_owned(),
        category: record.category.clone(),
        added_on: record.added_on,
        completion_on: record.completion_on,
        up_limit: record.upload_limit,
        ratio_limit: record.ratio_limit,
        tags: record.tags.clone(),
    }
}

#[cfg(test)]
mod tests {
    fn test_cx() -> Cx {
        Cx::current().expect("native test runtime context")
    }

    use super::*;

    fn engine_test_diagnostic(action: &str, expect: &str, observed: serde_json::Value, ok: bool) {
        // The P0-17 runner adds the E.1 UTC timestamp, case and step and retains
        // this structured observation alongside the unmodified test output.
        eprintln!(
            "FBT-P017-STEP {}",
            serde_json::json!({
                "action": action, "expect": expect,
                "observed": observed.to_string(), "ok": ok,
            })
        );
    }

    fn test_app(
        directory: &Path,
        downloads: &Path,
        ledger: Ledger,
        port: Option<u16>,
    ) -> AppEngine {
        let instance =
            InstanceLock::acquire_in(directory.join("runtime")).unwrap_or_else(|error| {
                use std::os::unix::fs::MetadataExt;
                let path = directory.join("runtime/engine.lock");
                let inode = std::fs::metadata(&path).map(|metadata| metadata.ino()).ok();
                let locks = std::fs::read_to_string("/proc/locks").ok().map(|locks| {
                    locks
                        .lines()
                        .filter(|line| {
                            inode.is_some_and(|inode| line.contains(&format!(":{inode} ")))
                        })
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                });
                engine_test_diagnostic(
                    "acquire isolated engine instance lock",
                    "The lock for this test's runtime directory is available.",
                    serde_json::json!({
                        "path": path, "inode": inode, "pid": std::process::id(),
                        "kernel_locks": locks, "error": format!("{error:#}"),
                    }),
                    false,
                );
                panic!("acquiring {}: {error:#}", path.display());
            });
        std::fs::create_dir_all(directory).unwrap();
        std::fs::create_dir_all(downloads).unwrap();
        let runtime = NativeRuntime::new().unwrap();
        let session = runtime
            .block_on(Session::new_with_opts(
                runtime.cx(),
                downloads.to_path_buf(),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    listen: port.map(|port| ListenerOptions {
                        mode: ListenerMode::TcpOnly,
                        listen_addr: ([127, 0, 0, 1], port).into(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ))
            .unwrap();
        let inner = Arc::new(Inner {
            session,
            ledger: Arc::new(Mutex::new(ledger)),
            additions: Arc::new(Mutex::new(())),
            directory: directory.to_path_buf(),
            default_folder: downloads.to_path_buf(),
            stopping: AtomicBool::new(false),
            max_active_downloads: AtomicUsize::new(3),
            network_blocked: AtomicBool::new(false),
        });
        runtime.block_on(async {
            let records = OwnedMutexGuard::lock(inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records
                .clone();
            for record in records {
                inner
                    .restore_record(&test_cx(), &record, true)
                    .await
                    .unwrap();
            }
            inner.maintain(&test_cx()).await.unwrap();
        });
        let dispatcher = EngineDispatcher {
            inner,
            runtime: runtime.handle(),
        };
        AppEngine {
            dispatcher,
            runtime: Some(runtime),
            server: None,
            instance,
        }
    }

    fn request(path: &str, args: &[(&str, &str)]) -> Request {
        Request {
            method: "POST".into(),
            path: path.into(),
            args: args
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
            file: None,
        }
    }

    fn review_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fbittorrent-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn instance_lock_excludes_the_same_directory_but_allows_another() {
        let directory = review_directory("instance-lock-directories");
        let first = InstanceLock::acquire_in(directory.join("first")).unwrap();
        let same = InstanceLock::acquire_in(directory.join("first"));
        let other = InstanceLock::acquire_in(directory.join("other"));
        engine_test_diagnostic(
            "check instance lock directory scope",
            "The same directory is excluded while another directory remains available.",
            serde_json::json!({"same_directory_excluded": same.is_err(),
                "other_directory_available": other.is_ok()}),
            same.is_err() && other.is_ok(),
        );
        assert!(same.is_err());
        assert!(other.is_ok());
        drop(first);
        drop(other);
        let reopened = InstanceLock::acquire_in(directory.join("first")).unwrap();
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dropping_instance_lock_releases_a_fork_inherited_descriptor() {
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let directory = review_directory("instance-lock-inheritance");
        let instance = InstanceLock::acquire_in(directory.clone()).unwrap();
        let (mut parent, child) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let parent_fd = parent.as_raw_fd();
        let child_fd = child.as_raw_fd();
        // Model a parallel provider spawn parked between fork and exec. In the
        // child, use only async-signal-safe syscalls and _exit: other libtest
        // threads may have held allocator or runtime locks when fork happened.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            unsafe {
                libc::close(parent_fd);
                let mut byte = 1u8;
                if libc::write(child_fd, (&byte as *const u8).cast(), 1) != 1 {
                    libc::_exit(1);
                }
                let read = libc::read(child_fd, (&mut byte as *mut u8).cast(), 1);
                libc::_exit(if read == 1 { 0 } else { 1 });
            }
        }
        drop(child);
        let mut ready = [0u8];
        parent.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
        drop(instance);
        let reopened = InstanceLock::acquire_in(directory.clone());
        // Reap the child before asserting, including on the pre-fix failure.
        parent.write_all(&[1]).unwrap();
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        engine_test_diagnostic(
            "release instance lock while a forked child retains its descriptor",
            "Dropping the owner permits immediate reacquisition before the child exits.",
            serde_json::json!({"reacquired": reopened.is_ok(),
                "error": reopened.as_ref().err().map(|error| format!("{error:#}")),
                "child_pid": pid, "child_waited": waited, "child_status": status}),
            reopened.is_ok() && waited == pid && status == 0,
        );
        assert_eq!(waited, pid);
        assert_eq!(status, 0);
        assert!(
            reopened.is_ok(),
            "owner dropped but inherited lock remained: {:?}",
            reopened.err()
        );
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn review_record(hash: &str, downloads: &Path) -> Record {
        Record {
            hash: hash.into(),
            name: "saved fixture".into(),
            source: String::new(),
            output_folder: downloads.to_string_lossy().into_owned(),
            only_files: None,
            upload_limit: NO_UPLOAD_LIMIT,
            ratio_limit: 0.0,
            category: String::new(),
            tags: String::new(),
            added_on: 1,
            completion_on: 0,
            paused: false,
            resume_after_migration: false,
        }
    }

    fn add_queue_fixture(dispatcher: &EngineDispatcher, index: usize, seed: bool) -> String {
        let name = format!("queue-{index}.txt");
        if seed {
            std::fs::write(dispatcher.inner.default_folder.join(&name), b"hello").unwrap();
        }
        let mut bytes = format!(
            "d4:infod6:lengthi5e4:name{}:{}12:piece lengthi16384e6:pieces20:",
            name.len(),
            name
        )
        .into_bytes();
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"ee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        dispatcher
            .dispatch(&Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: if seed {
                    vec![
                        ("ratioLimit".into(), "-2".into()),
                        ("upLimit".into(), "0".into()),
                    ]
                } else {
                    vec![]
                },
                file: Some(crate::ipc::Upload {
                    name: format!("queue-{index}.torrent"),
                    bytes,
                }),
            })
            .unwrap()
            .unwrap();
        dispatcher.runtime.block_on(async {
            let handle = dispatcher.inner.handle(&hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        hash
    }

    fn active_queue(dispatcher: &EngineDispatcher) -> Vec<String> {
        dispatcher.runtime.block_on(async {
            let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap();
            ledger
                .records
                .iter()
                .filter_map(|record| {
                    let handle = dispatcher.inner.handle(&record.hash).unwrap();
                    (!handle.is_paused() && !handle.stats().finished).then(|| record.hash.clone())
                })
                .collect()
        })
    }

    #[test]
    fn queue_caps_downloads_preserves_fifo_pause_delete_completion_and_restart() {
        let directory = review_directory("queue-lifecycle");
        let downloads = directory.join("downloads");
        let app = test_app(&directory, &downloads, Ledger::default(), None);
        let dispatcher = app.dispatcher();
        let hashes: Vec<_> = (0..6)
            .map(|index| add_queue_fixture(&dispatcher, index, false))
            .collect();
        assert_eq!(active_queue(&dispatcher), hashes[..3]);
        let info: Vec<Torrent> = serde_json::from_str(
            &dispatcher
                .dispatch(&request("torrents/info", &[]))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            info.iter()
                .filter(|torrent| torrent.state == "queuedDL")
                .count(),
            3
        );
        let status: serde_json::Value = serde_json::from_str(
            &dispatcher
                .dispatch(&request("app/queue", &[]))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["active_downloads"], 3);
        assert_eq!(status["queued_downloads"], 3);
        assert!(dispatcher
            .dispatch(&request("app/setQueue", &[("max_active_downloads", "0")]))
            .unwrap()
            .is_err());
        assert_eq!(active_queue(&dispatcher), hashes[..3]);

        dispatcher
            .dispatch(&request("torrents/stop", &[("hashes", &hashes[0])]))
            .unwrap()
            .unwrap();
        assert_eq!(active_queue(&dispatcher), hashes[1..4]);
        dispatcher
            .dispatch(&request("torrents/start", &[("hashes", &hashes[0])]))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&dispatcher),
            hashes[..3],
            "resume keeps the original FIFO position"
        );
        dispatcher
            .dispatch(&request("torrents/delete", &[("hashes", &hashes[1])]))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&dispatcher),
            vec![hashes[0].clone(), hashes[2].clone(), hashes[3].clone()]
        );
        dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hashes[0]), ("id", "0"), ("priority", "0")],
            ))
            .unwrap()
            .unwrap();
        assert!(
            dispatcher
                .inner
                .handle(&hashes[0])
                .unwrap()
                .stats()
                .finished
        );
        assert!(dispatcher.inner.handle(&hashes[0]).unwrap().is_paused());
        assert_eq!(
            active_queue(&dispatcher),
            hashes[2..5],
            "selected-file completion must admit the next download"
        );
        dispatcher
            .dispatch(&request("torrents/stop", &[("hashes", &hashes[5])]))
            .unwrap()
            .unwrap();
        let saved = read_ledger(&directory.join("torrents.json")).unwrap();
        assert_eq!(
            saved
                .records
                .iter()
                .map(|record| &record.hash)
                .collect::<Vec<_>>(),
            vec![&hashes[0], &hashes[2], &hashes[3], &hashes[4], &hashes[5]]
        );
        assert!(
            !saved.records[0].paused,
            "automatic completion suspension must retain running intent"
        );
        assert!(
            saved.records.last().unwrap().paused,
            "manual pause must be durable"
        );
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        let app = test_app(&directory, &downloads, saved, None);
        app.dispatcher.runtime.block_on(async {
            for hash in [&hashes[0], &hashes[2], &hashes[3], &hashes[4], &hashes[5]] {
                let handle = app.dispatcher.inner.handle(hash).unwrap();
                time::timeout(
                    &test_cx(),
                    Duration::from_secs(5),
                    handle.wait_until_initialized(&test_cx()),
                )
                .await
                .unwrap()
                .unwrap();
            }
            app.dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&app.dispatcher), hashes[2..5]);
        assert!(app.dispatcher.inner.handle(&hashes[5]).unwrap().is_paused());
        app.dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hashes[0]), ("id", "0"), ("priority", "1")],
            ))
            .unwrap()
            .unwrap();
        assert_eq!(
            active_queue(&app.dispatcher),
            vec![hashes[0].clone(), hashes[2].clone(), hashes[3].clone()],
            "expanding completed selection must respect the cap"
        );
        drop(app);
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn queue_limit_changes_and_network_suspension_keep_intent_and_exclude_seeders() {
        let directory = review_directory("queue-limit");
        let downloads = directory.join("downloads");
        let app = test_app(&directory, &downloads, Ledger::default(), None);
        let dispatcher = app.dispatcher();
        let seeder = add_queue_fixture(&dispatcher, 10, true);
        let hashes: Vec<_> = (0..5)
            .map(|index| add_queue_fixture(&dispatcher, index, false))
            .collect();
        assert!(!dispatcher.inner.handle(&seeder).unwrap().is_paused());
        assert_eq!(active_queue(&dispatcher), hashes[..3]);
        dispatcher.runtime.block_on(async {
            dispatcher
                .inner
                .max_active_downloads
                .store(1, Ordering::Release);
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&dispatcher), hashes[..1]);
        assert!(
            !dispatcher.inner.handle(&seeder).unwrap().is_paused(),
            "finished seeders do not consume download slots"
        );
        let before = std::fs::read(directory.join("torrents.json")).unwrap();
        dispatcher.set_network_blocked(true).unwrap();
        assert!(active_queue(&dispatcher).is_empty());
        assert!(dispatcher.inner.handle(&seeder).unwrap().is_paused());
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            before,
            "network suspension must not rewrite intent"
        );
        dispatcher.set_network_blocked(false).unwrap();
        assert_eq!(active_queue(&dispatcher), hashes[..1]);
        dispatcher.runtime.block_on(async {
            dispatcher
                .inner
                .max_active_downloads
                .store(4, Ordering::Release);
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
        });
        assert_eq!(active_queue(&dispatcher), hashes[..4]);
        let closing = std::time::Instant::now();
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        assert!(closing.elapsed() < Duration::from_secs(4));
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parked_history_preserves_order_unknown_progress_and_durable_manual_intent() {
        let directory = review_directory("parked-queue");
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = b"d4:infod6:lengthi1e4:name11:offline.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let hash = librqbit::torrent_from_bytes(bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        let mut record = review_record(&hash, &directory.join("downloads"));
        record.completion_on = 10;
        let ledger = Ledger {
            migrated_legacy: false,
            records: vec![record],
        };
        save_parked_ledger(&directory, &ledger).unwrap();
        let info: serde_json::Value = serde_json::from_str(
            &dispatch_parked_in(&request("torrents/info", &[]), &directory)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(info[0]["state"], "waitingVPN");
        assert_eq!(info[0]["size"], 1);
        assert_eq!(info[0]["progress_known"], false);
        let files: serde_json::Value = serde_json::from_str(
            &dispatch_parked_in(&request("torrents/files", &[("hash", &hash)]), &directory)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(files[0]["name"], "offline.bin");
        assert_eq!(files[0]["progress_known"], false);
        dispatch_parked_in(&request("torrents/stop", &[("hashes", &hash)]), &directory)
            .unwrap()
            .unwrap();
        assert!(
            read_ledger(&directory.join("torrents.json"))
                .unwrap()
                .records[0]
                .paused
        );
        dispatch_parked_in(&request("torrents/start", &[("hashes", &hash)]), &directory)
            .unwrap()
            .unwrap();
        assert!(
            !read_ledger(&directory.join("torrents.json"))
                .unwrap()
                .records[0]
                .paused
        );
        assert!(dispatch_parked_in(
            &request(
                "torrents/add",
                &[(
                    "urls",
                    "magnet:?xt=urn:btih:0000000000000000000000000000000000000000"
                )]
            ),
            &directory
        )
        .unwrap()
        .is_err());
        assert!(dispatch_parked_in(
            &request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")]
            ),
            &directory
        )
        .unwrap()
        .is_err());
        let saved = std::fs::read(directory.join("torrents.json")).unwrap();
        std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
        assert!(
            dispatch_parked_in(&request("torrents/stop", &[("hashes", &hash)]), &directory)
                .unwrap()
                .is_err()
        );
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            saved
        );
        std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
        dispatch_parked_in(
            &request("torrents/delete", &[("hashes", &hash)]),
            &directory,
        )
        .unwrap()
        .unwrap();
        assert!(read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .is_empty());
        assert!(!directory.join(format!("{hash}.torrent")).exists());
        assert!(
            !directory.join("downloads").exists(),
            "offline mutations do not create a session or touch payload"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_saved_records_preserve_the_original_ledger() {
        let directory = review_directory("invalid-ledger");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("torrents.json");
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let record = review_record(hash, &directory.join("downloads"));
        let mut invalid_hash = record.clone();
        invalid_hash.hash = "../outside".into();
        let samples = [
            b"{\"records\":[".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&Ledger {
                migrated_legacy: false,
                records: vec![invalid_hash],
            })
            .unwrap(),
            serde_json::to_vec(&Ledger {
                migrated_legacy: false,
                records: vec![record.clone(), record.clone()],
            })
            .unwrap(),
        ];
        for bytes in samples {
            std::fs::write(&path, &bytes).unwrap();
            assert!(read_ledger(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&Ledger {
                migrated_legacy: false,
                records: vec![record],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read_ledger(&path).unwrap().records.len(), 1);
        assert!(!directory.join("torrents.json.tmp").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mismatched_saved_metadata_never_opens_download_files() {
        let directory = review_directory("metadata-admission");
        let store = directory.join("store");
        let downloads = directory.join("downloads");
        let app = test_app(&store, &downloads, Ledger::default(), None);
        let bytes = b"d4:infod6:lengthi1e4:name11:foreign.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let expected = "0000000000000000000000000000000000000000";
        assert_ne!(
            librqbit::torrent_from_bytes(bytes)
                .unwrap()
                .info_hash
                .as_string(),
            expected
        );
        std::fs::write(store.join(format!("{expected}.torrent")), bytes).unwrap();
        let record = review_record(expected, &downloads);
        app.dispatcher.runtime.block_on(async {
            let error = app
                .dispatcher
                .inner
                .restore_record(&test_cx(), &record, false)
                .await
                .err()
                .unwrap();
            assert!(error.to_string().contains("hash does not match"));
            assert_eq!(
                app.dispatcher
                    .inner
                    .session
                    .with_torrents(|torrents| torrents.count()),
                0
            );
            assert!(!downloads.join("foreign.bin").exists());
            let mut invalid = record;
            invalid.hash = "../outside".into();
            assert!(app
                .dispatcher
                .inner
                .restore_record(&test_cx(), &invalid, false)
                .await
                .is_err());
            assert_eq!(
                app.dispatcher
                    .inner
                    .session
                    .with_torrents(|torrents| torrents.count()),
                0
            );
        });
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restoring_downloads_preserves_upload_policy() {
        let record = Record {
            hash: "abc".into(),
            name: "fixture".into(),
            source: String::new(),
            output_folder: "/tmp/fixture".into(),
            only_files: Some(vec![1]),
            upload_limit: NO_UPLOAD_LIMIT,
            ratio_limit: 0.0,
            category: String::new(),
            tags: String::new(),
            added_on: 1,
            completion_on: 0,
            paused: false,
            resume_after_migration: false,
        };
        let stored = serde_json::to_vec(&record).unwrap();
        let restored: Record = serde_json::from_slice(&stored).unwrap();
        let options = restored.options(restored.paused).unwrap();
        assert_eq!(
            options.ratelimits.upload_bps.unwrap().get(),
            NO_UPLOAD_LIMIT as u32
        );
        assert_eq!(options.only_files, Some(vec![1]));
        assert!(!options.paused);
        assert!(restored.stop_on_complete());
    }

    #[test]
    fn native_lifecycle_hashes_existing_data_stops_restores_and_deletes_durably() {
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-native-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("fixture.txt"), b"hello").unwrap();
        // A real five-byte v1 torrent, whose only piece is SHA1("hello").
        let mut torrent =
            b"d4:infod6:lengthi5e4:name11:fixture.txt12:piece lengthi16384e6:pieces20:".to_vec();
        torrent.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        torrent.extend_from_slice(b"ee");
        let make_app = |ledger: Ledger| test_app(&directory, &downloads, ledger, None);
        let app = make_app(Ledger::default());
        let dispatcher = app.dispatcher();
        dispatcher
            .dispatch(&Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: vec![],
                file: Some(crate::ipc::Upload {
                    name: "fixture.torrent".into(),
                    bytes: torrent,
                }),
            })
            .unwrap()
            .unwrap();
        let (hash, saved) = dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                handle.stats().finished,
                "existing bytes must pass real torrent verification"
            );
            dispatcher.inner.maintain(&test_cx()).await.unwrap();
            assert!(
                handle.is_paused(),
                "manual download must stop at completion"
            );
            let duplicate = Request {
                method: "POST".into(),
                path: "torrents/add".into(),
                args: vec![(
                    "urls".into(),
                    format!("magnet:?xt=urn:btih:{}", record.hash),
                )],
                file: None,
            };
            time::timeout(
                &test_cx(),
                Duration::from_secs(2),
                dispatcher.add(&test_cx(), &duplicate),
            )
            .await
            .expect("a saved magnet must not need peer metadata lookup")
            .unwrap();
            let saved: Ledger = serde_json::from_slice(
                &rqbit_runtime::fs::read(directory.join("torrents.json"))
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                saved.records.len(),
                1,
                "duplicate magnet must keep one durable record"
            );
            assert!(
                !saved.records[0].paused,
                "completion suspension preserves running intent"
            );
            assert!(saved.records[0].completion_on > 0);
            (record.hash, saved)
        });
        let old_session = dispatcher.inner.session.clone();
        drop(dispatcher);
        drop(app);
        assert!(old_session.cancellation_token().is_cancelled());
        let app = make_app(saved);
        app.dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(app.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = app.dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            app.dispatcher.inner.maintain(&test_cx()).await.unwrap();
            assert_eq!(handle.info_hash().as_string(), hash);
            assert!(handle.is_paused());
            assert_eq!(
                record
                    .options(record.paused)
                    .unwrap()
                    .ratelimits
                    .upload_bps
                    .unwrap()
                    .get(),
                32 * 1024
            );
            assert_eq!(
                rqbit_runtime::fs::read(downloads.join("fixture.txt"))
                    .await
                    .unwrap(),
                b"hello"
            );
        });
        let ledger_before = std::fs::read(directory.join("torrents.json")).unwrap();
        std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
        assert!(app
            .dispatcher
            .dispatch(&request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")],
            ))
            .unwrap()
            .is_err());
        assert_eq!(
            std::fs::read(directory.join("torrents.json")).unwrap(),
            ledger_before
        );
        assert_eq!(
            std::fs::read(downloads.join("fixture.txt")).unwrap(),
            b"hello"
        );
        assert!(app.dispatcher.inner.handle(&hash).is_ok());
        std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
        app.dispatcher
            .dispatch(&request(
                "torrents/delete",
                &[("hashes", &hash), ("deleteFiles", "true")],
            ))
            .unwrap()
            .unwrap();
        assert!(!downloads.join("fixture.txt").exists());
        assert!(!app.dispatcher.inner.torrent_path(&hash).unwrap().exists());
        let deleted = read_ledger(&directory.join("torrents.json")).unwrap();
        assert!(deleted.records.is_empty());
        drop(app);
        let reopened = make_app(deleted);
        assert!(reopened.dispatcher.inner.handle(&hash).is_err());
        assert!(!downloads.join("fixture.txt").exists());
        drop(reopened);
        drop(old_session);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_import_preserves_files_and_refuses_renamed_layout_before_writes() {
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-migrate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = directory.join("store");
        let downloads = directory.join("downloads");
        let backup = directory.join("backup");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(downloads.join("original.txt"), b"hello").unwrap();
        let app = test_app(&store, &downloads, Ledger::default(), None);
        let bytes = app.dispatcher.runtime.block_on(async {
            librqbit::create_torrent(
                &test_cx(),
                &downloads.join("original.txt"),
                librqbit::CreateTorrentOptions::default(),
                &librqbit::spawn_utils::BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap()
        });
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(backup.join(format!("{hash}.torrent")), &bytes).unwrap();
        std::fs::rename(
            downloads.join("original.txt"),
            downloads.join("renamed.txt"),
        )
        .unwrap();
        let mut torrent = Torrent {
            hash: hash.clone(),
            name: "renamed.txt".into(),
            state: "stoppedUP".into(),
            progress: 1.0,
            size: 5,
            completed: 5,
            save_path: downloads.to_string_lossy().into_owned(),
            content_path: downloads.join("renamed.txt").to_string_lossy().into_owned(),
            category: "fixture".into(),
            tags: "read".into(),
            up_limit: NO_UPLOAD_LIMIT,
            ratio_limit: 0.0,
            ..Default::default()
        };
        let mut file = TorrentFile {
            name: "renamed.txt".into(),
            size: 5,
            progress: 1.0,
            priority: 1,
            ..Default::default()
        };
        let rejected = app
            .dispatcher
            .import_legacy(
                vec![torrent.clone()],
                HashMap::from([(hash.clone(), vec![file.clone()])]),
                backup.clone(),
            )
            .unwrap_err();
        assert!(rejected
            .to_string()
            .contains("renamed or unsupported layout"));
        assert!(!store.join("torrents.json").exists());
        assert!(!store.join(format!("{hash}.torrent")).exists());
        assert!(!downloads.join("original.txt").exists());
        assert_eq!(
            std::fs::read(downloads.join("renamed.txt")).unwrap(),
            b"hello"
        );
        std::fs::rename(
            downloads.join("renamed.txt"),
            downloads.join("original.txt"),
        )
        .unwrap();
        torrent.content_path = downloads
            .join("original.txt")
            .to_string_lossy()
            .into_owned();
        file.name = "original.txt".into();
        assert_eq!(
            app.dispatcher
                .import_legacy(
                    vec![torrent],
                    HashMap::from([(hash.clone(), vec![file])]),
                    backup
                )
                .unwrap(),
            1
        );
        assert!(app.dispatcher.legacy_migration_completed().unwrap());
        let record = app.dispatcher.runtime.block_on(async {
            let handle = app.dispatcher.inner.handle(&hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(handle.stats().finished);
            assert!(handle.is_paused());
            OwnedMutexGuard::lock(app.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone()
        });
        assert_eq!(record.upload_limit, NO_UPLOAD_LIMIT);
        assert_eq!(record.only_files, Some(vec![0]));
        assert_eq!(record.category, "fixture");
        assert_eq!(record.tags, "read");
        let error = app
            .dispatcher
            .dispatch(&request(
                "torrents/setUploadLimit",
                &[("hashes", &hash), ("limit", "1024")],
            ))
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("at least 16 KiB/s"));
        let error = app
            .dispatcher
            .dispatch(&request(
                "torrents/filePrio",
                &[("hash", &hash), ("id", "0"), ("priority", "7")],
            ))
            .unwrap()
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("high and maximum are unavailable"));
        assert_eq!(
            std::fs::read(downloads.join("original.txt")).unwrap(),
            b"hello"
        );
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loopback_transfer_obeys_cap_stops_with_app_and_resumes_exact_bytes() {
        use librqbit::{create_torrent, spawn_utils::BlockingSpawner, CreateTorrentOptions};
        use std::time::Instant;

        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-loopback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let seed_store = directory.join("seed-store");
        let seed_files = directory.join("seed-files");
        let client_files = directory.join("client-files");
        std::fs::create_dir_all(&seed_files).unwrap();
        std::fs::create_dir_all(&client_files).unwrap();
        let payload: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();
        std::fs::write(seed_files.join("fixture.bin"), &payload).unwrap();
        let mut seed = test_app(&seed_store, &seed_files, Ledger::default(), Some(0));
        let bytes = seed.dispatcher.runtime.block_on(async {
            create_torrent(
                &test_cx(),
                &seed_files.join("fixture.bin"),
                CreateTorrentOptions {
                    piece_length: Some(16 * 1024),
                    ..Default::default()
                },
                &BlockingSpawner::new(1),
            )
            .await
            .unwrap()
            .as_bytes()
            .unwrap()
        });
        let mut add = request("torrents/add", &[("tags", SEED_TAG)]);
        add.file = Some(crate::ipc::Upload {
            name: "fixture.torrent".into(),
            bytes: bytes.to_vec(),
        });
        seed.dispatcher.dispatch(&add).unwrap().unwrap();
        let hash = seed.dispatcher.runtime.block_on(async {
            let record = OwnedMutexGuard::lock(seed.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .clone();
            let handle = seed.dispatcher.inner.handle(&record.hash).unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(5),
                handle.wait_until_initialized(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(handle.stats().finished);
            record.hash
        });
        let cap = NO_UPLOAD_LIMIT.to_string();
        seed.dispatcher
            .dispatch(&request(
                "torrents/setUploadLimit",
                &[("hashes", &hash), ("limit", &cap)],
            ))
            .unwrap()
            .unwrap();
        let effective_upload_limit = seed.dispatcher.runtime.block_on(async {
            OwnedMutexGuard::lock(seed.dispatcher.inner.ledger.clone(), &test_cx())
                .await
                .unwrap()
                .records[0]
                .options(false)
                .unwrap()
                .ratelimits
                .upload_bps
                .map(|limit| limit.get())
        });
        let addr = seed.dispatcher.inner.session.listen_addr().unwrap();
        let retained = seed.dispatcher.inner.session.clone();
        let dispatcher = seed.dispatcher();
        seed.serve(&Config::default(), move |req| {
            dispatcher.dispatch(req).unwrap()
        })
        .unwrap();
        let socket = seed_store.join("runtime/engine.sock");
        assert!(socket.exists());
        assert!(InstanceLock::acquire_in(seed_store.join("runtime")).is_err());

        let client_runtime = NativeRuntime::new().unwrap();
        let client = client_runtime
            .block_on(Session::new_with_opts(
                client_runtime.cx(),
                client_files.clone(),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    ..Default::default()
                },
            ))
            .unwrap();
        let began = Instant::now();
        let (handle, partial, sample_before, elapsed, observations) =
            client_runtime.block_on(async {
                let handle = client
                    .add_torrent(
                        &test_cx(),
                        // Resolve real BEP 9 metadata from the seed before payload.
                        AddTorrent::from_url(format!("magnet:?xt=urn:btih:{hash}")),
                        Some(AddTorrentOptions {
                            overwrite: true,
                            initial_peers: Some(vec![addr]),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap()
                    .into_handle()
                    .unwrap();
                let mut observations = Vec::new();
                let (partial, sample_before, elapsed) =
                    time::timeout(&test_cx(), Duration::from_secs(15), async {
                        // During initialization progress_bytes counts disk bytes checked,
                        // including invalid/missing payload. Wait for the engine's state
                        // notification before interpreting it as downloaded payload.
                        // Otherwise a full hash-check scan looks like an uncapped transfer.
                        handle.wait_until_initialized(&test_cx()).await.unwrap();
                        loop {
                            let before = began.elapsed().as_secs_f64();
                            let stats = handle.stats();
                            let after = began.elapsed().as_secs_f64();
                            observations.push(serde_json::json!({
                                "before_s": before, "after_s": after,
                                "state": format!("{:?}", stats.state),
                                "progress_bytes": stats.progress_bytes,
                            }));
                            if stats.progress_bytes >= 64 * 1024 {
                                // Retain this sample and its bracketing monotonic times;
                                // resampling after block_on returns can race later payload.
                                break (stats.progress_bytes, before, after);
                            }
                            time::sleep(&test_cx(), Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .unwrap();
                (handle, partial, sample_before, elapsed, observations)
            });
        let observed = serde_json::json!({
            "partial": partial, "payload_len": payload.len(),
            "configured_upload_limit_bps": NO_UPLOAD_LIMIT,
            "effective_upload_limit_bps": effective_upload_limit,
            "sample_before_s": sample_before, "sample_after_s": elapsed,
            "observer_timestamps": observations,
        });
        let is_partial = partial < payload.len() as u64;
        let within_cap = partial as f64 <= NO_UPLOAD_LIMIT as f64 * (elapsed + 1.5);
        engine_test_diagnostic(
            "observe partial loopback payload",
            "The first observation is smaller than the full payload.",
            observed.clone(),
            is_partial,
        );
        engine_test_diagnostic(
            "check loopback upload cap allowance",
            "Payload bytes do not exceed cap * (monotonic elapsed seconds + 1.5).",
            observed,
            within_cap,
        );
        assert!(
            within_cap,
            "payload rate exceeded the configured cap, allowing its initial token burst"
        );
        assert!(is_partial);
        let closed = Instant::now();
        drop(seed);
        assert!(closed.elapsed() < Duration::from_secs(5));
        assert!(retained.cancellation_token().is_cancelled());
        assert!(!socket.exists());
        assert!(std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err());
        let stopped_bytes = handle.stats().progress_bytes;
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            handle.stats().progress_bytes,
            stopped_bytes,
            "closing the app must stop payload transfer"
        );

        let saved: Ledger =
            serde_json::from_slice(&std::fs::read(seed_store.join("torrents.json")).unwrap())
                .unwrap();
        assert_eq!(saved.records[0].upload_limit, NO_UPLOAD_LIMIT);
        assert!(
            !saved.records[0].paused,
            "app shutdown must preserve the intent to resume"
        );
        let seed = test_app(&seed_store, &seed_files, saved, Some(addr.port()));
        client_runtime.block_on(async {
            client.pause(&test_cx(), &handle).await.unwrap();
            client.unpause(&test_cx(), &handle).await.unwrap();
            time::timeout(
                &test_cx(),
                Duration::from_secs(20),
                handle.wait_until_completed(&test_cx()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                rqbit_runtime::fs::read(client_files.join("fixture.bin"))
                    .await
                    .unwrap(),
                payload
            );
            assert_eq!(handle.stats().progress_bytes, payload.len() as u64);
            client.stop(&test_cx()).await.unwrap();
        });
        assert!(
            began.elapsed() >= Duration::from_secs(5),
            "the fixture cannot complete at an uncapped loopback speed"
        );
        drop(seed);
        drop(retained);
        drop(client);
        client_runtime
            .shutdown_timeout(Duration::from_secs(2))
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pending_metadata_keeps_reads_responsive_and_cancels_on_close() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::os::unix::net::UnixStream;
        use std::sync::mpsc;
        use std::time::Instant;

        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-metadata-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = test_app(
            &directory,
            &directory.join("downloads"),
            Ledger::default(),
            None,
        );
        let dispatcher = app.dispatcher();
        let (handled_tx, handled_rx) = mpsc::channel();
        app.serve(&Config::default(), move |request| {
            let result = dispatcher.dispatch(request).unwrap();
            handled_tx
                .send(
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                )
                .unwrap();
            result
        })
        .unwrap();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!("http://{}/pending.torrent", listener.local_addr().unwrap());
        let (connected_tx, connected_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            connected_tx.send(()).unwrap();
            // Keep the HTTP lookup pending until the app closes.
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            drop(stream);
        });
        // Exercise the real owned socket worker, whose join is part of app
        // teardown. The HTTP peer remains open throughout the shutdown check.
        let mut client = UnixStream::connect(directory.join("runtime/engine.sock")).unwrap();
        let frame = serde_json::to_vec(&request("torrents/add", &[("urls", &url)])).unwrap();
        client
            .write_all(&(frame.len() as u32).to_be_bytes())
            .unwrap();
        client.write_all(&frame).unwrap();
        connected_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let began = Instant::now();
        let info = app
            .dispatcher
            .dispatch(&request("torrents/info", &[]))
            .unwrap()
            .unwrap();
        assert_eq!(info, "[]");
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "metadata lookup must not hold the ledger lock"
        );
        let closing = Instant::now();
        drop(app);
        assert!(handled_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err()
            .contains("closing"));
        assert!(
            closing.elapsed() < Duration::from_secs(4),
            "metadata lookup and socket worker must drain when the app closes"
        );
        assert!(!directory.join("runtime/engine.sock").exists());
        drop(client);
        release_tx.send(()).unwrap();
        peer.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
