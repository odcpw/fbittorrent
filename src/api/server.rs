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
#[cfg(test)]
#[path = "server/admission_tests.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "server/argument_tests.rs"]
mod argument_tests;
#[cfg(test)]
#[path = "server/chunk_tests.rs"]
mod chunk_tests;
#[path = "server/framing.rs"]
mod framing;
#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;
use framing::*;
#[path = "server/dispatch.rs"]
mod dispatch;
use dispatch::*;
#[path = "server/mutation_flow.rs"]
mod mutation_flow;
use mutation_flow::*;
#[path = "server/connection.rs"]
mod connection;
use connection::*;

#[derive(Debug)]
struct Invalid {
    id: Option<u64>,
    error: Error,
    close: bool,
}

const CONNECTIONS: usize = 32;
const INFLIGHT: usize = 8;
const DISPATCH: usize = 8;
const QUEUE: usize = 64;
const OUTGOING: usize = 512;
const HELLO: Duration = Duration::from_secs(5);

type Frame = Value;
// P1-09: mutation execution outlives any particular client connection.
#[derive(Clone)]
enum Confirmation {
    Worker,
    Recheck(Arc<crate::engine::recheck::Batch>),
    Move(Arc<crate::engine::moving::Batch>),
    Metadata(Reply),
    Engine(u64),
}
#[derive(Clone)]
struct Background {
    ticket: mutations::Ticket,
    op: String,
    deadline: Option<Instant>,
    confirmation: Option<Confirmation>,
    _activity: Arc<crate::daemon::policy::Mutation>,
    handler_add: bool,
}
struct Pending {
    generation: u64,
    deadline: Instant,
    no_timeout: bool,
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
    dashboard_visible: bool,
    overlay_visible:bool,
    glance: bool,
    caps: validation::NegotiatedCaps,
}
impl ClientState {
    fn is_interactive(&self)->bool {
        self.interactive && self.client.as_ref().is_some_and(|c|c.kind!=ClientKind::Bar || self.dashboard_visible || self.overlay_visible)
    }
}
struct Connection {
    number: u64,
    stream: UnixStream,
    accepted: Instant,
    closed: AtomicBool,
    read_done: AtomicBool,
    client: Mutex<ClientState>,
    pending: Mutex<BTreeMap<u64, Pending>>,
    sequence: AtomicU64,
    out: Mutex<Outbox>,
    ready: Condvar,
    emission: Mutex<()>,
}
struct Job {
    connection: Arc<Connection>,
    request: Request,
    generation: u64,
    mutation: Option<mutations::Ticket>,
    _activity: Option<Arc<crate::daemon::policy::Mutation>>,
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
struct Shared {
    daemon: Arc<Daemon>,
    bus: Arc<bus::Bus>,
    pool: Pool,
    clients: Mutex<HashMap<u64, Arc<Connection>>>,
    counts: Mutex<ClientCounts>,
    connections_since_start: Mutex<ClientCounts>,
    began: Instant,
    stopping: AtomicBool,
    mutation_ring: Mutex<mutations::Ring>,
    mutation_sequence: AtomicU64,
    mutation_active: Mutex<HashMap<(u64, u64), Background>>,
    mutation_events: Mutex<VecDeque<Event>>,
    mutation_changed: Condvar,
}
pub struct Server {
    path: PathBuf,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    dispatch: Vec<JoinHandle<()>>,
    mutation_observer: Option<JoinHandle<()>>,
    bus_worker: Option<JoinHandle<()>>,
    _lock: Arc<InstanceLock>,
}
impl Server {
    pub(crate) fn event_bus(&self) -> std::sync::Weak<bus::Bus> { Arc::downgrade(&self.shared.bus) }
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
            connections_since_start: Mutex::new(ClientCounts { cli: 0, agent: 0, tui: 0, bar: 0, test: 0 }),
            began: Instant::now(),
            stopping: AtomicBool::new(false),
            mutation_ring: Mutex::new(mutations::Ring::default()),
            mutation_sequence: AtomicU64::new(1),
            mutation_active: Mutex::new(HashMap::new()),
            mutation_events: Mutex::new(VecDeque::new()),
            mutation_changed: Condvar::new(),
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
            mutation_observer: None,
            bus_worker: Some(bus.start()?),
            _lock: lock,
        };
        let observer = shared.clone();
        server.mutation_observer = Some(
            thread::Builder::new()
                .name("fbt-api-mutations".into())
                .spawn(move || loop {
                    let mut active = observer.mutation_active.lock().unwrap();
                    while active.is_empty() && !observer.stopping.load(Ordering::Acquire) {
                        active = observer.mutation_changed.wait(active).unwrap();
                    }
                    if observer.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    drop(active);
                    observer.observe_mutations();
                    let active = observer.mutation_active.lock().unwrap();
                    let _guard = observer
                        .mutation_changed
                        .wait_timeout(active, Duration::from_millis(25))
                        .unwrap();
                })?,
        );
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
                            // Wake immediately for a connection without probing
                            // accept 100 times a second while idle. The timeout
                            // still bounds shutdown's join of this worker.
                            let mut ready = libc::pollfd {
                                fd: listener.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            // SAFETY: the listener owns this fd for the entire
                            // call, and poll receives one initialized entry.
                            unsafe { libc::poll(&mut ready, 1, 100); }
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
        if let Some(worker) = self.bus_worker.take() {
            let _ = worker.join();
        }
        {
            let _active = self.shared.mutation_active.lock().unwrap();
            self.shared.stopping.store(true, Ordering::Release);
            self.shared.mutation_changed.notify_all();
        }
        if let Some(thread) = self.accept.take() {
            let _ = thread.join();
        }
    }
    pub fn drain(&mut self, reason: StopReason) {
        self.stop_accepting();
        self.shared.pool.close();
        if let Some(observer) = self.mutation_observer.take() {
            let _ = observer.join();
        }
        for thread in self.dispatch.drain(..) {
            let _ = thread.join();
        }
        let reason = match reason {
            StopReason::Idle => ByeReason::Idle,
            StopReason::Upgrade => ByeReason::Upgrade,
            _ => ByeReason::Shutdown,
        };
        // Engine-slot drain joins rechecks before API drain. Publish their
        // terminal receipts before bye, even though the observer has stopped.
        self.shared.observe_mutations();
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

/// Subscription producers use the same queue, capability checks and frame cap
/// as replies. One emission guard keeps a snapshot's revision sequence atomic.
#[derive(Clone)]
pub(crate) struct EventSink(Arc<Connection>);
impl EventSink {
    #[cfg(test)]
    pub(crate) fn testing() -> Self {
        Self(Arc::new(tests::connection().0))
    }
    #[cfg(test)]
    pub(crate) fn testing_negotiate(&self,hello:&HelloResult) {
        self.0.client.lock().unwrap().caps=validation::NegotiatedCaps::from_hello(hello);
    }
    #[cfg(test)]
    pub(crate) fn testing_with_hello() -> Self {
        let sink=Self::testing();
        let frame:Value=include_str!("../../contracts/ipc/v1/hello.ndjson").lines().map(|l|serde_json::from_str::<Value>(l).unwrap()).find(|v|v["frame"]["result"].get("caps").is_some()).unwrap();
        let hello:HelloResult=serde_json::from_value(frame["frame"]["result"].clone()).unwrap();
        sink.0.client.lock().unwrap().caps=validation::NegotiatedCaps::from_hello(&hello);
        sink
    }
    #[cfg(test)]
    pub(crate) fn take_frames(&self) -> Vec<Value> {
        self.0.out.lock().unwrap().frames.drain(..).collect()
    }
    #[cfg(test)]
    pub(crate) fn terminal(&self) -> Option<Value> {
        self.0.out.lock().unwrap().terminal.clone()
    }
    pub fn id(&self) -> u64 {
        self.0.number
    }
    pub fn summary_interval(&self) -> u64 {
        let state = self.0.client.lock().unwrap();
        if !state.interactive
            && !state.glance
            && state
                .client
                .as_ref()
                .is_some_and(|c| c.kind == ClientKind::Bar)
        {
            5000
        } else {
            1000
        }
    }
    pub fn closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire) || self.0.out.lock().unwrap().ending
    }
    pub fn close(&self) {
        self.0.finish(ByeReason::InvalidMessage);
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
        self.snapshot_resync(sub, topic, previous, items, reason, None)
    }
    pub fn snapshot_resync(
        &self, sub: u64, topic: Topic, previous: u64, items: Vec<Item>,
        reason: SnapshotReason, oldest_seq: Option<u64>,
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
        if chunks.is_empty() && reason != SnapshotReason::EngineRestart {
            chunks.push(Vec::new());
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
            oldest_seq,
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
mod quick_presence_tests {
    use super::*;
    #[test]
    fn quick_palette_presence_requires_a_visible_surface() {
        let mut state=ClientState::default();
        state.client=Some(serde_json::from_value(json!({"name":"shared-bar","version":"0.2.0","kind":"bar"})).unwrap());
        state.interactive=true;
        assert!(!state.is_interactive(),"bar-only socket cannot keep daemon alive");
        state.overlay_visible=true;assert!(state.is_interactive());
        state.overlay_visible=false;assert!(!state.is_interactive());
        state.dashboard_visible=true;assert!(state.is_interactive());
        state.interactive=false;assert!(!state.is_interactive());
        for value in [true,false] {
            let request:Request=serde_json::from_value(json!({"id":1,"op":"app.client","args":{"interactive":value,"overlay_visible":value}})).unwrap();
            assert!(matches!(request.call,Call::AppClient{args} if args.overlay_visible==Some(value)));
        }
        let failure=framing::decode(br#"{"id":1,"op":"app.client","args":{"overlay_visible":"true"}}"#).err().unwrap();
        assert_eq!(failure.error.details.as_ref().unwrap()["key"],"overlay_visible");
        assert_eq!(failure.error.details.as_ref().unwrap()["expected"],"boolean");
    }
}
