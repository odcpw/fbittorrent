pub mod lifecycle;
pub(crate) mod policy;
pub(crate) mod notify;
pub(crate) mod log;
mod log_capture;
mod runtime_dir;
mod sdnotify;
mod signals;
mod inhibit;
mod clipboard;
pub(crate) mod rss;
pub(crate) mod shows;
pub(crate) mod staged_sources;
pub(crate) mod watch;
pub(crate) mod history;
pub(crate) mod config_cache;
pub(crate) mod stats;
mod upgrade;
mod test_ready;
pub(crate) mod supervisor;
pub mod slot;

use crate::{
    agent::{AppError, Code},
    config::Config,
    engine, ipc, network_app,
};
use anyhow::Result;
use lifecycle::{Lifecycle, Request as LifecycleRequest, StopReason};
use slot::{EngineSlot, Slot, State};
use std::io::Read;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, Instant};

/// Process identity and engine state for the API server to borrow in P1-06.
/// Neither the legacy listener nor the instance lock belongs to an engine slot.
pub struct Daemon {
    pub lifecycle: Lifecycle,
    pub slot: RwLock<EngineSlot>,
    pub instance: String,
    pub launcher: &'static str,
    pub(crate) policy: Arc<Mutex<policy::Policy>>,
    pub(crate) settings: crate::api::ops::settings::Service,
    /// Last valid config.toml for the sampler and engine restarts.
    pub(crate) config: config_cache::ConfigCache,
    pub no_notify: bool,
    pub(crate) upgrade_pending: AtomicBool,
    pub(crate) notifier: notify::Notifier,
    event_bus: Mutex<std::sync::Weak<crate::api::bus::Bus>>,
    pub(crate) undo: Arc<engine::undo::History>,
    pub(crate) logs: Arc<log::Ring>,
    resume: Arc<engine::resume::Store>,
    pub(crate) inhibitor: inhibit::Inhibitor,
    pub(crate) clipboard: clipboard::Watcher,
    pub(crate) rss: rss::Service,
    pub(crate) shows: shows::Service,
    pub(crate) staged_sources: staged_sources::Registry,
    pub(crate) peers: crate::api::peers::Sampler,
    pub(crate) history: Arc<Mutex<history::History>>,
    pub(crate) stats: Mutex<stats::Stats>,
    pub(crate) disk: Mutex<supervisor::DiskMonitor>,
    pub(crate) imdb: crate::imdb::service::Service,
    pub(crate) search: crate::search::service::Service,
    pub(crate) watch: Mutex<watch::Watch>,
    network: RwLock<network_app::NetworkState>,
    stopping: AtomicBool,
}

impl Daemon {
    pub(crate) fn statistics(&self)->(crate::api::SessionTotals,crate::api::AllTimeTotals) {
        let slot=self.slot.read().unwrap_or_else(|e|e.into_inner());
        let mut stats=self.stats.lock().unwrap();
        if let Slot::Running(app)=&slot.slot {stats.observe(slot.engine_epoch,app.dispatcher().transfer_totals());}
        if let Err(error)=stats.save_cancellable(Instant::now(),false,Some(&self.stopping)){
            if !self.is_stopping() || !stats::checkpoint_deferred(&error) {eprintln!("ERROR stats save failed error={:?}",format!("{error:#}"));}
        }
        (stats.session.clone(),stats.all_time.clone())
    }

    pub(crate) fn idle_exit_secs(&self) -> u64 {
        self.policy.lock().unwrap().seconds()
    }

    pub(crate) fn network(&self) -> network_app::NetworkState {
        self.network.read().unwrap().clone()
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    /// Typed access shares the owner's slot and cancellation with legacy IPC.
    pub(crate) fn execute(&self, op: engine::ops::EngineOp) -> Result<engine::ops::EngineResult> {
        if self.is_stopping() {
            return Err(Self::unavailable(State::Stopping));
        }
        let began = Instant::now();
        loop {
            let slot = self.slot.read().unwrap_or_else(|error| error.into_inner());
            if self.is_stopping() {
                return Err(Self::unavailable(State::Stopping));
            }
            match &slot.slot {
                Slot::Starting if began.elapsed() < Duration::from_secs(60) => {}
                Slot::Starting => return Err(Self::unavailable(State::Starting)),
                Slot::Stopping => return Err(Self::unavailable(State::Stopping)),
                Slot::Parked(ledger) => {
                    if let engine::ops::EngineOp::Get { hash } = &op {
                        return engine::ops::parked_get(
                            ledger,
                            hash,
                            &crate::config::data_dir().join("engine"),
                        );
                    }
                    return ledger.execute(op);
                }
                Slot::Running(app) => {
                    return app.dispatcher().execute(op).map_err(|error| {
                        if app.is_quiescing() {
                            Self::unavailable(if self.is_stopping() {
                                State::Stopping
                            } else {
                                State::Starting
                            })
                        } else {
                            error
                        }
                    })
                }
            }
            drop(slot);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Apply settings to the current slot while its read guard prevents replacement.
    /// The durable config remains authoritative across a subsequent slot restart.
    pub(crate) fn apply_settings(&self, cfg: &Config) -> Result<()> {
        let began = Instant::now();
        loop {
            let slot = self.slot.read().unwrap();
            if self.is_stopping() { return Err(Self::unavailable(State::Stopping)); }
            match &slot.slot {
                Slot::Running(app) => { app.dispatcher().apply_settings(cfg)?; break; },
                Slot::Parked(_) => break,
                Slot::Starting if began.elapsed() < Duration::from_secs(60) => {},
                Slot::Starting => return Err(Self::unavailable(State::Starting)),
                Slot::Stopping => return Err(Self::unavailable(State::Stopping)),
            }
            drop(slot);
            std::thread::sleep(Duration::from_millis(20));
        }
        self.clipboard.enabled(cfg.clipboard_watch);
        Ok(())
    }

    pub fn info(&self) -> serde_json::Value {
        let slot = self.slot.read().unwrap_or_else(|error| error.into_inner());
        serde_json::json!({
            "server": {"version": env!("CARGO_PKG_VERSION"), "build": crate::api::build_id(),
                "pid": std::process::id(), "launcher": self.launcher,
                "log": if self.launcher == "systemd" { "journalctl --user -u fbittorrent" }
                    else { "the terminal running fbittorrent serve" }},
            "engine": format!("librqbit {}", librqbit::version()),
            "instance": self.instance, "engine_epoch": slot.engine_epoch,
            "state": slot.slot.state().name(), "policy": {"idle_exit_secs": self.idle_exit_secs()},
        })
    }

    fn unavailable(state: State) -> anyhow::Error {
        AppError::new(
            Code::Backend,
            format!("engine_unavailable state={} retryable=true", state.name()),
        )
        .hint("fbittorrent status --json")
        .into()
    }

    fn quiesce_engine(&self) {
        // Running handlers hold read guards. Cancellation makes their existing
        // engine dispatch select return before we wait for exclusive ownership.
        if let Slot::Running(app) = &self
            .slot
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .slot
        {
            app.quiesce();
        }
    }

    fn replace(&self, next: Slot) -> Result<Slot> {
        self.quiesce_engine();
        let mut slot = self.slot.write().unwrap_or_else(|error| error.into_inner());
        let old_epoch=slot.engine_epoch;
        let initialized=old_epoch>0;
        let mut old=slot.replace(next)?;
        eprintln!(
            "INFO engine state changed state={} engine_epoch={}",
            slot.slot.state().name(),
            slot.engine_epoch
        );
        let (epoch,state)=(slot.engine_epoch,slot.slot.state().name());
        drop(slot);
        // Record transitions at their source: a fast restart can finish between sampler ticks.
        // The sampler waits up to a second; the first slot also wakes it.
        if let Some(bus)=self.event_bus.lock().unwrap().upgrade(){if initialized {bus.engine_changed(epoch,state);} else {bus.refresh();}}
        if let Slot::Running(app)=&mut old {
            slot::drain_engine(app, Duration::from_secs(4))?;
            self.stats.lock().unwrap().observe(old_epoch,app.dispatcher().transfer_totals());
        }
        Ok(old)
    }
}

fn network_state(cfg: &Config) -> network_app::NetworkState {
    let mut state = network_app::state(cfg);
    if !state.ready {
        state.message = format!(
            "Waiting for {}",
            state.interface.as_deref().unwrap_or("configured interface")
        );
    }
    state
}

fn open_slot(daemon: &Daemon, cfg: &Config, lock: Arc<ipc::InstanceLock>) -> Result<()> {
    let network = network_state(cfg);
    *daemon.network.write().unwrap() = network.clone();
    if !network.ready {
        eprintln!("INFO network {}", network.message);
        daemon.replace(Slot::Parked(engine::ParkedLedger::new(
            crate::config::data_dir().join("engine"),
        ).with_network(network.clone()).with_undo(daemon.undo.clone())))?;
    } else {
        let mut engine_cfg = cfg.clone();
        engine_cfg.network_interface = network.interface.clone();
        eprintln!("INFO engine starting session");
        let engine =
            match engine::start_with_lock(&engine_cfg, daemon.lifecycle.clone(), lock.clone(), Some(daemon.resume.clone())) {
                Ok(engine) => engine,
                Err(_) if cfg.network_interface.is_some() && !network_state(cfg).ready => {
                    return open_slot(daemon, cfg, lock);
                }
                Err(error) => return Err(error),
            };
        engine.dispatcher().attach_undo(daemon.undo.clone());
        let history = daemon.history.clone();
        engine.dispatcher().attach_rates(Arc::new(move |download, upload| {
            history.lock().unwrap().record_tick(Instant::now(), download, upload);
        }));
        eprintln!(
            "INFO engine session exists restores={}",
            engine.dispatcher().restore_outcomes().len()
        );
        daemon.replace(Slot::Running(engine))?;
    }
    daemon.notifier.enabled(cfg.notify && !daemon.no_notify);
    let resumed = match daemon.execute(engine::ops::EngineOp::Info) {
        Ok(engine::ops::EngineResult::Torrents(rows)) => rows.into_iter()
            .filter_map(|row| crate::api::ops::torrents::torrent(row).ok())
            .filter(|row| row.intent == crate::api::Intent::Run && !row.staged && row.completed_at.is_none()).count(),
        _ => 0,
    };
    daemon.notifier.network(network.ready, cfg.network_interface.as_deref().unwrap_or("network"), resumed);
    Ok(())
}

fn supervise(
    daemon: &Daemon,
    mut cfg: Config,
    lock: Arc<ipc::InstanceLock>,
    notify: &sdnotify::Notify,
) -> Result<StopReason> {
    let mut upgrade = upgrade::Upgrade::new()?;
    open_slot(daemon, &cfg, lock.clone())?;
    notify.send("STATUS=Engine ready")?;
    loop {
        // Disk sampling rides on the bus observation (api/bus/sample.rs),
        // which already holds every row; see DiskMonitor::sample.
        let timeout = {
            let mut policy = daemon.policy.lock().unwrap();
            let now = Instant::now();
            if upgrade.check(now, policy.busy()) {
                daemon.lifecycle.request_stop(StopReason::Upgrade);
            }
            daemon.upgrade_pending.store(upgrade.pending(), Ordering::Release);
            if policy.expired(now) && !upgrade.pending() { return Ok(StopReason::Idle); }
            policy.wait(now)
        };
        // Network safety still checks once a second when bound. Otherwise the
        // idle supervisor sleeps five seconds, or until its earlier deadline.
        let timeout = if cfg.network_interface.is_some() {
            timeout.min(Duration::from_secs(1))
        } else { timeout };
        let request = daemon.lifecycle.wait(timeout.min(Duration::from_secs(5)).min(upgrade.wait(Instant::now())));
        daemon.undo.expire(&crate::config::data_dir().join("engine"));
        match request {
            Some(LifecycleRequest::Stop(reason)) => return Ok(reason),
            Some(LifecycleRequest::Restart(reason)) => {
                eprintln!("INFO daemon restarting engine reason={reason:?}");
                drop(daemon.replace(Slot::Starting)?);
                // A broken edit must not end the owner: restart with the last
                // valid settings; the sampler reports the unreadable file.
                cfg = daemon.config.config();
                if daemon.no_notify {
                    cfg.notify = false;
                }
                open_slot(daemon, &cfg, lock.clone())?;
            }
            None if cfg.network_interface.is_some() => {
                let network = network_state(&cfg);
                if *daemon.network.read().unwrap() != network {
                    // The network check runs only here, once per second, never
                    // in a client render or in the engine's request handlers.
                    drop(daemon.replace(Slot::Starting)?);
                    open_slot(daemon, &cfg, lock.clone())?;
                    notify.send(&format!(
                        "STATUS={}",
                        daemon.network.read().unwrap().message
                    ))?;
                }
            }
            None => {}
        }
    }
}

/// Blocks until the daemon owner has drained. Signal setup precedes all daemon
/// threads; main routes here before search::begin_session or any TUI guard.
pub fn run_serve(mut cfg: Config, idle_exit_secs: Option<u64>, no_notify: bool) -> Result<i32> {
    if no_notify {
        cfg.notify = false;
    }
    let directory = crate::config::runtime_dir();
    runtime_dir::validate(&directory)?;
    let lock = Arc::new(ipc::InstanceLock::acquire_wait(
        directory.clone(),
        Duration::from_secs(5),
    )?);
    engine::staging::discard_at_start(&crate::config::data_dir().join("engine"))?;
    crate::search::begin_session()?;
    struct SearchDrain;
    impl Drop for SearchDrain { fn drop(&mut self) { crate::search::shutdown_all(); } }
    let _search_drain = SearchDrain;
    let lifecycle = Lifecycle::default();
    let _signals = signals::Signals::install(lifecycle.clone())?;
    let log_capture = log_capture::Capture::install()?;
    let panicked = Arc::new(AtomicBool::new(false));
    let hook = PanicHook::install(lifecycle.clone(), panicked.clone());
    let notify = sdnotify::Notify::from_env()?;
    let mut random = [0; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let daemon = Arc::new(Daemon {
        lifecycle,
        slot: RwLock::new(EngineSlot::new()),
        instance: random.iter().map(|byte| format!("{byte:02x}")).collect(),
        launcher: if std::env::var_os("NOTIFY_SOCKET").is_some() {
            "systemd"
        } else {
            "foreground"
        },
        policy: Arc::new(Mutex::new(policy::Policy::new(cfg.idle_exit_secs, idle_exit_secs, Instant::now()))),
        settings: Default::default(),
        config: config_cache::ConfigCache::new(cfg.clone()),
        no_notify,
        upgrade_pending: AtomicBool::new(false),
        notifier: notify::Notifier::new(cfg.notify)?,
        event_bus: Mutex::default(),
        inhibitor: inhibit::Inhibitor::new()?,
        clipboard: clipboard::Watcher::new()?,
        rss: rss::Service::new()?,
        shows: shows::Service::new()?,
        staged_sources: Default::default(),
        peers: Default::default(),
        history: Default::default(),
        stats: Mutex::new(stats::Stats::open(stats::path(),Instant::now())?),
        logs: log_capture.ring.clone(),
        disk: Default::default(),
        imdb: Default::default(),
        undo: Arc::new(engine::undo::History::start(&crate::config::data_dir().join("engine"))?),
        search: Default::default(),
        watch: Mutex::new(watch::Watch::open(crate::config::data_dir().join("watch.json"))),
        resume: Arc::new(engine::resume::Store::open(&crate::config::data_dir().join("engine"))?),
        network: RwLock::new(network_state(&cfg)),
        stopping: AtomicBool::new(false),
    });
    let mut api = crate::api::server::Server::start(&directory, lock.clone(), daemon.clone())?;
    *daemon.event_bus.lock().unwrap()=api.event_bus();
    let rss_bus=api.event_bus();
    daemon.shows.set_bus(api.event_bus());
    daemon.rss.set_waker(Arc::new(move || {if let Some(bus)=rss_bus.upgrade(){bus.refresh();}}));
    daemon.clipboard.start(Arc::downgrade(&daemon), api.event_bus())?;
    daemon.clipboard.enabled(cfg.clipboard_watch);
    eprintln!(
        "INFO daemon listening {} no_notify={no_notify}",
        daemon.info()
    );
    // P1-13 owns desktop notifications. sd_notify remains active when
    // NOTIFY_SOCKET is set, independently of the notification preference.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        test_ready::wait(&daemon, &directory)?;
        notify.send("READY=1")?;
        eprintln!("INFO daemon ready state=starting");
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("fbittorrent-supervisor".into())
                .spawn_scoped(scope, || {
                    if std::env::var_os("FBT_TEST_PANIC_AFTER_READY").as_deref()
                        == Some(std::ffi::OsStr::new("1"))
                    {
                        panic!("FBT_TEST_PANIC_AFTER_READY injected daemon panic");
                    }
                    supervise(&daemon, cfg, lock.clone(), &notify)
                })?;
            worker.join().unwrap_or_else(|_| Ok(StopReason::Panic))
        })
    }));
    daemon.stopping.store(true, Ordering::Release);
    eprintln!("INFO daemon draining");
    // The bus and service workers can already be borrowing the engine while
    // waiting for its ledger. Cancel those dispatches BEFORE joining them;
    // joining first can wait forever and never reach the engine's drain fuse.
    daemon.quiesce_engine();
    eprintln!("DEBUG daemon joining component=shows");
    daemon.shows.stop();
    eprintln!("DEBUG daemon joining component=rss");
    daemon.rss.stop();
    eprintln!("DEBUG daemon joining component=clipboard");
    daemon.clipboard.stop();
    eprintln!("DEBUG daemon joining component=inhibitor");
    daemon.inhibitor.stop();
    eprintln!("DEBUG daemon joining component=notifier");
    daemon.notifier.stop();
    // Quiesce the persistent listener before taking the slot's writer lock.
    // Already-read requests complete; new handlers see stopping immediately.
    eprintln!("DEBUG daemon joining component=api_bus");
    api.stop_accepting();
    daemon.search.shutdown();
    let began = Instant::now();
    if daemon
        .slot
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .slot
        .state()
        != State::Stopping
    {
        let previous = daemon.replace(Slot::Stopping)?;
        let parked = matches!(&previous, Slot::Parked(_));
        drop(previous);
        if parked {
            eprintln!("INFO engine slot empty state=parked session=absent");
        }
    }
    eprintln!(
        "INFO daemon slot released elapsed_ms={}",
        began.elapsed().as_millis()
    );
    daemon.watch.lock().unwrap().flush(Instant::now() + Duration::from_secs(1));
    let stats_saved=daemon.stats.lock().unwrap().save(Instant::now(),true);
    let drain_reason = if panicked.load(Ordering::Acquire) {
        StopReason::Panic
    } else {
        result
            .as_ref()
            .ok()
            .and_then(|outcome| outcome.as_ref().ok())
            .copied()
            .unwrap_or(StopReason::Panic)
    };
    if drain_reason==StopReason::Panic {daemon.resume.abandon_clean();}
    else if let Err(error)=daemon.resume.preserve_parked_clean() {eprintln!("WARN resume parked clean marker unavailable error={error:?}");}
    api.drain(drain_reason);
    drop(api);
    eprintln!("INFO daemon sockets unlinked");
    drop(lock);
    if let Err(error) = notify.send("STOPPING=1") {
        eprintln!("WARN daemon stopping notification failed error={error}");
    }
    let reason = if panicked.load(Ordering::Acquire) {
        Ok(StopReason::Panic)
    } else {
        result.unwrap_or(Ok(StopReason::Panic))
    };
    drop(hook);
    let reason = reason?;
    if let Err(error)=stats_saved {
        if stats::checkpoint_deferred(&error) {
            eprintln!("INFO stats retained previous durable checkpoint reason=shutdown_deadline");
        } else {
            eprintln!("ERROR stats drain save failed error={:?}",format!("{error:#}"));
            if reason!=StopReason::Panic {return Err(error);}
        }
    }
    eprintln!(
        "INFO daemon stopped reason={reason:?} code={}",
        reason.exit_code()
    );
    Ok(reason.exit_code())
}

struct PanicHook(Option<Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync>>);
impl PanicHook {
    fn install(lifecycle: Lifecycle, panicked: Arc<AtomicBool>) -> Self {
        let old = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            panicked.store(true, Ordering::Release);
            eprintln!("ERROR daemon panic message={:?}", info.to_string());
            lifecycle.request_stop(StopReason::Panic);
        }));
        Self(Some(old))
    }
}
impl Drop for PanicHook {
    fn drop(&mut self) {
        // A setup/teardown panic is caught by main and exits 70. Rust refuses
        // hook changes during unwinding, so do not turn that panic into abort.
        if !std::thread::panicking() {
            if let Some(old) = self.0.take() {
                std::panic::set_hook(old);
            }
        }
    }
}
