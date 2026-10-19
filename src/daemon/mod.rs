pub mod lifecycle;
mod runtime_dir;
mod sdnotify;
mod signals;
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
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, RwLock,
};
use std::time::{Duration, Instant};

/// Process identity and engine state for the API server to borrow in P1-06.
/// Neither the legacy listener nor the instance lock belongs to an engine slot.
pub struct Daemon {
    pub lifecycle: Lifecycle,
    pub slot: RwLock<EngineSlot>,
    pub instance: String,
    pub launcher: &'static str,
    pub idle_exit_secs: AtomicU64,
    pub(crate) settings: crate::api::ops::settings::Service,
    pub no_notify: bool,
    pub(crate) undo: Arc<engine::undo::History>,
    resume: Arc<engine::resume::Store>,
    pub(crate) imdb: crate::imdb::service::Service,
    network: RwLock<network_app::NetworkState>,
    stopping: AtomicBool,
}

impl Daemon {
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
        if let Some(result) = op.control(&self.network(), &self.lifecycle) {
            return result;
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
            "state": slot.slot.state().name(), "policy": {"idle_exit_secs": self.idle_exit_secs.load(Ordering::Acquire)},
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

    fn dispatch(&self, request: &ipc::Request) -> Result<String> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Self::unavailable(State::Stopping));
        }
        let op = crate::api::legacy::parse(request)?;
        let network = self.network.read().unwrap().clone();
        if let Some(result) = crate::api::legacy::control(&op, &network, &self.lifecycle) {
            return result;
        }
        let began = Instant::now();
        loop {
            let slot = self.slot.read().unwrap_or_else(|error| error.into_inner());
            if self.stopping.load(Ordering::Acquire) {
                return Err(Self::unavailable(State::Stopping));
            }
            match &slot.slot {
                Slot::Starting => {
                    if began.elapsed() >= Duration::from_secs(60) {
                        return Err(Self::unavailable(State::Starting));
                    }
                }
                Slot::Running(app) => {
                    eprintln!(
                        "DEBUG api legacy dispatch path={:?}",
                        request.path.chars().take(80).collect::<String>()
                    );
                    return crate::api::legacy::running(&app.dispatcher(), op).map_err(|error| {
                        if app.is_quiescing() {
                            Self::unavailable(if self.stopping.load(Ordering::Acquire) {
                                State::Stopping
                            } else {
                                State::Starting
                            })
                        } else {
                            error
                        }
                    });
                }
                Slot::Parked(ledger) => {
                    let interface = network
                        .interface
                        .as_deref()
                        .unwrap_or("configured interface");
                    return crate::api::legacy::parked(ledger, op, interface);
                }
                Slot::Stopping => return Err(Self::unavailable(State::Stopping)),
            }
            drop(slot);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn replace(&self, next: Slot) -> Result<Slot> {
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
        let mut slot = self.slot.write().unwrap_or_else(|error| error.into_inner());
        let old = slot.replace(next)?;
        eprintln!(
            "INFO engine state changed state={} engine_epoch={}",
            slot.slot.state().name(),
            slot.engine_epoch
        );
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
    daemon.lifecycle.set_expected_network(Some(network.clone()));
    if !network.ready {
        eprintln!("INFO network {}", network.message);
        daemon.replace(Slot::Parked(engine::ParkedLedger::new(
            crate::config::data_dir().join("engine"),
            daemon.lifecycle.clone(),
        ).with_undo(daemon.undo.clone())))?;
    } else {
        let mut engine_cfg = cfg.clone();
        engine_cfg.network_interface = network.interface;
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
        eprintln!(
            "INFO engine session exists restores={}",
            engine.dispatcher().restore_outcomes().len()
        );
        daemon.replace(Slot::Running(engine))?;
    }
    Ok(())
}

fn supervise(
    daemon: &Daemon,
    mut cfg: Config,
    lock: Arc<ipc::InstanceLock>,
    notify: &sdnotify::Notify,
) -> Result<StopReason> {
    open_slot(daemon, &cfg, lock.clone())?;
    notify.send("STATUS=Engine ready")?;
    loop {
        let request = daemon.lifecycle.wait(Duration::from_secs(1));
        daemon.undo.expire(&crate::config::data_dir().join("engine"));
        match request {
            Some(LifecycleRequest::Stop(reason)) => return Ok(reason),
            Some(LifecycleRequest::Restart(reason)) => {
                eprintln!("INFO daemon restarting engine reason={reason:?}");
                drop(daemon.replace(Slot::Starting)?);
                cfg = Config::load()?;
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
pub fn run_serve(mut cfg: Config, idle_exit_secs: u64, no_notify: bool) -> Result<i32> {
    if no_notify {
        cfg.notify = false;
    }
    let directory = crate::config::runtime_dir();
    runtime_dir::validate(&directory)?;
    let lock = Arc::new(ipc::InstanceLock::acquire_wait(
        directory.clone(),
        Duration::from_secs(5),
    )?);
    let lifecycle = Lifecycle::default();
    let _signals = signals::Signals::install(lifecycle.clone())?;
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
        idle_exit_secs: AtomicU64::new(idle_exit_secs),
        settings: Default::default(),
        no_notify,
        imdb: Default::default(),
        undo: Arc::new(engine::undo::History::start(&crate::config::data_dir().join("engine"))?),
        resume: Arc::new(engine::resume::Store::open(&crate::config::data_dir().join("engine"))?),
        network: RwLock::new(network_state(&cfg)),
        stopping: AtomicBool::new(false),
    });
    let dispatch = daemon.clone();
    let mut server = ipc::Server::start(&cfg, &lock, move |request| dispatch.dispatch(request))?;
    let mut api = crate::api::server::Server::start(&directory, lock.clone(), daemon.clone())?;
    eprintln!(
        "INFO daemon listening {} no_notify={no_notify}",
        daemon.info()
    );
    // P1-12 owns the idle busy table; P1-13 owns desktop notifications and
    // consumes no_notify. sd_notify is always active when NOTIFY_SOCKET is set.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
    // Quiesce the persistent listener before taking the slot's writer lock.
    // Already-read requests complete; new handlers see stopping immediately.
    server.stop_accepting();
    api.stop_accepting();
    let began = Instant::now();
    if daemon
        .slot
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .slot
        .state()
        != State::Stopping
    {
        drop(daemon.replace(Slot::Stopping)?);
    }
    eprintln!(
        "INFO daemon slot released elapsed_ms={}",
        began.elapsed().as_millis()
    );
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
    drop(server);
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
