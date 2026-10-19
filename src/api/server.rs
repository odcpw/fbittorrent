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
mod wire;
use wire::*;

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
            let client = self.client.lock().unwrap();
            if let Some(client) = &client.client {
                eprintln!(
                    "WARN api slow client kind={} name={} queued=512",
                    serde_json::to_value(client.kind).unwrap(),
                    serde_json::to_string(&client.name).unwrap()
                );
            }
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
    _activity: Option<crate::daemon::policy::Mutation>,
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
    bus: Arc<bus::Bus>,
    pool: Pool,
    clients: Mutex<HashMap<u64, Arc<Connection>>>,
    counts: Mutex<ClientCounts>,
    began: Instant,
    stopping: AtomicBool,
}
impl Shared {
    fn presence(&self, connection: &Connection) {
        let client = connection.client.lock().unwrap();
        let interactive = client.interactive && client.client.as_ref()
            .is_some_and(|c| c.kind != ClientKind::Bar);
        let glance = client.glance;
        drop(client);
        self.daemon.policy.lock().unwrap().client(connection.number, interactive, glance, Instant::now());
        self.bus.refresh();
    }
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
            self.bus.publish(
                serde_json::to_value(Event::MutationResult {
                    seq: 0, // Assigned once by the instance-wide bus ring below.
                    ts: now(),
                    reference,
                    op: "torrents.add".into(),
                    outcome,
                    error,
                    result,
                })
                .unwrap(),
            ),
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
                    self.bus.counts(),
                ),
                Call::Subscribe { args } => {
                    self.bus.subscribe(EventSink(job.connection.clone()), args)
                }
                Call::Unsubscribe { args } => {
                    Ok(self.bus.unsubscribe(job.connection.number, args.sub))
                }
                Call::TransferInfo { .. } => ops::transfer::info(&self.daemon),
                _ => ops::torrents::dispatch(&self.daemon, &job.request.call),
            }
        };
        let activate = match &result {
            Ok(Reply::Subscribe(sub)) => Some(sub.sub),
            _ => None,
        };
        let timely = job
            .connection
            .complete(job.request.id.get(), job.generation, result);
        if let Some(sub) = activate {
            if timely {
                self.bus.activate(job.connection.number, sub);
            } else {
                self.bus.unsubscribe(job.connection.number, sub);
            }
        }
    }
}
pub struct Server {
    path: PathBuf,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    dispatch: Vec<JoinHandle<()>>,
    bus_worker: Option<JoinHandle<()>>,
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
        let bus = bus::Bus::new(daemon.clone());
        let shared = Arc::new(Shared {
            daemon,
            bus: bus.clone(),
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
            bus_worker: Some(bus.start()?),
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
        self.shared.bus.stop();
        if let Some(thread) = self.bus_worker.take() {
            let _ = thread.join();
        }
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

mod transport;
use transport::*;

#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "server/admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "server/argument_tests.rs"]
mod argument_tests;

mod events;
pub(crate) use events::EventSink;

#[cfg(test)]
#[path = "server/chunk_tests.rs"]
mod chunk_tests;
