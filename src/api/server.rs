//! Bounded, same-user NDJSON transport owned by the daemon, across slot restarts.
use super::{
    ops::{self, error, ApiResult},
    *,
};
use crate::{
    daemon::{lifecycle::StopReason, Daemon},
    ipc::InstanceLock,
};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const CONNECTIONS: usize = 32;
const INFLIGHT: usize = 8;
const DISPATCH: usize = 8;
const QUEUE: usize = 64;
const OUTGOING: usize = 512;
const HELLO: Duration = Duration::from_secs(5);

type Frame = Value;
fn response(id: Option<u64>, failure: Error) -> Frame {
    json!({"id":id,"ok":false,"error":failure})
}
fn bye(reason: ByeReason) -> Frame {
    json!({"ev":"bye","reason":reason})
}
fn add_timeout() -> Error {
    let mut failure = error(
        ErrorCode::Timeout,
        "URL add timed out; outcome is unknown",
        None,
    );
    failure.details = Some(serde_json::Map::from_iter([("after_s".into(), json!(95))]));
    failure
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn busy(limit: &str) -> Error {
    let mut failure = error(
        ErrorCode::Busy,
        "server limit reached; retry in a moment",
        None,
    );
    failure.details = Some(serde_json::Map::from_iter([("limit".into(), limit.into())]));
    failure
}
fn stopping() -> Error {
    let mut failure = error(ErrorCode::EngineUnavailable, "engine is stopping", None);
    failure.details = Some(serde_json::Map::from_iter([(
        "state".into(),
        "stopping".into(),
    )]));
    failure
}

/// Value's ordinary deserializer keeps the last duplicate key. Reject repeated
/// keys recursively before interpreting the strict typed request.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Unique, M::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, Unique>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!(
                            "duplicate key {}",
                            key.chars().take(80).collect::<String>()
                        )));
                    }
                    values.insert(key, value.0);
                }
                Ok(Unique(Value::Object(values)))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Unique, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                Ok(Unique(json!(value)))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
        }
        deserializer.deserialize_any(V)
    }
}
#[derive(Debug)]
struct Invalid {
    id: Option<u64>,
    error: Error,
    close: bool,
}

// The wire structs enforce semantic bounds too. Inspect field shapes first so
// a type error names its actual argument instead of serde's enclosing enum.
fn argument_types(raw: &Value) -> Option<String> {
    if raw
        .get("ref")
        .is_some_and(|v| serde_json::from_value::<MutationRef>(v.clone()).is_err())
    {
        return Some("ref".into());
    }
    if raw
        .get("instance")
        .is_some_and(|v| serde_json::from_value::<Instance>(v.clone()).is_err())
    {
        return Some("instance".into());
    }
    let Some(args) = raw.get("args") else {
        return None;
    };
    let Some(args) = args.as_object() else {
        return Some("args".into());
    };
    let op = raw["op"].as_str().unwrap_or("");
    for (key, value) in args {
        let valid = match key.as_str() {
            "interactive" | "glance" | "stage" | "stopped" | "seed" | "force" | "delete_files"
            | "on" => value.is_boolean(),
            "proto" | "file" | "sub" | "interval_ms" | "since_seq" => value.is_u64(),
            "bps" => value.is_u64() || (op == "torrents.set_upload_limit" && value.is_null()),
            "hash" => serde_json::from_value::<Hash>(value.clone()).is_ok(),
            "hashes" => serde_json::from_value::<Vec<Hash>>(value.clone()).is_ok(),
            "topics" => serde_json::from_value::<Vec<Topic>>(value.clone()).is_ok(),
            "states" => serde_json::from_value::<Vec<TorrentState>>(value.clone()).is_ok(),
            "files" | "include" | "skip" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_u64)),
            "origin" if op == "torrents.list" => value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string)),
            "reason" | "save_path" | "category" | "origin" => value.is_string(),
            "scope" => serde_json::from_value::<RestartScope>(value.clone()).is_ok(),
            "source" => serde_json::from_value::<AddSource>(value.clone()).is_ok(),
            "client" => {
                let Some(client) = value.as_object() else {
                    return Some("client".into());
                };
                for (key, value) in client {
                    let valid = match key.as_str() {
                        "kind" => serde_json::from_value::<ClientKind>(value.clone()).is_ok(),
                        "name" => serde_json::from_value::<ClientName>(value.clone()).is_ok(),
                        "version" => value.is_string(),
                        _ => true,
                    };
                    if !valid {
                        return Some(format!("client.{key}"));
                    }
                }
                true
            }
            _ => true, // Unknown fields are rejected by the strict wire DTO.
        };
        if !valid {
            return Some(key.clone());
        }
    }
    None
}

fn decode(bytes: &[u8]) -> Result<Request, Invalid> {
    let malformed = |message: &str| Invalid {
        id: None,
        error: error(ErrorCode::InvalidMessage, message, None),
        close: true,
    };
    if bytes.len() > LINE_IN {
        return Err(malformed("input line exceeds 65536 bytes"));
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let raw =
        match Unique::deserialize(&mut decoder).and_then(|value| decoder.end().map(|_| value.0)) {
            Ok(value) => value,
            Err(e) if e.to_string().contains("duplicate key") => {
                return Err(Invalid {
                    id: None,
                    error: error(ErrorCode::InvalidArgs, "duplicate object key", None),
                    close: false,
                })
            }
            Err(_) => return Err(malformed("expected one UTF-8 JSON object")),
        };
    let Some(object) = raw.as_object() else {
        return Err(malformed("expected a JSON object"));
    };
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "id" | "op" | "args" | "ref" | "instance"))
    {
        return Err(Invalid {
            id: None,
            error: error(
                ErrorCode::InvalidArgs,
                "unknown top-level field",
                Some(&key.chars().take(80).collect::<String>()),
            ),
            close: false,
        });
    }
    let Some(id) = raw.get("id").and_then(Value::as_u64).filter(|id| *id > 0) else {
        return Err(Invalid {
            id: None,
            error: error(
                ErrorCode::InvalidArgs,
                "id must be a positive integer",
                Some("id"),
            ),
            close: false,
        });
    };
    if !raw.get("op").is_some_and(Value::is_string) {
        return Err(Invalid {
            id: Some(id),
            error: error(ErrorCode::InvalidArgs, "op must be a string", Some("op")),
            close: false,
        });
    }
    if let Some(op) = raw.get("op").and_then(Value::as_str) {
        if !implemented(op) {
            let mut failure = error(
                ErrorCode::UnknownOp,
                &format!(
                    "unknown operation {:?}",
                    op.chars().take(80).collect::<String>()
                ),
                None,
            );
            let nearest: Vec<_> = ops::TABLE
                .iter()
                .filter(|row| implemented(row.name) && distance(op, row.name) <= 2)
                .take(3)
                .map(|row| row.name)
                .collect();
            failure.details = Some(serde_json::Map::from_iter([(
                "nearest".into(),
                json!(nearest),
            )]));
            failure.hint = Some(if nearest.is_empty() {
                "no supported operation within two edits; inspect hello capabilities".into()
            } else {
                format!("try {}", nearest.join(", "))
            });
            return Err(Invalid {
                id: Some(id),
                error: failure,
                close: false,
            });
        }
    }
    if let Some(key) = argument_types(&raw) {
        return Err(Invalid {
            id: Some(id),
            error: error(
                ErrorCode::InvalidArgs,
                "invalid argument type or value",
                Some(&key),
            ),
            close: false,
        });
    }
    serde_json::from_value(raw).map_err(|e| {
        let text = e.to_string();
        let key = text
            .split('`')
            .nth(1)
            .or_else(|| {
                ["topics", "hashes", "bps", "since_seq"]
                    .into_iter()
                    .find(|key| text.starts_with(&format!("{key}:")))
            })
            .unwrap_or("args");
        Invalid {
            id: Some(id),
            error: error(
                ErrorCode::InvalidArgs,
                &text.chars().take(80).collect::<String>(),
                Some(key),
            ),
            close: false,
        }
    })
}
// TABLE remains the frozen full contract. Availability follows the phase's
// negotiated capabilities; P1-08 installs subscription routing.
fn implemented(op: &str) -> bool {
    !matches!(op, "subscribe" | "unsubscribe") && ops::TABLE.iter().any(|row| row.name == op)
}
fn distance(a: &str, b: &str) -> usize {
    if a.len() > 128 {
        return usize::MAX;
    }
    let mut row: Vec<_> = (0..=b.chars().count()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut prior = row[0];
        row[0] = i + 1;
        for (j, y) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(prior + usize::from(x != y));
            prior = old;
        }
    }
    *row.last().unwrap()
}
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::other("invalid peer credentials"));
    }
    Ok(credentials.uid)
}
fn allowed_uid(peer: u32, owner: u32) -> bool {
    peer == owner
}

struct Background {
    deadline: Instant,
    reference: MutationRef,
}
struct Pending {
    generation: u64,
    deadline: Instant,
    after: u64,
}
#[derive(Default)]
struct Outbox {
    frames: VecDeque<Frame>,
    terminal: Option<Frame>,
    ending: bool,
}
#[derive(Default)]
struct ClientState {
    client: Option<Client>,
    interactive: bool,
    glance: bool,
    caps: validation::NegotiatedCaps,
}
struct Connection {
    number: u64,
    stream: UnixStream,
    accepted: Instant,
    closed: AtomicBool,
    read_done: AtomicBool,
    client: Mutex<ClientState>,
    pending: Mutex<BTreeMap<u64, Pending>>,
    background: Mutex<BTreeMap<u64, Background>>,
    sequence: AtomicU64,
    out: Mutex<Outbox>,
    ready: Condvar,
    emission: Mutex<()>,
}
impl Connection {
    fn send(&self, frame: Frame) {
        let mut out = self.out.lock().unwrap();
        if out.ending {
            return;
        }
        if out.frames.len() >= OUTGOING {
            out.frames.clear();
            out.ending = true;
            out.terminal = Some(bye(ByeReason::SlowClient));
            let _ = self.stream.shutdown(Shutdown::Read);
        } else {
            out.frames.push_back(frame);
        }
        self.ready.notify_one();
    }
    fn finish(&self, reason: ByeReason) {
        let mut out = self.out.lock().unwrap();
        if !out.ending {
            out.ending = true;
            out.terminal = Some(bye(reason));
        }
        self.ready.notify_one();
    }
    fn protocol(&self, failure: Error, reason: ByeReason) {
        self.send(json!({"ev":"error","error":failure}));
        self.finish(reason);
    }
    fn reserve(&self, request: &Request) -> Result<u64, Invalid> {
        self.expire();
        let id = request.id.get();
        let mut pending = self.pending.lock().unwrap();
        if pending.contains_key(&id) {
            return Err(Invalid {
                id: None,
                error: error(
                    ErrorCode::InvalidArgs,
                    "id is already in flight",
                    Some("id"),
                ),
                close: false,
            });
        }
        if pending.len() >= INFLIGHT {
            return Err(Invalid {
                id: Some(id),
                error: busy("inflight"),
                close: false,
            });
        }
        let generation = self.sequence.fetch_add(1, Ordering::Relaxed);
        let after = if matches!(request.call, Call::TorrentsAdd { .. }) {
            95
        } else {
            15
        };
        pending.insert(
            id,
            Pending {
                generation,
                deadline: Instant::now() + Duration::from_secs(after),
                after,
            },
        );
        Ok(generation)
    }
    fn complete(&self, id: u64, generation: u64, result: ApiResult<Reply>) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if !pending
            .get(&id)
            .is_some_and(|entry| entry.generation == generation)
        {
            return false;
        }
        let entry = pending.remove(&id).unwrap();
        drop(pending);
        let timely = entry.deadline > Instant::now();
        let result = if !timely {
            let mut failure = error(
                ErrorCode::Timeout,
                "operation timed out; outcome is unknown",
                None,
            );
            failure.details = Some(serde_json::Map::from_iter([(
                "after_s".into(),
                json!(entry.after),
            )]));
            Err(failure)
        } else {
            result
        };
        let frame = match result {
            Ok(result) => json!({"id":id,"ok":true,"result":result}),
            Err(failure) => response(Some(id), failure),
        };
        // A collection that cannot fit is a request failure, not malformed
        // client input. Keep its ID and the connection usable (P1-06 ruling 2).
        let frame = if encode_frame(&frame).is_err() {
            let mut failure = error(
                ErrorCode::InvalidMessage,
                "response exceeds the 1048576-byte frame cap",
                None,
            );
            failure.hint =
                Some("request a smaller collection; chunked subscriptions follow in P1-08".into());
            response(Some(id), failure)
        } else {
            frame
        };
        self.send(frame);
        timely
    }
    fn expire(&self) {
        let mut pending = self.pending.lock().unwrap();
        let expired: Vec<_> = pending
            .iter()
            .filter(|(_, entry)| entry.deadline <= Instant::now())
            .map(|(id, entry)| (*id, entry.after))
            .collect();
        for (id, after) in expired {
            pending.remove(&id);
            let mut failure = error(
                ErrorCode::Timeout,
                "operation timed out; outcome is unknown",
                None,
            );
            failure.details = Some(serde_json::Map::from_iter([(
                "after_s".into(),
                json!(after),
            )]));
            self.send(response(Some(id), failure));
        }
    }
}
struct Job {
    connection: Arc<Connection>,
    request: Request,
    generation: u64,
}
#[derive(Default)]
struct Work {
    jobs: VecDeque<Job>,
    closed: bool,
}
#[derive(Default)]
struct Pool {
    work: Mutex<Work>,
    changed: Condvar,
}
impl Pool {
    fn submit(&self, job: Job) -> Result<(), Job> {
        let mut work = self.work.lock().unwrap();
        if work.closed || work.jobs.len() >= QUEUE {
            return Err(job);
        }
        work.jobs.push_back(job);
        self.changed.notify_one();
        Ok(())
    }
    fn next(&self) -> Option<Job> {
        let mut work = self.work.lock().unwrap();
        while work.jobs.is_empty() && !work.closed {
            work = self.changed.wait(work).unwrap();
        }
        work.jobs.pop_front()
    }
    fn close(&self) {
        self.work.lock().unwrap().closed = true;
        self.changed.notify_all();
    }
}
struct Shared {
    daemon: Arc<Daemon>,
    pool: Pool,
    clients: Mutex<HashMap<u64, Arc<Connection>>>,
    counts: Mutex<ClientCounts>,
    began: Instant,
    stopping: AtomicBool,
    event_sequence: AtomicU64,
}
impl Shared {
    fn url_outcome(
        &self,
        connection: &Connection,
        reference: MutationRef,
        result: ApiResult<Reply>,
    ) {
        let (outcome, error, result) = match result {
            Ok(reply) => (
                MutationOutcome::Done,
                None,
                Some(reply.to_value().unwrap().as_object().unwrap().clone()),
            ),
            Err(error) => (
                if error.code == ErrorCode::Timeout {
                    MutationOutcome::Timeout
                } else {
                    MutationOutcome::Failed
                },
                Some(error),
                None,
            ),
        };
        connection.send(
            serde_json::to_value(Event::MutationResult {
                seq: self.event_sequence.fetch_add(1, Ordering::Relaxed),
                ts: now(),
                reference,
                op: "torrents.add".into(),
                outcome,
                error,
                result,
            })
            .unwrap(),
        );
    }
    fn expire_background(&self, connection: &Connection) {
        let mut background = connection.background.lock().unwrap();
        let expired: Vec<_> = background
            .iter()
            .filter(|(_, task)| task.deadline <= Instant::now())
            .map(|(generation, _)| *generation)
            .collect();
        for generation in expired {
            let task = background.remove(&generation).unwrap();
            self.url_outcome(connection, task.reference, Err(add_timeout()));
        }
    }
    fn dispatch(&self, job: Job) {
        if !job
            .connection
            .pending
            .lock()
            .unwrap()
            .get(&job.request.id.get())
            .is_some_and(|p| p.generation == job.generation)
        {
            return;
        }
        let reference = job
            .request
            .reference
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("api-{}-{}", job.connection.number, job.generation));
        if !self.daemon.is_stopping() {
            if let Call::TorrentsAdd { args } = &job.request.call {
                if matches!(args.source, AddSource::Url(_)) {
                    if let Err(failure) = ops::torrents::validate_url_add(args) {
                        job.connection
                            .complete(job.request.id.get(), job.generation, Err(failure));
                        return;
                    }
                    let Some(deadline) = job
                        .connection
                        .pending
                        .lock()
                        .unwrap()
                        .get(&job.request.id.get())
                        .filter(|entry| entry.generation == job.generation)
                        .map(|entry| entry.deadline)
                    else {
                        return;
                    };
                    let reference = MutationRef::new(reference).unwrap();
                    if !job.connection.complete(
                        job.request.id.get(),
                        job.generation,
                        Ok(Reply::TorrentsAdd(AddResult {
                            status: MutationResult::Pending {
                                reference: reference.clone(),
                            },
                            hash: None,
                            name: None,
                            duplicate: None,
                            deferred: None,
                        })),
                    ) {
                        return;
                    }
                    job.connection.background.lock().unwrap().insert(
                        job.generation,
                        Background {
                            deadline,
                            reference,
                        },
                    );
                    let result = ops::torrents::dispatch(&self.daemon, &job.request.call);
                    if let Some(task) = job
                        .connection
                        .background
                        .lock()
                        .unwrap()
                        .remove(&job.generation)
                    {
                        let result = if task.deadline <= Instant::now() {
                            Err(add_timeout())
                        } else {
                            result
                        };
                        self.url_outcome(&job.connection, task.reference, result);
                    }
                    return;
                }
            }
        }
        let result = if self.daemon.is_stopping() {
            Err(stopping())
        } else {
            match &job.request.call {
                Call::AppInfo { .. }
                | Call::AppNetwork { .. }
                | Call::AppShutdown { .. }
                | Call::AppRestart { .. } => ops::app::dispatch(
                    &self.daemon,
                    &job.request.call,
                    &reference,
                    serde_json::to_value(&*self.counts.lock().unwrap()).unwrap(),
                    self.began.elapsed().as_secs(),
                ),
                Call::TransferInfo { .. } => ops::transfer::info(&self.daemon),
                _ => ops::torrents::dispatch(&self.daemon, &job.request.call),
            }
        };
        job.connection
            .complete(job.request.id.get(), job.generation, result);
    }
}
pub struct Server {
    path: PathBuf,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    dispatch: Vec<JoinHandle<()>>,
    _lock: Arc<InstanceLock>,
}
impl Server {
    pub(crate) fn start(
        directory: &Path,
        lock: Arc<InstanceLock>,
        daemon: Arc<Daemon>,
    ) -> anyhow::Result<Self> {
        let path = directory.join("api.sock");
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            daemon,
            pool: Pool::default(),
            clients: Mutex::new(HashMap::new()),
            counts: Mutex::new(ClientCounts {
                cli: 0,
                agent: 0,
                tui: 0,
                bar: 0,
                test: 0,
            }),
            began: Instant::now(),
            stopping: AtomicBool::new(false),
            event_sequence: AtomicU64::new(1),
        });
        let connections = Arc::new(Mutex::new(Vec::new()));
        // Own partially-created workers too: an OS thread-spawn failure must
        // close the pool and join everything already started.
        let mut server = Self {
            path,
            shared: shared.clone(),
            accept: None,
            connections: connections.clone(),
            dispatch: Vec::new(),
            _lock: lock,
        };
        for index in 0..DISPATCH {
            let state = shared.clone();
            server.dispatch.push(
                thread::Builder::new()
                    .name(format!("fbt-api-dispatch-{index}"))
                    .spawn(move || {
                        while let Some(job) = state.pool.next() {
                            state.dispatch(job);
                        }
                    })?,
            );
        }
        let workers = connections.clone();
        let state = shared.clone();
        let accept = thread::Builder::new()
            .name("fbt-api-accept".into())
            .spawn(move || {
                let mut sequence = 0;
                while !state.stopping.load(Ordering::Acquire) {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(e) => {
                            eprintln!("WARN api accept: {e}");
                            continue;
                        }
                    };
                    let accepted = Instant::now();
                    match peer_uid(&stream) {
                        Ok(uid) if allowed_uid(uid, unsafe { libc::geteuid() }) => {}
                        observed => {
                            eprintln!(
                                "WARN api foreign or unavailable peer credential: {observed:?}"
                            );
                            continue;
                        }
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    if state.clients.lock().unwrap().len() >= CONNECTIONS {
                        let mut stream = stream;
                        let _ = write_frame(
                            &mut stream,
                            &json!({"ev":"error","error":busy("connections")}),
                        );
                        let _ = write_frame(&mut stream, &bye(ByeReason::InvalidMessage));
                        continue;
                    }
                    sequence += 1;
                    let Ok(reader) = stream.try_clone() else {
                        continue;
                    };
                    let Ok(writer) = stream.try_clone() else {
                        continue;
                    };
                    let connection = Arc::new(Connection {
                        number: sequence,
                        stream,
                        accepted,
                        closed: AtomicBool::new(false),
                        read_done: AtomicBool::new(false),
                        client: Mutex::new(ClientState::default()),
                        pending: Mutex::new(BTreeMap::new()),
                        background: Mutex::new(BTreeMap::new()),
                        sequence: AtomicU64::new(1),
                        out: Mutex::new(Outbox::default()),
                        ready: Condvar::new(),
                        emission: Mutex::new(()),
                    });
                    state
                        .clients
                        .lock()
                        .unwrap()
                        .insert(sequence, connection.clone());
                    let c = connection.clone();
                    let s = state.clone();
                    let reading = thread::Builder::new()
                        .name("fbt-api-reader".into())
                        .spawn(move || read_client(s, c, reader));
                    let c = connection.clone();
                    let s = state.clone();
                    let writing = thread::Builder::new()
                        .name("fbt-api-writer".into())
                        .spawn(move || write_client(s, c, writer));
                    let mut threads = workers.lock().unwrap();
                    for result in [reading, writing] {
                        match result {
                            Ok(thread) => threads.push(thread),
                            Err(e) => {
                                eprintln!("WARN api thread: {e}");
                                connection.closed.store(true, Ordering::Release);
                                connection.ready.notify_one();
                                let _ = connection.stream.shutdown(Shutdown::Both);
                                state.clients.lock().unwrap().remove(&connection.number);
                            }
                        }
                    }
                    let mut index = 0;
                    while index < threads.len() {
                        if threads[index].is_finished() {
                            let _ = threads.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                }
            })?;
        server.accept = Some(accept);
        Ok(server)
    }
    /// P1-08 owns subscriptions; these handles are its bounded event transport.
    pub(crate) fn event_sinks(&self) -> Vec<EventSink> {
        self.shared
            .clients
            .lock()
            .unwrap()
            .values()
            .filter(|connection| connection.client.lock().unwrap().client.is_some())
            .map(|connection| EventSink(connection.clone()))
            .collect()
    }

    pub fn stop_accepting(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.accept.take() {
            let _ = thread.join();
        }
    }
    pub fn drain(&mut self, reason: StopReason) {
        self.stop_accepting();
        self.shared.pool.close();
        for thread in self.dispatch.drain(..) {
            let _ = thread.join();
        }
        let reason = match reason {
            StopReason::Idle => ByeReason::Idle,
            StopReason::Upgrade => ByeReason::Upgrade,
            _ => ByeReason::Shutdown,
        };
        for client in self.shared.clients.lock().unwrap().values() {
            let mut pending = client.pending.lock().unwrap();
            for id in pending.keys() {
                client.send(response(Some(*id), stopping()));
            }
            pending.clear();
            drop(pending);
            client.finish(reason);
        }
        for thread in self.connections.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.drain(StopReason::Quit);
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_client(shared: Arc<Shared>, connection: Arc<Connection>, stream: UnixStream) {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    let mut staged: Vec<Job> = Vec::new();
    loop {
        if !reader.buffer().contains(&b'\n') {
            for job in staged.drain(..) {
                if let Err(job) = shared.pool.submit(job) {
                    job.connection.complete(
                        job.request.id.get(),
                        job.generation,
                        Err(if shared.stopping.load(Ordering::Acquire) {
                            stopping()
                        } else {
                            busy("dispatch")
                        }),
                    );
                }
            }
        }
        if connection.closed.load(Ordering::Acquire) {
            break;
        }
        if connection.client.lock().unwrap().client.is_none()
            && connection.accepted.elapsed() >= HELLO
        {
            connection.protocol(
                error(
                    ErrorCode::HelloRequired,
                    "hello required within five seconds of accept",
                    None,
                ),
                ByeReason::HelloRequired,
            );
            return;
        }
        let available = match reader.fill_buf() {
            Ok([]) => {
                if !bytes.is_empty() {
                    connection.protocol(
                        error(ErrorCode::InvalidMessage, "incomplete JSON line", None),
                        ByeReason::InvalidMessage,
                    );
                    return;
                }
                break;
            }
            Ok(bytes) => bytes,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => break,
        };
        let length = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(available.len());
        if bytes.len() + length > LINE_IN {
            connection.protocol(
                error(
                    ErrorCode::InvalidMessage,
                    "input line exceeds 65536 bytes",
                    None,
                ),
                ByeReason::InvalidMessage,
            );
            return;
        }
        bytes.extend_from_slice(&available[..length]);
        reader.consume(length);
        if bytes.last() != Some(&b'\n') {
            continue;
        }
        let decoded = decode(&bytes);
        bytes.clear();
        let request = match decoded {
            Ok(request) => request,
            Err(failure) => {
                if failure.close {
                    connection.protocol(failure.error, ByeReason::InvalidMessage);
                    return;
                }
                if failure.error.code == ErrorCode::UnknownOp
                    && connection.client.lock().unwrap().client.is_none()
                {
                    connection.protocol(
                        error(
                            ErrorCode::HelloRequired,
                            "first request must be hello",
                            None,
                        ),
                        ByeReason::HelloRequired,
                    );
                    return;
                }
                connection.send(response(failure.id, failure.error));
                continue;
            }
        };
        let hello_done = connection.client.lock().unwrap().client.is_some();
        if !hello_done {
            if connection.accepted.elapsed() >= HELLO {
                connection.protocol(
                    error(ErrorCode::HelloRequired, "hello deadline expired", None),
                    ByeReason::HelloRequired,
                );
                return;
            }
            let Call::Hello { args } = &request.call else {
                connection.protocol(
                    error(
                        ErrorCode::HelloRequired,
                        "first request must be hello",
                        None,
                    ),
                    ByeReason::HelloRequired,
                );
                return;
            };
            if args.proto != PROTO {
                connection.send(response(
                    Some(request.id.get()),
                    error(
                        ErrorCode::ProtoUnsupported,
                        "server supports protocol 1",
                        Some("proto"),
                    ),
                ));
                connection.finish(ByeReason::InvalidMessage);
                return;
            }
            let mut counts = shared.counts.lock().unwrap();
            match args.client.kind {
                ClientKind::Cli => counts.cli += 1,
                ClientKind::Agent => counts.agent += 1,
                ClientKind::Tui => counts.tui += 1,
                ClientKind::Bar => counts.bar += 1,
                ClientKind::Test => counts.test += 1,
            }
            drop(counts);
            let hello = ops::app::hello(&shared.daemon);
            *connection.client.lock().unwrap() = ClientState {
                client: Some(args.client.clone()),
                interactive: args.interactive.unwrap_or(false),
                glance: false,
                caps: validation::NegotiatedCaps::from_hello(&hello),
            };
            connection.send(json!({"id":request.id,"ok":true,"result":hello}));
            continue;
        }
        if matches!(request.call, Call::Hello { .. }) {
            connection.send(response(
                Some(request.id.get()),
                error(ErrorCode::InvalidState, "hello already completed", None),
            ));
            continue;
        }
        let generation = match connection.reserve(&request) {
            Ok(generation) => generation,
            Err(e) => {
                connection.send(response(e.id, e.error));
                continue;
            }
        };
        if let Call::AppClient { args } = &request.call {
            if shared.daemon.is_stopping() {
                connection.complete(request.id.get(), generation, Err(stopping()));
                continue;
            }
            let mut client = connection.client.lock().unwrap();
            if let Some(value) = args.interactive {
                client.interactive = value;
            }
            if let Some(value) = args.glance {
                client.glance = value;
            }
            drop(client);
            connection.complete(
                request.id.get(),
                generation,
                if shared.daemon.is_stopping() {
                    Err(stopping())
                } else {
                    Ok(Reply::AppClient(EmptyResult {}))
                },
            );
            continue;
        }
        let control = matches!(
            request.call,
            Call::AppShutdown { .. } | Call::AppRestart { .. }
        );
        let job = Job {
            connection: connection.clone(),
            request,
            generation,
        };
        // Owner control remains responsive even when all dispatch workers are
        // waiting for upstream work. The owner still performs the drain.
        if control {
            shared.dispatch(job);
            continue;
        }
        staged.push(job);
    }
    // A half-closed reader may still be waiting for its accepted replies. The
    // writer closes after those replies drain (or their deadlines expire).
    connection.read_done.store(true, Ordering::Release);
    connection.ready.notify_one();
}
fn write_frame(stream: &mut UnixStream, frame: &Frame) -> io::Result<()> {
    stream.write_all(&encode_frame(frame)?)
}
fn encode_frame(frame: &Frame) -> io::Result<Vec<u8>> {
    struct Capped(Vec<u8>);
    impl Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len() + bytes.len() + 1 > LINE_OUT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "output frame exceeds 1 MiB",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Capped(Vec::new());
    serde_json::to_writer(&mut encoded, frame)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    encoded.0.push(b'\n');
    Ok(encoded.0)
}
fn write_client(shared: Arc<Shared>, connection: Arc<Connection>, mut stream: UnixStream) {
    let mut last = Instant::now();
    loop {
        connection.expire();
        shared.expire_background(&connection);
        let disconnected = connection.read_done.load(Ordering::Acquire)
            && connection.pending.lock().unwrap().is_empty()
            && connection.background.lock().unwrap().is_empty();
        let mut out = connection.out.lock().unwrap();
        let (frame, terminal) = if let Some(frame) = out.frames.pop_front() {
            (frame, false)
        } else if let Some(frame) = out.terminal.take() {
            (frame, true)
        } else if connection.closed.load(Ordering::Acquire) || disconnected {
            break;
        } else if last.elapsed() >= Duration::from_secs(15)
            && connection.client.lock().unwrap().client.is_some()
        {
            (json!({"ev":"heartbeat","ts":now()}), false)
        } else {
            let _guard = connection
                .ready
                .wait_timeout(out, Duration::from_millis(25))
                .unwrap();
            continue;
        };
        drop(out);
        match write_frame(&mut stream, &frame) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let _ = write_frame(
                    &mut stream,
                    &json!({"ev":"error","error":error(ErrorCode::InvalidMessage,"output frame exceeds 1 MiB; use chunked subscriptions",None)}),
                );
                let _ = write_frame(&mut stream, &bye(ByeReason::InvalidMessage));
                break;
            }
            Err(_) => {
                // The peer may already be gone or its kernel buffer full. Try
                // once without adding another blocking write deadline.
                let _ = stream.set_nonblocking(true);
                let _ = write_frame(&mut stream, &bye(ByeReason::SlowClient));
                break;
            }
        }
        last = Instant::now();
        if terminal {
            break;
        }
    }
    connection.closed.store(true, Ordering::Release);
    *connection.client.lock().unwrap() = ClientState::default();
    connection.pending.lock().unwrap().clear();
    connection.background.lock().unwrap().clear();
    let _ = stream.shutdown(Shutdown::Both);
    shared.clients.lock().unwrap().remove(&connection.number);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hello() -> Vec<u8> {
        br#"{"id":1,"op":"hello","args":{"proto":1,"client":{"name":"test","version":"0","kind":"test"}}}"#.to_vec()
    }
    #[test]
    fn peercred_rejects_other_uid() {
        let (socket, _) = UnixStream::pair().unwrap();
        let owner = unsafe { libc::geteuid() };
        assert_eq!(peer_uid(&socket).unwrap(), owner);
        assert!(allowed_uid(owner, owner));
        assert!(!allowed_uid(owner.wrapping_add(1), owner));
        println!(
            "peer credentials: kernel same uid={owner} accepted; injected foreign uid={} refused",
            owner.wrapping_add(1)
        );
    }
    #[test]
    fn garbage_table() {
        use ErrorCode::*;
        let table = vec![
            ("invalid UTF-8", vec![0xff, b'\n'], InvalidMessage, true),
            ("empty line", b"\n".to_vec(), InvalidMessage, true),
            ("65537 bytes", vec![b' '; 65_537], InvalidMessage, true),
            (
                "10000-deep nesting",
                ("[".repeat(10_000) + &"]".repeat(10_000) + "\n").into_bytes(),
                InvalidMessage,
                true,
            ),
            ("non-object", b"[]\n".to_vec(), InvalidMessage, true),
            (
                "duplicate keys",
                br#"{"id":1,"id":2,"op":"app.info"}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "nested duplicate keys",
                br#"{"id":1,"op":"app.client","args":{"glance":true,"glance":false}}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "id zero",
                br#"{"id":0,"op":"app.info"}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "negative id",
                br#"{"id":-1,"op":"app.info"}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "fractional id",
                br#"{"id":1.5,"op":"app.info"}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "string id",
                br#"{"id":"1","op":"app.info"}"#.to_vec(),
                InvalidArgs,
                false,
            ),
            (
                "unknown top-level key",
                br#"{"id":1,"op":"app.info","other":true}"#.to_vec(),
                InvalidArgs,
                false,
            ),
        ];
        for (name, input, code, close) in table {
            let failure = decode(&input).unwrap_err();
            println!(
                "api garbage {name}: code={:?}; closes={}; id={:?}",
                failure.error.code, failure.close, failure.id
            );
            assert_eq!(failure.error.code, code, "{name}");
            assert_eq!(failure.close, close, "{name}");
        }
        let mut crlf = hello();
        crlf.extend_from_slice(b"\r\n");
        assert!(matches!(decode(&crlf).unwrap().call, Call::Hello { .. }));
        println!("api garbage CRLF hello: accepted");
        let error = decode(br#"{"id":1,"op":"torrents.lis"}"#)
            .unwrap_err()
            .error;
        assert_eq!(error.code, UnknownOp);
        assert!(error.details.unwrap()["nearest"]
            .as_array()
            .unwrap()
            .contains(&json!("torrents.list")));
    }
    pub(super) fn connection() -> (Connection, UnixStream) {
        let (stream, peer) = UnixStream::pair().unwrap();
        (
            Connection {
                number: 1,
                stream,
                accepted: Instant::now(),
                closed: AtomicBool::new(false),
                read_done: AtomicBool::new(false),
                client: Mutex::new(ClientState::default()),
                pending: Mutex::new(BTreeMap::new()),
                background: Mutex::new(BTreeMap::new()),
                sequence: AtomicU64::new(1),
                out: Mutex::new(Outbox::default()),
                ready: Condvar::new(),
                emission: Mutex::new(()),
            },
            peer,
        )
    }
    #[test]
    fn timed_out_id_is_reusable_and_late_result_is_dropped() {
        let (connection, _) = connection();
        let request = decode(br#"{"id":7,"op":"app.info"}"#).unwrap();
        let first = connection.reserve(&request).unwrap();
        connection
            .pending
            .lock()
            .unwrap()
            .get_mut(&7)
            .unwrap()
            .deadline = Instant::now();
        connection.expire();
        assert!(connection.pending.lock().unwrap().is_empty());
        let second = connection.reserve(&request).unwrap();
        assert_ne!(first, second);
        connection.complete(7, first, Err(stopping()));
        assert_eq!(connection.out.lock().unwrap().frames.len(), 1);
        connection.complete(7, second, Ok(Reply::AppClient(EmptyResult {})));
        let frames = &connection.out.lock().unwrap().frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["error"]["code"], "timeout");
        assert_eq!(frames[1]["ok"], true);
        println!("timeout released id=7; generation {first} late result dropped; generation {second} delivered");
    }
    #[test]
    fn inflight_and_outgoing_limits_are_bounded() {
        let (connection, _) = connection();
        for id in 1..=8 {
            connection
                .reserve(
                    &decode(format!("{{\"id\":{id},\"op\":\"app.info\"}}").as_bytes()).unwrap(),
                )
                .unwrap();
        }
        let failure = connection
            .reserve(&decode(br#"{"id":9,"op":"app.info"}"#).unwrap())
            .unwrap_err();
        assert_eq!(failure.error.details.unwrap()["limit"], "inflight");
        let reused = connection
            .reserve(&decode(br#"{"id":1,"op":"app.info"}"#).unwrap())
            .unwrap_err();
        assert_eq!(reused.id, None);
        assert_eq!(reused.error.code, ErrorCode::InvalidArgs);
        for _ in 0..=512 {
            connection.send(json!({"ev":"heartbeat","ts":1}));
        }
        let out = connection.out.lock().unwrap();
        assert!(out.frames.is_empty());
        assert!(out.ending);
        assert_eq!(out.terminal.as_ref().unwrap()["reason"], "slow_client");
        println!("inflight=8; ninth busy; reused id invalid_args; outgoing=512 overflow ends with slow_client");
    }
    #[test]
    fn output_cap_is_checked_before_writing_bytes() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let result = write_frame(&mut stream, &json!({"result":"x".repeat(LINE_OUT)}));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        peer.set_nonblocking(true).unwrap();
        assert_eq!(
            peer.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        println!("oversized output rejected before any socket bytes; hard cap includes newline");
    }
    #[test]
    fn file_sort_key_is_natural_and_preserves_digit_runs() {
        let key = ops::torrents::sort_key;
        assert!(key("Dir/E2.mkv") < key("dir/e10.mkv"));
        assert_eq!(key("a12b3"), "a0000000012b0000000003");
        assert_eq!(key("a12345678901"), "a12345678901");
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn dispatch_queue_is_64_and_closed_pool_refuses_work() {
        let pool = Pool::default();
        let (connection, _) = super::tests::connection();
        let connection = Arc::new(connection);
        let job = || Job {
            connection: connection.clone(),
            request: decode(br#"{"id":1,"op":"app.info"}"#).unwrap(),
            generation: 1,
        };
        for _ in 0..64 {
            assert!(pool.submit(job()).is_ok());
        }
        assert!(pool.submit(job()).is_err());
        assert!(pool.next().is_some());
        assert!(pool.submit(job()).is_ok());
        pool.close();
        assert!(pool.submit(job()).is_err());
        let mut remaining = 0;
        while pool.next().is_some() {
            remaining += 1;
        }
        assert_eq!(remaining, 64);
        println!("dispatch queue: 64 admitted; 65th refused; consuming frees capacity; close drains accepted work only");
    }
    #[test]
    fn completion_after_deadline_is_timeout_without_writer_poll() {
        let (connection, _) = super::tests::connection();
        let request = decode(br#"{"id":1,"op":"app.info"}"#).unwrap();
        let generation = connection.reserve(&request).unwrap();
        connection
            .pending
            .lock()
            .unwrap()
            .get_mut(&1)
            .unwrap()
            .deadline = Instant::now();
        connection.complete(1, generation, Ok(Reply::AppClient(EmptyResult {})));
        assert_eq!(
            connection.out.lock().unwrap().frames[0]["error"]["code"],
            "timeout"
        );
        assert!(connection.pending.lock().unwrap().is_empty());
        println!("completion at/after deadline yields timeout even before writer's timer poll");
    }
}

#[cfg(test)]
mod argument_tests {
    use super::*;
    #[test]
    fn subscription_operations_are_unknown_until_p1_08() {
        for (op, args) in [
            ("subscribe", json!({"topics":["torrents"]})),
            ("unsubscribe", json!({"sub":1})),
        ] {
            let failure =
                decode(&serde_json::to_vec(&json!({"id":1,"op":op,"args":args})).unwrap())
                    .unwrap_err();
            assert_eq!(failure.error.code, ErrorCode::UnknownOp);
            assert!(failure.error.hint.is_some());
            assert_eq!(failure.error.details.unwrap()["nearest"], json!([]));
            assert!(!failure.close);
            println!("{op}: unknown_op, nearest=[], hint present, connection kept");
        }
        let failure = decode(br#"{"id":1,"op":"torrents.lis"}"#).unwrap_err();
        assert_eq!(
            failure.error.details.unwrap()["nearest"],
            json!(["torrents.list"])
        );
    }

    #[test]
    fn oversized_collection_is_an_id_bearing_error_and_does_not_end_connection() {
        let (connection, _) = super::tests::connection();
        let request = decode(br#"{"id":1,"op":"torrents.files","args":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#).unwrap();
        let generation = connection.reserve(&request).unwrap();
        let files = (0..2000)
            .map(|index| File {
                index,
                path: "x".repeat(1024),
                sort_key: "x".repeat(1024),
                size: 1,
                progress: 0.0,
                included: true,
                ready: None,
            })
            .collect();
        connection.complete(
            1,
            generation,
            Ok(Reply::TorrentsFiles(FilesResult { files })),
        );
        let next = decode(br#"{"id":2,"op":"app.client","args":{}}"#).unwrap();
        let generation = connection.reserve(&next).unwrap();
        connection.complete(2, generation, Ok(Reply::AppClient(EmptyResult {})));
        let out = connection.out.lock().unwrap();
        assert!(!out.ending);
        assert!(out.terminal.is_none());
        assert_eq!(out.frames.len(), 2);
        assert_eq!(out.frames[0]["id"], 1);
        assert_eq!(out.frames[0]["error"]["code"], "invalid_message");
        assert_eq!(out.frames[1]["id"], 2);
        assert_eq!(out.frames[1]["ok"], true);
        println!("oversized files: id=1 invalid_message; id=2 succeeds; no bye or close");
    }

    // Python owns process lifecycle and E.1 records. This opt-in adapter only
    // runs the frozen P1-05 Rust decoder/validator on captured live frames.
    #[test]
    #[ignore = "invoked by daemon.py with FBT_API_CONTRACT_TRACE"]
    fn validate_live_transcript() {
        let path = std::env::var("FBT_API_CONTRACT_TRACE").expect("live transcript path");
        let text = std::fs::read_to_string(&path).unwrap();
        crate::api::fixtures::check_group("live-P1-06", &text).unwrap();
        println!("P1-05 types, bounds, negotiated capabilities and canonical round trips validated: {path}");
    }

    #[test]
    fn live_transcript_validator_rejects_invalid_payloads() {
        let request = json!({"dir":"c2s","frame":{"id":1,"op":"torrents.list","args":{}}});
        for frame in [
            json!({"id":1,"ok":true,"result":{"torrents":[]}}),
            json!({"id":1,"ok":false,"error":{"code":"not_found","message":"missing","retryable":false}}),
        ] {
            let text = format!("{request}\n{}\n", json!({"dir":"s2c","frame":frame}));
            crate::api::fixtures::check_group("live-P1-06", &text).unwrap();
        }
        for frame in [
            json!({"id":1,"ok":true,"result":{"torrents":42}}),
            json!({"id":1,"ok":false,"error":{"code":"not_found","message":"missing"}}),
        ] {
            let text = format!("{request}\n{}\n", json!({"dir":"s2c","frame":frame}));
            assert!(crate::api::fixtures::check_group("live-P1-06", &text).is_err());
        }
    }

    #[test]
    fn strict_type_errors_name_the_argument() {
        for (op, args, key) in [
            ("app.client", json!({"interactive":"yes"}), "interactive"),
            ("torrents.start", json!({"hashes":["bad"]}), "hashes"),
            ("torrents.add", json!({"source":{"path":3}}), "source"),
            (
                "torrents.set_upload_limit",
                json!({"hashes":["ab".repeat(20)],"bps":4}),
                "bps",
            ),
            ("app.restart", json!({"scope":"unknown"}), "scope"),
            ("app.info", Value::Null, "args"),
        ] {
            let failure =
                decode(&serde_json::to_vec(&json!({"id":1,"op":op,"args":args})).unwrap())
                    .unwrap_err();
            assert_eq!(failure.error.code, ErrorCode::InvalidArgs);
            assert_eq!(failure.error.details.unwrap()["key"], key);
        }
        let clear = json!({"id":1,"op":"torrents.set_upload_limit","args":{"hashes":["ab".repeat(20)],"bps":null}});
        assert!(decode(&serde_json::to_vec(&clear).unwrap()).is_ok());
    }
}

/// Subscription producers use the same queue, capability checks and frame cap
/// as replies. One emission guard keeps a snapshot's revision sequence atomic.
#[derive(Clone)]
pub(crate) struct EventSink(Arc<Connection>);
impl EventSink {
    pub fn id(&self) -> u64 {
        self.0.number
    }
    fn checked(&self, event: Event) -> ApiResult<()> {
        event
            .validate()
            .map_err(|_| error(ErrorCode::Internal, "invalid server event", None))?;
        { self.0.client.lock().unwrap().caps.event(&event) }.map_err(|_| {
            error(
                ErrorCode::Internal,
                "event requires an unnegotiated capability",
                None,
            )
        })?;
        self.0.send(serde_json::to_value(event).unwrap());
        Ok(())
    }
    pub fn send(&self, event: Event) -> ApiResult<()> {
        let _emission = self.0.emission.lock().unwrap();
        self.checked(event)
    }
    pub fn snapshot(
        &self,
        sub: u64,
        topic: Topic,
        previous: u64,
        items: Vec<Item>,
        reason: SnapshotReason,
    ) -> ApiResult<u64> {
        let _emission = self.0.emission.lock().unwrap();
        let count = items.len() as u64;
        let mut chunks = Vec::new();
        let mut chunk = Vec::new();
        let mut bytes = 0;
        // Topic and numeric envelope overhead is below 1024 bytes. Both the
        // item-count and byte limits apply; UTF-8/escaping can exceed estimates
        // based solely on the 1024-character torrent-name limit.
        for item in items {
            let size = serde_json::to_vec(&item).unwrap().len() + 1;
            if size > LINE_OUT - 1024 {
                return Err(error(
                    ErrorCode::InvalidMessage,
                    "one subscription item exceeds the frame cap",
                    None,
                ));
            }
            if chunk.len() == MAX_CHUNK_ITEMS || bytes + size > LINE_OUT - 1024 {
                chunks.push(std::mem::take(&mut chunk));
                bytes = 0;
            }
            bytes += size;
            chunk.push(item);
        }
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        let final_rev = previous
            .checked_add(chunks.len() as u64 + 2)
            .ok_or_else(|| error(ErrorCode::Internal, "subscription revision exhausted", None))?;
        // Validate every payload before publishing the begin marker.
        for items in &chunks {
            let event = Event::SnapshotChunk {
                sub,
                topic: topic.clone(),
                rev: 1,
                items: items.clone(),
            };
            event.validate().map_err(|_| {
                error(
                    ErrorCode::Internal,
                    "snapshot payload does not match topic",
                    None,
                )
            })?;
            self.0
                .client
                .lock()
                .unwrap()
                .caps
                .event(&event)
                .map_err(|_| {
                    error(
                        ErrorCode::Internal,
                        "snapshot requires an unnegotiated capability",
                        None,
                    )
                })?;
        }
        let mut rev = previous + 1;
        self.checked(Event::SnapshotBegin {
            sub,
            topic: topic.clone(),
            rev,
            count,
            reason,
            oldest_seq: None,
        })?;
        for items in chunks {
            rev += 1;
            self.checked(Event::SnapshotChunk {
                sub,
                topic: topic.clone(),
                rev,
                items,
            })?;
        }
        self.checked(Event::SnapshotEnd {
            sub,
            topic,
            rev: final_rev,
        })?;
        Ok(final_rev)
    }
    pub fn added(&self, sub: u64, topic: Topic, previous: u64, items: Vec<Item>) -> ApiResult<u64> {
        let _emission = self.0.emission.lock().unwrap();
        let mut rev = previous;
        let mut chunk = Vec::new();
        let mut bytes = 0;
        for item in items {
            let size = serde_json::to_vec(&item).unwrap().len() + 1;
            if size > LINE_OUT - 1024 {
                return Err(error(
                    ErrorCode::InvalidMessage,
                    "one delta item exceeds the frame cap",
                    None,
                ));
            }
            if chunk.len() == MAX_CHUNK_ITEMS || bytes + size > LINE_OUT - 1024 {
                rev = rev.checked_add(1).ok_or_else(|| {
                    error(ErrorCode::Internal, "subscription revision exhausted", None)
                })?;
                self.checked(Event::Delta {
                    sub,
                    topic: topic.clone(),
                    rev,
                    changed: None,
                    added: Some(std::mem::take(&mut chunk)),
                    removed: None,
                })?;
                bytes = 0;
            }
            bytes += size;
            chunk.push(item);
        }
        if !chunk.is_empty() {
            rev = rev.checked_add(1).ok_or_else(|| {
                error(ErrorCode::Internal, "subscription revision exhausted", None)
            })?;
            self.checked(Event::Delta {
                sub,
                topic,
                rev,
                changed: None,
                added: Some(chunk),
                removed: None,
            })?;
        }
        Ok(rev)
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::*;
    #[test]
    fn collection_transport_splits_by_bytes_and_preserves_items_and_revisions() {
        let (connection, peer) = super::tests::connection();
        let connection = Arc::new(connection);
        let sink = EventSink(connection.clone());
        let item = |index| {
            Item::File(File {
                index,
                path: "💾".repeat(2000),
                size: 1,
                progress: 0.0,
                included: true,
                sort_key: index.to_string(),
                ready: None,
            })
        };
        let topic = Topic::Files(Hash::new("ab".repeat(20)).unwrap());
        let end = sink
            .snapshot(
                7,
                topic.clone(),
                3,
                (0..1000).map(item).collect(),
                SnapshotReason::Subscribe,
            )
            .unwrap();
        let last = sink
            .added(7, topic, end, (1000..1100).map(item).collect())
            .unwrap();
        let frames: Vec<_> = connection.out.lock().unwrap().frames.drain(..).collect();
        let read = thread::spawn(move || {
            let mut found = Vec::new();
            let mut revisions = Vec::new();
            let mut max = 0;
            for line in BufReader::new(peer).lines() {
                let line = line.unwrap();
                max = max.max(line.len() + 1);
                assert!(line.len() + 1 <= LINE_OUT);
                let event: Event = serde_json::from_str(&line).unwrap();
                event.validate().unwrap();
                revisions.push(event.subscription_revision().unwrap().1);
                match event {
                    Event::SnapshotChunk { items, .. }
                    | Event::Delta {
                        added: Some(items), ..
                    } => {
                        assert!(items.len() <= 256);
                        for item in items {
                            let Item::File(file) = item else {
                                panic!("file payload")
                            };
                            found.push(file.index);
                        }
                    }
                    _ => {}
                }
            }
            (found, revisions, max)
        });
        let mut stream = connection.stream.try_clone().unwrap();
        for frame in &frames {
            write_frame(&mut stream, frame).unwrap();
        }
        stream.shutdown(Shutdown::Write).unwrap();
        let (found, revisions, max) = read.join().unwrap();
        assert_eq!(found, (0..1100).collect::<Vec<_>>());
        assert_eq!(revisions, (4..=last).collect::<Vec<_>>());
        println!("chunk transport: 1100 items, {} real Unix frames, max={} bytes including newline; contiguous rev=4..={last}",frames.len(),max);
    }
}
