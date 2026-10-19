//! Bounded, same-user NDJSON transport owned by the daemon, across slot restarts.
use super::{
    ops::{self, ApiResult, error},
    *,
};
use crate::{
    daemon::{Daemon, lifecycle::StopReason},
    ipc::InstanceLock,
};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, json};
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
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod connection;
mod dispatch;
mod events;
mod frame;
mod listener;
mod quiesce;

use connection::{read_client, write_client};
use frame::*;
use listener::{allowed_uid, peer_uid};

type Frame = Value;

#[derive(Debug)]
struct Invalid {
    id: Option<u64>,
    error: Error,
    close: bool,
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

struct Shared {
    daemon: Arc<Daemon>,
    pool: Pool,
    clients: Mutex<HashMap<u64, Arc<Connection>>>,
    counts: Mutex<ClientCounts>,
    began: Instant,
    stopping: AtomicBool,
    event_sequence: AtomicU64,
}

pub struct Server {
    path: PathBuf,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
    dispatch: Vec<JoinHandle<()>>,
    _lock: Arc<InstanceLock>,
}

/// Subscription producers use the same queue, capability checks and frame cap
/// as replies. One emission guard keeps a snapshot's revision sequence atomic.
#[derive(Clone)]
pub(crate) struct EventSink(Arc<Connection>);

#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "server/admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "server/argument_tests.rs"]
mod argument_tests;

#[cfg(test)]
#[path = "server/chunk_tests.rs"]
mod chunk_tests;
