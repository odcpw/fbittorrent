//! Opt-in signal diagnostics use a file because a closing pty loses stderr.
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Trace {
    began: Option<Instant>,
    pending: Option<i32>,
}
static TRACE: OnceLock<Mutex<Trace>> = OnceLock::new();
static REQUESTED: AtomicI32 = AtomicI32::new(0);

pub(super) fn request(signal: i32) {
    let _ = REQUESTED.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
}

pub(crate) fn event(event: &str) {
    if std::env::var_os("FBT_DEBUG_SIGNALS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let mut trace = TRACE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let pending = ftui::core::shutdown_signal::pending_termination_signal();
    let mut records = String::new();
    if trace.began.is_none() && pending.is_some() {
        trace.began = Some(Instant::now());
        records.push_str(&format!(
            "0 signal recorded {} (pending slot observed)\n",
            pending.unwrap()
        ));
    }
    let Some(began) = trace.began else {
        return;
    };
    if trace.pending.is_some() && pending.is_none() {
        records.push_str(&format!("{} clear observed\n", began.elapsed().as_millis()));
    }
    trace.pending = pending;
    if !event.is_empty() {
        records.push_str(&format!("{} {event}\n", began.elapsed().as_millis()));
    }
    if records.is_empty() {
        return;
    }
    if let Some(directory) = dirs::state_dir().map(|p| p.join("fbittorrent")) {
        use std::os::unix::fs::OpenOptionsExt;
        if std::fs::create_dir_all(&directory).is_ok() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(directory.join("tui-signals.log"))
            {
                let _ = file.write_all(records.as_bytes());
            }
        }
    }
}

/// The owner is taken exactly once, by normal teardown or the signal watchdog.
/// A dead pty can block ftui before its model/shutdown hooks ever run.
pub(crate) struct AppGuard<T: Send + 'static> {
    owner: Arc<Mutex<Option<T>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl<T: Send + 'static> AppGuard<T> {
    pub(crate) fn new(owner: T) -> Self {
        REQUESTED.store(0, Ordering::SeqCst);
        let owner = Arc::new(Mutex::new(Some(owner)));
        let stopping = Arc::new(AtomicBool::new(false));
        let (shared, stop) = (owner.clone(), stopping.clone());
        let thread = std::thread::spawn(move || {
            let mut drain: Option<std::thread::JoinHandle<()>> = None;
            while !stop.load(Ordering::Acquire) || drain.as_ref().is_some_and(|t| !t.is_finished())
            {
                let pending = ftui::core::shutdown_signal::pending_termination_signal();
                let requested = REQUESTED.load(Ordering::SeqCst);
                if let Some(signal) = (requested != 0).then_some(requested).or(pending) {
                    request(signal);
                    if drain.is_none() {
                        event("watchdog observed signal");
                    }
                    // Ack before the four-second drain, and keep acknowledging
                    // repeated hangups while the drain worker owns the engine.
                    if pending.is_some() {
                        ftui::core::shutdown_signal::clear_pending_termination_signal();
                        event("clear (watchdog)");
                    }
                    if drain.is_none() {
                        let owner = shared.lock().unwrap_or_else(|e| e.into_inner()).take();
                        drain = Some(std::thread::spawn(move || {
                            event("drop(app) begin (watchdog)");
                            drop(owner);
                            event("drop(app) end (watchdog)");
                            crate::search::shutdown_all();
                            std::process::exit(128 + signal);
                        }));
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Some(drain) = drain {
                let _ = drain.join();
            }
        });
        Self {
            owner,
            stopping,
            thread: Some(thread),
        }
    }
}
impl<T: Send + 'static> Drop for AppGuard<T> {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        event("drop(app) begin");
        drop(self.owner.lock().unwrap_or_else(|e| e.into_inner()).take());
        event("drop(app) end");
    }
}
