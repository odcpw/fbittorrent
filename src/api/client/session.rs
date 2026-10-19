//! Bounded blocking API session. One reader owns framing; requests own reply channels.
use super::*;
use crate::api::{self, HelloResult};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
};
static NEXT_REF: AtomicU64 = AtomicU64::new(1);
type Pending = Arc<Mutex<HashMap<u64, mpsc::SyncSender<std::result::Result<Value, String>>>>>;
#[derive(Debug)]
struct Disconnected(String);
impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Disconnected {}

struct Connection {
    writer: Mutex<UnixStream>,
    pending: Pending,
    events: Mutex<mpsc::Receiver<Value>>,
    reader: Option<std::thread::JoinHandle<()>>,
    hello: HelloResult,
    end_reason: Arc<Mutex<Option<String>>>,
}
impl Connection {
    fn connect(cfg: &Config, agent: bool, path: &std::path::Path, name: Option<&str>) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut writer = connect_probe(path, deadline)?;
        writer.set_write_timeout(Some(Duration::from_millis(200)))?;
        let mut reader = BufReader::new(writer.try_clone()?);
        send(
            &mut writer,
            &json!({"id":1,"op":"hello","args":{"proto":1,"client":{"name":name.map(str::to_owned).unwrap_or_else(||format!("cli-{}",std::process::id())),"kind":if agent{"agent"}else{"cli"},"version":env!("CARGO_PKG_VERSION")}}}),
        )?;
        let mut bytes = Vec::new();
        let frame = read_frame(&mut reader, &mut bytes, Some(deadline))?;
        let id = frame["id"].clone();
        let result = result(frame)?;
        anyhow::ensure!(id == 1, "API hello has a mismatched request id");
        let hello: HelloResult = serde_json::from_value(result)?;
        anyhow::ensure!(
            hello.proto == 1,
            "unsupported API protocol; run fbittorrent restart"
        );
        reader.get_ref().set_read_timeout(None)?;
        writer.set_write_timeout(Some(Duration::from_secs(cfg.timeout_secs.max(1))))?;
        let pending: Pending = Arc::default();
        let replies = pending.clone();
        let end_reason: Arc<Mutex<Option<String>>> = Arc::default();
        let ended = end_reason.clone();
        let (events, receiver) = mpsc::sync_channel(256);
        let thread = std::thread::Builder::new()
            .name("api-reader".into())
            .spawn(move || {
                let reason = loop {
                    match read_frame(&mut reader, &mut bytes, None) {
                        Ok(frame) if frame.get("ev").is_some() => {
                            let bye = (frame["ev"] == "bye").then(|| {
                                format!(
                                    "bye:{}",
                                    frame["reason"].as_str().unwrap_or("invalid_message")
                                )
                            });
                            if events.try_send(frame).is_err() {
                                break "API event queue full or closed".into();
                            }
                            if let Some(reason) = bye {
                                break reason;
                            }
                        }
                        Ok(frame) => {
                            if let Some(id) = frame["id"].as_u64() {
                                if let Some(reply) = replies.lock().unwrap().remove(&id) {
                                    let _ = reply.send(Ok(frame));
                                }
                            } else {
                                break "invalid API reply id".into();
                            }
                        }
                        Err(error) => break format!("API connection closed: {error}"),
                    }
                };
                *ended.lock().unwrap() = Some(reason.clone());
                for (_, reply) in replies.lock().unwrap().drain() {
                    let _ = reply.try_send(Err(reason.clone()));
                }
                let _ = events.try_send(json!({"ev":"client.disconnected","message":reason}));
                let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
            })?;
        Ok(Self {
            writer: Mutex::new(writer),
            pending,
            events: Mutex::new(receiver),
            reader: Some(thread),
            hello,
            end_reason,
        })
    }
    fn request_frame(
        &self,
        id: u64,
        op: &str,
        args: Value,
        reference: Option<&str>,
        instance: &api::Instance,
        timeout: Duration,
    ) -> Result<Value> {
        if let Some(reason) = self.end_reason.lock().unwrap().clone() {
            return Err(Disconnected(reason).into());
        }
        let (reply, receive) = mpsc::sync_channel(1);
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= 8 {
                return Err(AppError::new(Code::Backend, "API in-flight limit reached").into());
            }
            pending.insert(id, reply);
        }
        let mut frame = json!({"id":id,"op":op,"args":args});
        if let Some(reference) = reference {
            frame["ref"] = json!(reference);
            frame["instance"] = json!(instance);
        }
        if let Err(error) = send(&mut self.writer.lock().unwrap(), &frame) {
            self.pending.lock().unwrap().remove(&id);
            return Err(Disconnected(error.to_string()).into());
        }
        let answer = receive.recv_timeout(timeout);
        self.pending.lock().unwrap().remove(&id);
        match answer {
            Ok(Ok(frame)) => Ok(frame),
            Ok(Err(error)) => Err(Disconnected(error).into()),
            Err(_) => Err(AppError::new(
                Code::Timeout,
                "API request timed out; outcome is unknown",
            )
            .hint("fbittorrent list --json")
            .into()),
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self
            .writer
            .get_mut()
            .unwrap()
            .shutdown(std::net::Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub struct Client {
    connection: Mutex<Arc<Connection>>,
    cfg: Config,
    agent: bool,
    name: Option<String>,
    path: PathBuf,
    next: AtomicU64,
    pub hello: HelloResult,
    reconnects: AtomicU64,
    reconnect_limit: AtomicU64,
}
impl Client {
    pub fn connect(cfg: &Config, agent: bool) -> Result<Self> {
        Self::connect_at(
            cfg,
            agent,
            &crate::config::runtime_dir().join("api.sock"),
        )
    }
    pub fn connect_named(cfg: &Config, name: &str) -> Result<Self> {
        Self::connect_at_named(cfg,true,&crate::config::runtime_dir().join("api.sock"),Some(name))
    }
    pub(super) fn connect_at(cfg: &Config, agent: bool, path: &std::path::Path) -> Result<Self> {
        Self::connect_at_named(cfg,agent,path,None)
    }
    fn connect_at_named(cfg: &Config, agent: bool, path: &std::path::Path, name: Option<&str>) -> Result<Self> {
        let connection = Connection::connect(cfg, agent, path, name)?;
        let hello = connection.hello.clone();
        Ok(Self {
            connection: Mutex::new(Arc::new(connection)),
            cfg: cfg.clone(),
            agent,
            name: name.map(str::to_owned),
            path: path.into(),
            next: AtomicU64::new(2),
            hello,
            reconnects: AtomicU64::new(0),
            reconnect_limit: AtomicU64::new(u64::MAX),
        })
    }
    pub fn limit_reconnects(&self, count: u64) {
        self.reconnect_limit.store(count, Ordering::Relaxed);
    }
    pub fn call(&self, op: &str, args: Value) -> Result<Value> {
        self.request(op, args, None)
    }
    pub fn request(&self, op: &str, args: Value, reference: Option<&str>) -> Result<Value> {
        self.execute(op,args,reference,false)
    }
    pub fn envelope(&self, op: &str, args: Value, reference: Option<&str>) -> Result<Value> {
        self.execute(op,args,reference,true)
    }
    pub fn hello_envelope(cfg: &Config, args: Value) -> Result<Value> {
        let deadline=Instant::now()+Duration::from_millis(200);
        let path=crate::config::runtime_dir().join("api.sock");
        let mut socket=connect_probe(&path,deadline)?;
        socket.set_write_timeout(Some(Duration::from_millis(200)))?;
        send(&mut socket,&json!({"id":1,"op":"hello","args":args}))?;
        read_frame(&mut BufReader::new(socket),&mut Vec::new(),Some(deadline))
    }
    fn execute(&self, op: &str, args: Value, reference: Option<&str>, envelope: bool) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let generated = crate::api::ops::TABLE
            .iter()
            .any(|entry| entry.name == op && entry.mutating)
            .then(|| {
                format!(
                    "cli-{}-{}",
                    std::process::id(),
                    NEXT_REF.fetch_add(1, Ordering::Relaxed)
                )
            });
        let reference = reference.or(generated.as_deref());
        let connection = self.connection.lock().unwrap().clone();
        let timeout = Duration::from_secs(if op == "torrents.add" {
            self.cfg.timeout_secs.max(95)
        } else if op=="torrents.move" { self.cfg.timeout_secs.max(65)
        } else {
            self.cfg.timeout_secs.max(1)
        });
        match connection.request_frame(
            id,
            op,
            args.clone(),
            reference,
            &connection.hello.instance,
            timeout,
        ) {
            Ok(value) => if envelope { Ok(value) } else { result(value) },
            Err(error) => {
                let Some(lost) = error.downcast_ref::<Disconnected>() else {
                    return Err(error);
                };
                if [
                    "bye:slow_client",
                    "bye:hello_required",
                    "bye:invalid_message",
                ]
                .iter()
                .any(|reason| lost.0.contains(reason))
                {
                    return Err(AppError::new(Code::Failure, lost.to_string()).into());
                }
                if !Self::idempotent(op) {
                    return Err(AppError::new(
                        Code::Timeout,
                        "the daemon stopped while this request was in flight; check the list",
                    )
                    .hint("fbittorrent list --json")
                    .into());
                }
                if self.reconnects.fetch_add(1, Ordering::Relaxed)
                    >= self.reconnect_limit.load(Ordering::Relaxed)
                {
                    return Err(AppError::new(Code::Backend, "API reconnect limit reached").into());
                }
                let mut active = self.connection.lock().unwrap();
                if Arc::ptr_eq(&active, &connection) {
                    let replacement = match Connection::connect(&self.cfg, self.agent, &self.path, self.name.as_deref()) {
                        Ok(v) => v,
                        Err(_) => {
                            super::ensure_daemon(
                                &self.cfg,
                                !self.cfg.autostart || crate::backend::start_prevented(),
                            )?;
                            Connection::connect(&self.cfg, self.agent, &self.path, self.name.as_deref())?
                        }
                    };
                    *active = Arc::new(replacement);
                }
                // Keep both ref and the previously observed instance. The server's frozen
                // mutation policy decides whether the idempotent resend can be applied.
                active
                    .request_frame(
                        self.next.fetch_add(1, Ordering::Relaxed),
                        op,
                        args,
                        reference,
                        &connection.hello.instance,
                        timeout,
                    )
                    .map_err(|error| {
                        AppError::new(
                            Code::Timeout,
                            format!("API reconnect failed; outcome is unknown: {error}"),
                        )
                        .hint("fbittorrent list --json")
                        .into()
                    }).and_then(|frame| if envelope { Ok(frame) } else { result(frame) })
            }
        }
    }
    pub fn event(&self, timeout: Duration) -> Result<Option<Value>> {
        let connection = self.connection.lock().unwrap().clone();
        let answer = connection.events.lock().unwrap().recv_timeout(timeout);
        match answer {
            Ok(value) => Ok(Some(value)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(AppError::new(Code::Backend, "API event stream closed").into()),
        }
    }
    pub fn subscribe(&self, args: Value) -> Result<Subscription<'_>> {
        self.call("subscribe", args.clone())?;
        let mut subscription = Subscription::new(self);
        subscription.args = Some(args);
        Ok(subscription)
    }
    fn reconnect_subscription(&self, args: Value) -> Result<()> {
        let began = Instant::now();
        let bound = Duration::from_secs(self.cfg.timeout_secs.max(1));
        for delay in Self::reconnect_delays() {
            let left = bound.saturating_sub(began.elapsed());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(delay.min(left));
            if let Ok(connection) = Connection::connect(&self.cfg, self.agent, &self.path, self.name.as_deref()) {
                *self.connection.lock().unwrap() = Arc::new(connection);
                self.call("subscribe", args)?;
                return Ok(());
            }
        }
        Err(
            AppError::new(Code::Backend, "API subscription reconnect timed out")
                .hint("fbittorrent up")
                .into(),
        )
    }
    pub fn unsubscribe(&self, sub: u64) -> Result<Value> {
        self.call("unsubscribe", json!({"sub":sub}))
    }
    pub fn idempotent(op: &str) -> bool {
        crate::api::ops::TABLE
            .iter()
            .find(|entry| entry.name == op)
            .is_some_and(|entry| !entry.mutating || entry.idempotent)
    }
    pub fn reconnect_delays() -> impl Iterator<Item = Duration> {
        [500, 1000, 2000, 4000, 8000]
            .into_iter()
            .chain(std::iter::repeat(10000))
            .map(Duration::from_millis)
    }
}
fn send(stream: &mut UnixStream, frame: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(frame)?;
    anyhow::ensure!(
        bytes.len() < api::LINE_IN,
        "API request exceeds line_in limit"
    );
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    Ok(())
}
fn read_frame(
    reader: &mut BufReader<UnixStream>,
    bytes: &mut Vec<u8>,
    deadline: Option<Instant>,
) -> Result<Value> {
    bytes.clear();
    loop {
        if let Some(deadline) = deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!left.is_zero(), "API hello timed out");
            reader.get_ref().set_read_timeout(Some(left))?;
        }
        let buffer = reader.fill_buf()?;
        anyhow::ensure!(!buffer.is_empty(), "API EOF");
        let end = buffer.iter().position(|b| *b == b'\n');
        let count = end.map(|n| n + 1).unwrap_or(buffer.len());
        anyhow::ensure!(
            bytes.len() + count <= api::LINE_OUT,
            "API response exceeds line_out limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(serde_json::from_slice(bytes)?);
        }
    }
}
pub fn result(frame: Value) -> Result<Value> {
    if frame["ok"] == true {
        return frame
            .get("result")
            .cloned()
            .context("API success omitted result");
    }
    let failure: api::Error =
        serde_json::from_value(frame["error"].clone()).context("invalid API failure")?;
    Err(crate::api::errors::app_error(failure))
}

/// Subscription iterator verifies revision continuity independently of rendering.
pub struct Subscription<'a> {
    pub client: &'a Client,
    revisions: api::validation::Revisions,
    args: Option<Value>,
    ended: bool,
}
impl<'a> Subscription<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self {
            client,
            revisions: api::validation::Revisions::default(),
            args: None,
            ended: false,
        }
    }
    pub fn accept(&mut self, frame: Value) -> Result<Value> {
        // Typed decode rejects malformed events before revision state changes.
        let _: api::Event = serde_json::from_value(frame.clone())?;
        if let (Some(sub), Some(_topic), Some(rev)) = (
            frame["sub"].as_u64(),
            frame["topic"].as_str(),
            frame["rev"].as_u64(),
        ) {
            self.revisions
                .observe(sub, rev)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Ok(frame)
    }
}
impl Iterator for Subscription<'_> {
    type Item = Result<Value>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }
        loop {
            match self.client.event(Duration::from_secs(60)) {
                Ok(Some(frame)) => {
                    let bye = frame["ev"] == "bye";
                    let reconnect = frame["ev"] == "client.disconnected"
                        || (bye && matches!(frame["reason"].as_str(), Some("restart" | "upgrade")));
                    if reconnect {
                        if let Some(args) = self.args.clone() {
                            match self.client.reconnect_subscription(args) {
                                Ok(()) => {
                                    self.revisions = api::validation::Revisions::default();
                                    continue;
                                }
                                Err(error) => {
                                    self.ended = true;
                                    return Some(Err(error));
                                }
                            }
                        }
                    }
                    if bye {
                        self.ended = true;
                    }
                    return Some(self.accept(frame));
                }
                Ok(None) => continue,
                Err(error) => {
                    self.ended = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
