//! Bounded desktop delivery. Pending includes active children; shutdown kills
//! owned process groups before their sandbox or daemon listener can disappear.
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}
#[derive(Clone)]
struct Notice {
    kind: String,
    hash: String,
    serial: u64,
    title: String,
    body: String,
    target: Option<PathBuf>,
    actions: Vec<(&'static str, &'static str)>,
}
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Notice>,
    pending: HashSet<(String, String)>,
    enabled: bool,
    stopped: bool,
    serial: u64,
    network: Option<bool>,
    vpn_at: [Option<Instant>; 2],
}
struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
}
pub(crate) struct Notifier {
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
impl Notifier {
    pub fn new(enabled: bool) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                enabled,
                ..Queue::default()
            }),
            changed: Condvar::new(),
        });
        let owner = Self {
            shared,
            workers: Mutex::new(Vec::new()),
        };
        for i in 0..4 {
            let shared = owner.shared.clone();
            owner.workers.lock().unwrap().push(
                thread::Builder::new()
                    .name(format!("fbt-notify-{i}"))
                    .spawn(move || worker(shared))?,
            );
        }
        Ok(owner)
    }
    pub fn enabled(&self, value: bool) {
        let mut q = self.shared.queue.lock().unwrap();
        if q.enabled == value {
            return;
        }
        q.enabled = value;
        if !value {
            while let Some(n) = q.jobs.pop_front() {
                q.pending.remove(&(n.kind, n.hash));
            }
        }
        self.shared.changed.notify_all();
    }
    fn enqueue(&self, mut n: Notice) {
        let mut q = self.shared.queue.lock().unwrap();
        if !q.enabled || q.stopped || q.pending.contains(&(n.kind.clone(), n.hash.clone())) {
            return;
        }
        if q.jobs.len() >= 512 {
            eprintln!("WARN notify queue full capacity=512");
            return;
        }
        q.serial += 1;
        n.serial = q.serial;
        q.pending.insert((n.kind.clone(), n.hash.clone()));
        q.jobs.push_back(n);
        self.shared.changed.notify_one();
    }
    pub fn event(&self, event: &Value) {
        let kind = event["ev"].as_str().unwrap_or("");
        let (title, actions) = match kind {
            "torrent.added" if event["origin"] == "handler" => {
                ("Added", vec![("undo", "Undo"), ("window", "Open window")])
            }
            "torrent.playable" => ("Ready to watch", vec![("default", "Open"), ("open", "Open")]),
            "torrent.completed" => (
                "Download complete",
                vec![
                    ("default", "Open"),
                    ("open", "Open File"),
                    ("folder", "Open Folder"),
                ],
            ),
            _ => return,
        };
        self.enqueue(Notice {
            kind: kind.into(),
            hash: event["hash"].as_str().unwrap_or("").into(),
            serial: 0,
            title: title.into(),
            body: event["name"].as_str().unwrap_or("Torrent").into(),
            target: event["open_target"].as_str().map(PathBuf::from),
            actions,
        });
    }
    pub fn disk_low(&self) {
        self.enqueue(Notice { kind:"disk".into(), hash:"disk".into(), serial:0,
            title:"Disk almost full".into(), body:String::new(), target:None,
            actions:vec![("window","Open window")] });
    }
    pub fn clipboard(&self, event: &Value) {
        self.enqueue(Notice { kind:"clipboard".into(), hash:event["hash"].as_str().unwrap_or("").into(), serial:0,
            title:format!("Magnet on clipboard: {}",event["name"].as_str().unwrap_or("Torrent")),
            body:String::new(), target:None, actions:vec![("add","Add"),("dismiss","Dismiss")] });
    }
    pub fn failed(&self) {
        self.enqueue(Notice {
            kind: "failed".into(),
            hash: "add".into(),
            serial: 0,
            title: "Could not add".into(),
            body: "Could not add this torrent. Inspect the log for details.".into(),
            target: None,
            actions: vec![("log", "Open log")],
        });
    }
    pub fn network(&self, ready: bool, interface: &str, count: usize) {
        let now = Instant::now();
        let mut q = self.shared.queue.lock().unwrap();
        let changed = q.network.replace(ready) != Some(ready);
        if !changed || (ready && q.vpn_at.iter().all(Option::is_none)) {
            return;
        }
        let index = usize::from(ready);
        if q.vpn_at[index].is_some_and(|at| now.duration_since(at) < Duration::from_secs(2)) {
            return;
        }
        q.vpn_at[index] = Some(now);
        drop(q);
        self.enqueue(Notice {
            kind: "network".into(),
            hash: ready.to_string(),
            serial: 0,
            title: if ready {
                format!("VPN back; {count} downloads resumed")
            } else {
                format!("Waiting for {interface}")
            },
            body: String::new(),
            target: None,
            actions: Vec::new(),
        });
    }
    pub fn stop(&self) {
        let mut q = self.shared.queue.lock().unwrap();
        q.stopped = true;
        q.jobs.clear();
        self.shared.changed.notify_all();
        drop(q);
        for worker in self.workers.lock().unwrap().drain(..) {
            let _ = worker.join();
        }
    }
}
impl Drop for Notifier {
    fn drop(&mut self) {
        self.stop();
    }
}
fn worker(shared: Arc<Shared>) {
    loop {
        let notice = {
            let mut q = shared.queue.lock().unwrap();
            while q.jobs.is_empty() && !q.stopped {
                q = shared.changed.wait(q).unwrap();
            }
            if q.stopped {
                return;
            }
            q.jobs.pop_front().unwrap()
        };
        if let Err(error) = deliver(&shared, &notice) {
            eprintln!(
                "WARN notify delivery failed message={:?}",
                error.to_string()
            );
        }
        shared
            .queue
            .lock()
            .unwrap()
            .pending
            .remove(&(notice.kind, notice.hash));
    }
}
fn deliver(shared: &Shared, n: &Notice) -> anyhow::Result<()> {
    if !live(shared) {
        return Ok(());
    }
    let mut command = Command::new("notify-send");
    command.args(["-a", "fbittorrent", "-i", "fbittorrent"]);
    for (action, label) in &n.actions {
        command.arg("-A").arg(format!("{action}={label}"));
    }
    if n.kind == "clipboard" { command.stderr(Stdio::null()); }
    let mut child = command
        .arg(escape_markup(&n.title))
        .arg(escape_markup(&n.body))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if !live(shared) {
            break None;
        }
        thread::sleep(Duration::from_millis(25));
    };
    // A shell restart can leave descendants holding stdout. Kill only this
    // notification's group and never act on a failed/killed notification.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
    if !status.is_some_and(|s| s.success()) || !live(shared) {
        return Ok(());
    }
    let mut output = String::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(256).read_to_string(&mut output)?;
    }
    let action = output.trim();
    if !n.actions.iter().any(|(name, _)| *name == action) {
        return Ok(());
    }
    match action {
        "add" => {
            // Only the infohash survives selection parsing. Reconstruct the
            // magnet instead of keeping clipboard text or tracker URLs.
            request("torrents.add", json!({"source":{"magnet":format!("magnet:?xt=urn:btih:{}",n.hash)},"origin":"clipboard"}),
                Some(&format!("notify-clipboard-{}-{}", std::process::id(), n.serial)))?;
        }
        "dismiss" => { request("app.clipboard.dismiss", json!({"hash":n.hash}), None)?; }
        "undo" => {
            request(
                "torrents.remove",
                json!({"hashes":[n.hash],"delete_files":false}),
                Some(&format!("notify-undo-{}-{}", std::process::id(), n.serial)),
            )?;
        }
        "open" | "default" | "folder" => {
            if let Some(target) = &n.target {
                if n.kind == "torrent.playable" {request("torrents.open_target", json!({"hash":n.hash}), None)?;}
                let path = if action == "folder" {
                    if target.is_dir() {
                        target
                    } else {
                        target.parent().unwrap_or(target)
                    }
                } else {
                    target.as_path()
                };
                run_action(shared, Command::new("xdg-open").arg(path))?;
            }
        }
        "window" | "log" => {
            run_action(
                shared,
                Command::new(std::env::current_exe()?).arg(if action == "window" {
                    "dashboard"
                } else {
                    "log"
                }),
            )?;
        }
        _ => {}
    }
    Ok(())
}
fn live(shared: &Shared) -> bool {
    let q = shared.queue.lock().unwrap();
    !q.stopped && q.enabled
}
fn run_action(shared: &Shared, command: &mut Command) -> anyhow::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn()?;
    let end = Instant::now() + Duration::from_secs(5);
    while child.try_wait()?.is_none() {
        if !live(shared) || Instant::now() >= end {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
/// Notification actions use the real, bounded API and a stable idempotency ref.
fn request(op: &str, args: Value, reference: Option<&str>) -> anyhow::Result<Value> {
    let mut stream = UnixStream::connect(crate::config::runtime_dir().join("api.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let hello = json!({"id":1,"op":"hello","args":{"proto":1,"client":{"kind":"cli","name":"notification-action","version":env!("CARGO_PKG_VERSION")}}});
    writeln!(stream, "{hello}")?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    anyhow::ensure!(
        serde_json::from_str::<Value>(&line)?["ok"] == true,
        "notification API hello failed"
    );
    let mut call = json!({"id":2,"op":op,"args":args});
    if let Some(reference) = reference {
        call["ref"] = json!(reference);
    }
    writeln!(stream, "{call}")?;
    for _ in 0..64 {
        line.clear();
        reader.by_ref().take(1_048_577).read_line(&mut line)?;
        anyhow::ensure!(
            !line.is_empty() && line.len() <= 1_048_576,
            "notification API frame unavailable"
        );
        let answer: Value = serde_json::from_str(&line)?;
        if answer["id"] == 2 {
            anyhow::ensure!(answer["ok"] == true, "notification action refused");
            return Ok(answer);
        }
    }
    anyhow::bail!("notification action response unavailable")
}

#[cfg(test)]
mod tests {
    #[test]
    fn pending_notices_are_deduplicated_and_queue_is_bounded() {
        use super::*;
        let owner = Notifier {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    enabled: true,
                    ..Queue::default()
                }),
                changed: Condvar::new(),
            }),
            workers: Mutex::new(Vec::new()),
        };
        for i in 0..600 {
            let event = json!({"ev":"torrent.completed","hash":format!("{i:040x}"),"name":"unit"});
            owner.event(&event);
            owner.event(&event);
        }
        let q = owner.shared.queue.lock().unwrap();
        assert_eq!(q.jobs.len(), 512);
        assert_eq!(q.pending.len(), 512);
        drop(q);
        owner.enabled(false);
        assert!(owner.shared.queue.lock().unwrap().pending.is_empty());
    }
    #[test]
    fn escape_markup() {
        assert_eq!(super::escape_markup("A<&B &amp;"), "A&lt;&amp;B &amp;amp;");
    }
}
