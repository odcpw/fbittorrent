//! Opt-in, bounded clipboard selection reader. No clipboard contents are logged.
use super::Daemon;
use crate::{api::bus::Bus, engine::ops::{EngineOp, EngineResult}};
use serde_json::json;
use std::{
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::{net::UnixStream, process::CommandExt}},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const MAX: usize = 65_536;
#[derive(Default)]
struct State { enabled: bool, stopped: bool, generation: u64, last_hash: Option<String> }
pub(crate) struct Watcher {
    state: Arc<Mutex<State>>,
    wake: Mutex<UnixStream>,
    reader: Mutex<Option<UnixStream>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Watcher {
    pub fn new() -> io::Result<Self> {
        let (tx, rx) = UnixStream::pair()?;
        tx.set_nonblocking(true)?; rx.set_nonblocking(true)?;
        Ok(Self { state: Arc::default(), wake: Mutex::new(tx), reader: Mutex::new(Some(rx)), worker: Mutex::new(None) })
    }
    pub fn start(&self, daemon: Weak<Daemon>, bus: Weak<Bus>) -> io::Result<()> {
        let state = self.state.clone();
        let reader = self.reader.lock().unwrap().take().expect("one clipboard owner");
        *self.worker.lock().unwrap() = Some(thread::Builder::new().name("fbt-clipboard".into()).spawn(move || run(reader, state, daemon, bus))?);
        Ok(())
    }
    fn signal(&self) { let _ = self.wake.lock().unwrap().write(&[1]); }
    pub fn enabled(&self, value: bool) {
        let mut state = self.state.lock().unwrap();
        if state.enabled == value { return; }
        state.enabled = value; state.generation += 1;
        drop(state); self.signal();
    }
    pub fn dismiss(&self, hash: &str) {
        // Dismissal is idempotent. Keep the current hash so copying it again
        // cannot recreate the offer; stale dismissals cannot replace it.
        let mut state = self.state.lock().unwrap();
        if state.last_hash.as_deref() == Some(hash) { state.last_hash = Some(hash.into()); }
    }
    pub fn stop(&self) {
        self.state.lock().unwrap().stopped = true; self.signal();
        if let Some(worker) = self.worker.lock().unwrap().take() { let _ = worker.join(); }
    }
}
impl Drop for Watcher { fn drop(&mut self) { self.stop(); } }

#[derive(Default)]
struct Records { bytes: Vec<u8>, overflow: bool }
impl Records {
    fn clear(&mut self) { self.bytes.fill(0); self.bytes.clear(); self.overflow = false; }
    fn feed(&mut self, chunk: &[u8], mut offer: impl FnMut(&str)) {
        for &byte in chunk {
            if byte == 0 {
                if !self.overflow { if let Ok(text) = std::str::from_utf8(&self.bytes) { offer(text); } }
                self.clear();
            } else if !self.overflow {
                if self.bytes.len() == MAX { self.bytes.fill(0); self.bytes.clear(); self.overflow = true; }
                else { self.bytes.push(byte); }
            }
        }
    }
}
fn magnet(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    if !text.starts_with("magnet:?") { return None; }
    let url = url::Url::parse(text).ok()?;
    let xt = url.query_pairs().find(|(k,v)| k == "xt" && v.starts_with("urn:btih:"))?.1;
    // Parse only xt: a clipboard's untrusted so range must not cause an
    // enormous allocation inside the general magnet selection parser.
    let parsed = librqbit::Magnet::parse(&format!("magnet:?xt={xt}")).ok()?;
    let hash = parsed.as_id20()?.as_string();
    let name = url.query_pairs().find(|(k,v)| k == "dn" && !v.is_empty()).map(|(_,v)| v.chars().filter(|c| !c.is_control()).take(256).collect());
    Some((hash, name))
}
struct ChildGroup(Child);
impl Drop for ChildGroup {
    fn drop(&mut self) {
        let pgid = -(self.0.id() as i32);
        unsafe { libc::kill(pgid, libc::SIGTERM); }
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            if self.0.try_wait().ok().flatten().is_some() { break; }
            thread::sleep(Duration::from_millis(10));
        }
        unsafe { libc::kill(pgid, libc::SIGKILL); }
        let _ = self.0.wait();
    }
}
fn spawn() -> io::Result<ChildGroup> {
    let mut command = Command::new("wl-paste");
    command.args(["--watch", "sh", "-c", "cat; printf \"\\0\""])
        .env_remove("WAYLAND_DEBUG").stdin(Stdio::null()).stdout(Stdio::piped())
        .stderr(Stdio::null()).process_group(0);
    unsafe { command.pre_exec(|| {
        let mut empty = std::mem::zeroed(); libc::sigemptyset(&mut empty);
        if libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) != 0 { return Err(io::Error::last_os_error()); }
        libc::signal(libc::SIGPIPE, libc::SIG_DFL); Ok(())
    }); }
    let child = ChildGroup(command.spawn()?);
    let fd = child.0.stdout.as_ref().unwrap().as_raw_fd();
    if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0 { return Err(io::Error::last_os_error()); }
    eprintln!("INFO clipboard watcher started pid={}", child.0.id());
    Ok(child)
}
fn offer(text: &str, state: &Mutex<State>, daemon: &Weak<Daemon>, bus: &Weak<Bus>, generation: u64) {
    let Some((hash, name)) = magnet(text) else { return; };
    { let state = state.lock().unwrap(); if !state.enabled || state.stopped || state.generation != generation || state.last_hash.as_deref() == Some(&hash) { return; } }
    let (Some(daemon), Some(bus)) = (daemon.upgrade(), bus.upgrade()) else { return; };
    let Ok(EngineResult::Torrents(rows)) = daemon.execute(EngineOp::Info) else { return; };
    let mut state = state.lock().unwrap();
    if !state.enabled || state.stopped || state.generation != generation { return; }
    state.last_hash = Some(hash.clone());
    if rows.iter().any(|r| r.hash == hash) { return; }
    let mut event = json!({"ev":"clipboard.magnet","hash":hash,"already_added":false});
    if let Some(name) = name { event["name"] = json!(name); }
    bus.publish(event.clone());
    daemon.notifier.clipboard(&event);
}
fn run(mut wake: UnixStream, state: Arc<Mutex<State>>, daemon: Weak<Daemon>, bus: Weak<Bus>) {
    let mut child: Option<ChildGroup> = None;
    let mut records = Records { bytes: Vec::with_capacity(MAX), overflow: false };
    let mut generation = 0;
    let mut retry = Instant::now();
    loop {
        let s = state.lock().unwrap();
        if s.stopped { break; }
        let enabled = s.enabled;
        if generation != s.generation || !enabled { drop(child.take()); records.clear(); retry = Instant::now(); generation = s.generation; }
        drop(s);
        if enabled && child.is_none() && Instant::now() >= retry {
            // Serialize launch with disabling the preference. A stale poll
            // iteration must never start a new reader after off was applied.
            let current = state.lock().unwrap();
            if current.enabled && !current.stopped && current.generation == generation {
                match spawn() { Ok(value) => child = Some(value), Err(_) => eprintln!("WARN clipboard watcher unavailable") }
            }
            retry = Instant::now() + Duration::from_secs(5);
        }
        let mut fds = [libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: child.as_ref().map_or(-1, |c| c.0.stdout.as_ref().unwrap().as_raw_fd()), events: libc::POLLIN, revents: 0 }];
        let timeout = if enabled && child.is_none() { retry.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as i32 } else { -1 };
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) } < 0 { if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted { continue; } break; }
        if fds[0].revents != 0 { let mut bytes = [0;128]; while wake.read(&mut bytes).is_ok_and(|n| n>0) {} continue; }
        if fds[1].revents != 0 {
            let mut bytes = [0;8192];
            // A bounded read per turn lets off/drain win even with a producer
            // continuously filling the pipe. poll blocks when there is no data.
            match child.as_mut().unwrap().0.stdout.as_mut().unwrap().read(&mut bytes) {
                Ok(0) => { drop(child.take()); records.clear(); retry = Instant::now()+Duration::from_secs(5); },
                Ok(n) => { records.feed(&bytes[..n], |text| offer(text, &state, &daemon, &bus, generation)); bytes.fill(0); },
                Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {},
                Err(_) => { drop(child.take()); records.clear(); retry = Instant::now()+Duration::from_secs(5); },
            }
        }
    }
    records.clear(); // Do not retain partial selection bytes after off/drain.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_records_preserve_selection_boundaries() {
        let mut reader = Records::default(); let mut out = Vec::new();
        reader.feed(b"first", |s|out.push(s.to_owned())); assert!(out.is_empty());
        reader.feed(b"\0second\0third", |s|out.push(s.to_owned())); assert_eq!(out, ["first","second"]);
        reader.feed(&vec![b'x';MAX+1], |s|out.push(s.to_owned()));
        reader.feed(b"\0after\0", |s|out.push(s.to_owned())); assert_eq!(out, ["first","second","after"]);
        assert!(reader.bytes.is_empty()); assert!(reader.bytes.capacity() <= MAX);
        reader.feed(&vec![b'y';MAX], |s|out.push(s.to_owned())); reader.feed(b"\0", |s|out.push(s.to_owned())); assert_eq!(out.last().unwrap().len(), MAX);
    }
    #[test]
    fn parse_only_bounded_magnet_identity() {
        let hash = "1234567890123456789012345678901234567890";
        assert_eq!(magnet(&format!("magnet:?xt=urn:btih:{hash}&dn=Local%20name&so=0-18446744073709551615")),Some((hash.into(),Some("Local name".into()))));
        for bad in ["password", "https://example.org/", "magnet:?xt=urn:btmh:1220abcd", "magnet:?xt=urn:btih:bad"] { assert!(magnet(bad).is_none()); }
    }
}
