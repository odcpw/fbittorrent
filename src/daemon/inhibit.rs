//! One owned inhibitor while runnable transfers need the machine awake.
use crate::engine::view::{Intent, TorrentState, TorrentView};
use std::{
    io,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn eligible(intent: Intent, state: TorrentState) -> bool {
    intent == Intent::Run && matches!(state, TorrentState::Downloading | TorrentState::Stalled | TorrentState::Metadata | TorrentState::Checking)
}
trait Held: Send {
    fn exited(&mut self) -> io::Result<bool>;
    fn close(&mut self);
}
struct Group(Option<Child>);
impl Held for Group {
    fn exited(&mut self) -> io::Result<bool> { Ok(self.0.as_mut().unwrap().try_wait()?.is_some()) }
    fn close(&mut self) {
        // The launcher can exit before sleep. Its owned process group is still
        // ours to kill, independently of the launcher's current status.
        let Some(mut child) = self.0.take() else { return; };
        let pgid = -(child.id() as i32);
        unsafe { libc::kill(pgid, libc::SIGTERM); }
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            if child.try_wait().ok().flatten().is_some() { break; }
            thread::sleep(Duration::from_millis(10));
        }
        unsafe { libc::kill(pgid, libc::SIGKILL); }
        let _ = child.wait();
    }
}
impl Drop for Group { fn drop(&mut self) { self.close(); } }
type Spawn = Arc<dyn Fn() -> io::Result<Box<dyn Held>> + Send + Sync>;
#[derive(Default)]
struct Desired { wanted: bool, stopped: bool }
#[derive(Default)]
struct Shared { state: Mutex<Desired>, changed: Condvar }
pub(crate) struct Inhibitor { shared: Arc<Shared>, worker: Mutex<Option<JoinHandle<()>>> }
impl Inhibitor {
    pub fn new() -> io::Result<Self> {
        Self::with_spawn(Arc::new(|| {
            let mut cmd = Command::new("systemd-inhibit");
            cmd.args(["--what=sleep", "--who=fbittorrent", "--why=Downloading", "--mode=block", "sleep", "infinity"])
                .stdin(Stdio::null()).stdout(Stdio::null()).process_group(0);
            // The owner blocks shutdown signals for sigwait. External children
            // must receive their normal signal semantics after exec.
            unsafe {
                cmd.pre_exec(|| {
                    let mut empty = std::mem::zeroed();
                    libc::sigemptyset(&mut empty);
                    if libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) != 0 { return Err(io::Error::last_os_error()); }
                    libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                    Ok(())
                });
            }
            // Keep the real session bus reachable without changing the child's
            // private XDG_RUNTIME_DIR (important in isolated launchers).
            if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
                cmd.env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path=/run/user/{}/bus", unsafe { libc::geteuid() }));
            }
            let child = cmd.spawn()?;
            eprintln!("INFO inhibit started pid={}", child.id());
            Ok(Box::new(Group(Some(child))))
        }))
    }
    fn with_spawn(spawn: Spawn) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let worker = thread::Builder::new().name("fbt-inhibit".into()).spawn(move || run(state, spawn))?;
        Ok(Self { shared, worker: Mutex::new(Some(worker)) })
    }
    pub fn observe(&self, rows: &[TorrentView]) { self.wanted(rows.iter().any(|r| eligible(r.intent, r.state))); }
    fn wanted(&self, wanted: bool) {
        let mut state = self.shared.state.lock().unwrap();
        if state.wanted != wanted { state.wanted = wanted; self.shared.changed.notify_one(); }
    }
    pub fn stop(&self) {
        self.shared.state.lock().unwrap().stopped = true;
        self.shared.changed.notify_one();
        if let Some(worker) = self.worker.lock().unwrap().take() { let _ = worker.join(); }
    }
}
impl Drop for Inhibitor { fn drop(&mut self) { self.stop(); } }
fn run(shared: Arc<Shared>, spawn: Spawn) {
    let mut child: Option<Box<dyn Held>> = None;
    loop {
        let mut state = shared.state.lock().unwrap();
        if state.stopped || !state.wanted {
            if let Some(mut child) = child.take() {
                drop(state);
                child.close();
                eprintln!("INFO inhibit released");
                continue;
            }
            if state.stopped { return; }
            drop(shared.changed.wait(state).unwrap());
            continue;
        }
        drop(state);
        if child.as_mut().is_some_and(|c| c.exited().unwrap_or(true)) {
            child.take().unwrap().close();
        }
        if child.is_none() {
            match spawn() {
                Ok(value) => child = Some(value),
                Err(error) => eprintln!("WARN inhibit could not start message={:?}", error.to_string()),
            }
        }
        state = shared.state.lock().unwrap();
        if state.wanted && !state.stopped {
            drop(shared.changed.wait_timeout(state, Duration::from_secs(5)).unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_edges() {
        struct Fake(Arc<Mutex<(usize, usize)>>);
        impl Held for Fake {
            fn exited(&mut self) -> io::Result<bool> { Ok(false) }
            fn close(&mut self) { self.0.lock().unwrap().1 += 1; }
        }
        let counts = Arc::new(Mutex::new((0, 0)));
        let copies = counts.clone();
        let owner = Inhibitor::with_spawn(Arc::new(move || {
            copies.lock().unwrap().0 += 1;
            eprintln!("inhibit-test decision=spawn");
            Ok(Box::new(Fake(copies.clone())))
        })).unwrap();
        let wait = |expected| {
            let began = Instant::now();
            while *counts.lock().unwrap() != expected && began.elapsed() < Duration::from_secs(2) { thread::sleep(Duration::from_millis(1)); }
            assert_eq!(*counts.lock().unwrap(), expected);
            eprintln!("inhibit-test decision=observe counts={expected:?} elapsed_ms={}", began.elapsed().as_millis());
        };
        for state in [TorrentState::Downloading, TorrentState::Stalled, TorrentState::Metadata, TorrentState::Checking] {
            assert!(eligible(Intent::Run, state));
            assert!(!eligible(Intent::Pause, state));
            owner.wanted(eligible(Intent::Run, state));
            wait((1, 0));
        }
        for state in [TorrentState::Paused, TorrentState::Done, TorrentState::Seeding, TorrentState::Queued, TorrentState::WaitingVpn, TorrentState::Error] { assert!(!eligible(Intent::Run, state)); }
        owner.wanted(false); wait((1, 1));
        owner.wanted(true); wait((2, 1));
        owner.stop(); wait((2, 2));
    }
}
