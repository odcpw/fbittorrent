//! BEP 27 regression through the real add, tracker, DHT and peer wire paths.
//! Every remote address comes from these loopback fixtures. Public discovery
//! remains enabled as a positive control; no production discovery is mocked.

use crate::torrent_state::ManagedTorrentHandle;
use crate::{
    AddTorrent, AddTorrentOptions, DhtSessionConfig, ListenerOptions, PeerConnectionOptions,
    PeerEncryption, Session, SessionOptions,
};
use bencode::{BencodeValue, BencodeValueOwned, ByteBufOwned};
use librqbit_core::hash_id::Id20;
use peer_binary_protocol::Handshake;
use rqbit_runtime::{Budget, Cx, RuntimeBuilder, runtime::Runtime};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BOUND: Duration = Duration::from_secs(5);
const IO_TICK: Duration = Duration::from_millis(100);
// The real engine deliberately waits 10 s before its first PeX update.
const OBSERVE: Duration = Duration::from_secs(12);
const OBSERVE_LIMIT: Duration = Duration::from_secs(25);
const PEX_ID: u8 = 42;

#[derive(Clone, Default)]
struct EngineLog(Arc<Mutex<Vec<u8>>>);
impl Write for EngineLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

thread_local! {
    // Each native worker gets this test's dispatcher, without a process-global
    // subscriber that would interfere with other concurrently running tests.
    static TRACE_GUARD: RefCell<Option<tracing::dispatcher::DefaultGuard>> = const { RefCell::new(None) };
}

#[derive(Clone, Default, Debug)]
struct Counts {
    tracker: usize,
    extra_tracker: usize,
    injected_tracker: usize,
    backup_tracker: usize,
    empty_tracker: usize,
    dht_queries: usize,
    dht_announces: usize,
    peer_handshakes: usize,
    connected: usize,
    pex: usize,
}
type Observations = Arc<Mutex<HashMap<Id20, Counts>>>;

fn count(observed: &Observations, hash: Id20, update: impl FnOnce(&mut Counts)) {
    update(observed.lock().unwrap().entry(hash).or_default());
}
fn snapshot(observed: &Observations, hash: Id20) -> Counts {
    observed
        .lock()
        .unwrap()
        .get(&hash)
        .cloned()
        .unwrap_or_default()
}
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    write!(out, "{}:", value.len()).unwrap();
    out.extend_from_slice(value);
}
fn field<'a>(value: &'a BencodeValueOwned, key: &[u8]) -> Option<&'a BencodeValueOwned> {
    match value {
        BencodeValue::Dict(fields) => fields.get(&ByteBufOwned::from(key)),
        _ => None,
    }
}
fn string(value: &BencodeValueOwned) -> &[u8] {
    match value {
        BencodeValue::Bytes(b) => b.as_ref(),
        _ => panic!("not bytes: {value:?}"),
    }
}
fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

struct Fixtures {
    tracker: String,
    extra_tracker: String,
    injected_tracker: String,
    backup_tracker: String,
    empty_tracker: String,
    dht: SocketAddr,
    observed: Observations,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

#[derive(Clone, Copy)]
enum Tracker {
    Primary,
    Extra,
    Injected,
    Backup,
    Empty,
}

fn start_peer(
    stop: Arc<AtomicBool>,
    observed: Observations,
    threads: &mut Vec<JoinHandle<()>>,
) -> (SocketAddr, Arc<AtomicUsize>) {
    let peer = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = peer.local_addr().unwrap();
    peer.set_nonblocking(true).unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let connections = accepted.clone();
    threads.push(thread::spawn(move || {
        let mut peers = Vec::new();
        while !stop.load(Ordering::Acquire) {
            match peer.accept() {
                Ok((socket, _)) => {
                    // Count TCP contact before reading any handshake. Absence
                    // assertions must also catch a connection that sends nothing.
                    connections.fetch_add(1, Ordering::AcqRel);
                    let stop = stop.clone();
                    let observed = observed.clone();
                    peers.push(thread::spawn(move || serve_peer(socket, stop, observed)));
                }
                Err(e) if retry(&e) => thread::sleep(Duration::from_millis(10)),
                Err(e) => panic!("peer accept: {e}"),
            }
        }
        for peer in peers {
            peer.join().unwrap();
        }
    }));
    (addr, accepted)
}

impl Fixtures {
    fn new() -> Self {
        let observed = Observations::default();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let (peer_addr, _) = start_peer(stop.clone(), observed.clone(), &mut threads);
        let mut tracker = |kind| {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let url = format!("http://{}/announce", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = stop.clone();
            let observed = observed.clone();
            threads.push(thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((socket, _)) => serve_tracker(socket, peer_addr, kind, &observed),
                        Err(e) if retry(&e) => thread::sleep(Duration::from_millis(10)),
                        Err(e) => panic!("tracker accept: {e}"),
                    }
                }
            }));
            url
        };
        let tracker_url = tracker(Tracker::Primary);
        let extra_tracker = tracker(Tracker::Extra);
        let injected_tracker = tracker(Tracker::Injected);
        let backup_tracker = tracker(Tracker::Backup);
        let empty_tracker = tracker(Tracker::Empty);
        // An ID close to the public info hash also exercises announce_peer:
        // the engine only announces to DHT nodes sufficiently near that hash.
        let public_hash = librqbit_core::torrent_metainfo::torrent_from_bytes(&metainfo(
            "public.bin",
            false,
            Some(&tracker_url),
        ))
        .unwrap()
        .info_hash;
        let dht = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let dht_addr = dht.local_addr().unwrap();
        dht.set_read_timeout(Some(IO_TICK)).unwrap();
        threads.push(thread::spawn({
            let stop = stop.clone();
            let observed = observed.clone();
            move || serve_dht(dht, public_hash, stop, observed)
        }));
        Self {
            tracker: tracker_url,
            extra_tracker,
            injected_tracker,
            backup_tracker,
            empty_tracker,
            dht: dht_addr,
            observed,
            stop,
            threads,
        }
    }

    fn peer(&mut self) -> (SocketAddr, Arc<AtomicUsize>) {
        start_peer(self.stop.clone(), self.observed.clone(), &mut self.threads)
    }
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            // Preserve the original assertion when unwinding, but fail on any
            // fixture protocol/thread error during an otherwise passing test.
            let result = thread.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn serve_tracker(mut socket: TcpStream, peer: SocketAddr, kind: Tracker, observed: &Observations) {
    socket.set_read_timeout(Some(BOUND)).unwrap();
    socket.set_write_timeout(Some(BOUND)).unwrap();
    let mut request = Vec::new();
    let mut chunk = [0; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = socket.read(&mut chunk).unwrap();
        assert!(n > 0 && request.len() + n < 16384, "bounded HTTP request");
        request.extend_from_slice(&chunk[..n]);
    }
    let request = std::str::from_utf8(&request).unwrap();
    let query = request
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split_once('?')
        .unwrap()
        .1;
    let encoded = query
        .split('&')
        .find_map(|v| v.strip_prefix("info_hash="))
        .unwrap();
    let mut decoded = Vec::new();
    let mut encoded = encoded.as_bytes();
    while !encoded.is_empty() {
        if encoded[0] == b'%' {
            decoded.push(
                u8::from_str_radix(std::str::from_utf8(&encoded[1..3]).unwrap(), 16).unwrap(),
            );
            encoded = &encoded[3..];
        } else {
            decoded.push(if encoded[0] == b'+' { b' ' } else { encoded[0] });
            encoded = &encoded[1..];
        }
    }
    let hash = Id20::new(decoded.try_into().expect("20 byte tracker info hash"));
    count(observed, hash, |c| match kind {
        Tracker::Primary => c.tracker += 1,
        Tracker::Extra => c.extra_tracker += 1,
        Tracker::Injected => c.injected_tracker += 1,
        Tracker::Backup => c.backup_tracker += 1,
        Tracker::Empty => c.empty_tracker += 1,
    });
    let mut response = b"d8:intervali60e5:peers".to_vec();
    let mut compact = Vec::new();
    if matches!(kind, Tracker::Primary | Tracker::Injected | Tracker::Backup) {
        compact.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
        compact.extend_from_slice(&peer.port().to_be_bytes());
    }
    bytes(&mut response, &compact);
    response.push(b'e');
    write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    )
    .unwrap();
    socket.write_all(&response).unwrap();
}

fn serve_dht(socket: UdpSocket, node_id: Id20, stop: Arc<AtomicBool>, observed: Observations) {
    let mut buffer = [0; 4096];
    while !stop.load(Ordering::Acquire) {
        let (n, source) = match socket.recv_from(&mut buffer) {
            Ok(value) => value,
            Err(e) if retry(&e) => continue,
            Err(e) => panic!("DHT receive: {e}"),
        };
        assert!(source.ip().is_loopback());
        let query: BencodeValueOwned = bencode::from_bytes(&buffer[..n]).unwrap();
        let Some(method) = field(&query, b"q") else {
            continue;
        };
        let method = string(method);
        if let Some(hash) = field(&query, b"a").and_then(|a| field(a, b"info_hash")) {
            let hash = Id20::new(string(hash).try_into().unwrap());
            count(&observed, hash, |c| {
                c.dht_queries += 1;
                c.dht_announces += usize::from(method == b"announce_peer");
            });
        }
        // A real BEP 5 node, advertising only itself. This confines every
        // bootstrap, recursive lookup and announcement to the loopback socket.
        let mut response = b"d1:rd2:id20:".to_vec();
        response.extend_from_slice(&node_id.0);
        if method != b"announce_peer" {
            response.extend_from_slice(b"5:nodes26:");
            response.extend_from_slice(&node_id.0);
            response.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
            response.extend_from_slice(&socket.local_addr().unwrap().port().to_be_bytes());
            response.extend_from_slice(b"5:token4:seed");
            if method == b"get_peers" {
                response.extend_from_slice(b"6:valuesle");
            }
        }
        response.extend_from_slice(b"e1:t");
        bytes(&mut response, string(field(&query, b"t").unwrap()));
        response.extend_from_slice(b"1:y1:re");
        socket.send_to(&response, source).unwrap();
    }
}

fn serve_peer(mut socket: TcpStream, stop: Arc<AtomicBool>, observed: Observations) {
    socket.set_read_timeout(Some(BOUND)).unwrap();
    socket.set_write_timeout(Some(BOUND)).unwrap();
    let mut handshake = [0; 68];
    socket.read_exact(&mut handshake).unwrap();
    let (hello, _) = Handshake::deserialize(&handshake).unwrap();
    assert!(hello.supports_extended());
    let hash = hello.info_hash;
    let written =
        Handshake::new(hash, Id20::new([0x42; 20])).serialize_unchecked_len(&mut handshake);
    assert_eq!(written, handshake.len());
    socket.write_all(&handshake).unwrap();
    // Negotiate a non-default PeX id, then advertise the one available piece. The
    // peer stays choked so no test payload service is needed to remain live.
    let extension = format!("d1:md6:ut_pexi{PEX_ID}eee");
    let mut frame = ((extension.len() + 2) as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&[20, 0]);
    frame.extend_from_slice(extension.as_bytes());
    frame.extend_from_slice(&[0, 0, 0, 2, 5, 0x80]);
    socket.write_all(&frame).unwrap();
    count(&observed, hash, |c| {
        c.peer_handshakes += 1;
        c.connected += 1;
    });
    socket.set_read_timeout(Some(IO_TICK)).unwrap();
    let mut pending = Vec::new();
    let mut buffer = [0; 8192];
    while !stop.load(Ordering::Acquire) {
        match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => pending.extend_from_slice(&buffer[..n]),
            Err(e) if retry(&e) => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof
                ) =>
            {
                break;
            }
            Err(e) => panic!("peer receive: {e}"),
        }
        while pending.len() >= 4 {
            let n = u32::from_be_bytes(pending[..4].try_into().unwrap()) as usize;
            assert!(n <= 65536, "bounded peer frame");
            if pending.len() < n + 4 {
                break;
            }
            if n >= 2 && pending[4..6] == [20, PEX_ID] {
                let pex: BencodeValueOwned = bencode::from_bytes(&pending[6..n + 4]).unwrap();
                assert!(!string(field(&pex, b"added").unwrap()).is_empty());
                count(&observed, hash, |c| c.pex += 1);
            }
            pending.drain(..n + 4);
        }
    }
    count(&observed, hash, |c| c.connected -= 1);
}

fn metainfo(name: &str, private: bool, tracker: Option<&str>) -> Vec<u8> {
    metainfo_with_tiers(name, private, tracker, &[])
}

fn metainfo_with_tiers(
    name: &str,
    private: bool,
    tracker: Option<&str>,
    tiers: &[&[&str]],
) -> Vec<u8> {
    let mut torrent = b"d".to_vec();
    if let Some(tracker) = tracker {
        torrent.extend_from_slice(b"8:announce");
        bytes(&mut torrent, tracker.as_bytes());
    }
    if !tiers.is_empty() {
        torrent.extend_from_slice(b"13:announce-listl");
        for tier in tiers {
            torrent.push(b'l');
            for tracker in *tier {
                bytes(&mut torrent, tracker.as_bytes());
            }
            torrent.push(b'e');
        }
        torrent.push(b'e');
    }
    torrent.extend_from_slice(b"4:infod6:lengthi16384e4:name");
    bytes(&mut torrent, name.as_bytes());
    torrent.extend_from_slice(b"12:piece lengthi16384e6:pieces20:");
    torrent.extend_from_slice(&[0x33; 20]);
    if private {
        torrent.extend_from_slice(b"7:privatei1e");
    }
    torrent.extend_from_slice(b"ee");
    torrent
}

#[test]
fn private_no_dht_no_pex() {
    let fixtures = Fixtures::new();
    let output = tempfile::tempdir().unwrap();
    let log = EngineLog::default();
    let writer = log.clone();
    let dispatch = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish(),
    );
    let _guard = tracing::dispatcher::set_default(&dispatch);
    let worker_dispatch = dispatch.clone();
    let runtime = RuntimeBuilder::new()
        .worker_threads(2)
        .blocking_threads(1, 2)
        .enable_time()
        .on_thread_start(move || {
            TRACE_GUARD.with(|guard| {
                *guard.borrow_mut() = Some(tracing::dispatcher::set_default(&worker_dispatch))
            })
        })
        .on_thread_stop(|| {
            TRACE_GUARD.with(|guard| {
                guard.borrow_mut().take();
            })
        })
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let session = runtime
        .block_on(Session::new_with_opts(
            &cx,
            output.path().to_owned(),
            SessionOptions {
                peer_encryption: PeerEncryption::Off,
                dht: Some(DhtSessionConfig {
                    bootstrap_addrs: Some(vec![fixtures.dht.to_string()]),
                    port: Some(0),
                    persistence: None,
                }),
                listen: Some(ListenerOptions {
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    announce_port: Some(4240),
                    ..Default::default()
                }),
                trackers: HashSet::from([fixtures.extra_tracker.parse().unwrap()]),
                // The test observes DHT and PeX, with no unrelated multicast traffic.
                disable_local_service_discovery: true,
                ipv4_only: true,
                runtime_worker_threads: Some(2),
                ..Default::default()
            },
        ))
        .unwrap();
    let add = |name, private, tracker| {
        runtime
            .block_on(session.add_torrent(
                &cx,
                AddTorrent::from_bytes(metainfo(name, private, tracker)),
                Some(AddTorrentOptions {
                    peer_opts: Some(PeerConnectionOptions {
                        read_write_timeout: Some(Duration::from_secs(45)),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            ))
            .unwrap()
            .into_handle()
            .unwrap()
    };
    let private = add("private.bin", true, Some(fixtures.tracker.as_str()));
    let no_tracker = add("private-no-tracker.bin", true, None);
    let public = add("public.bin", false, Some(fixtures.tracker.as_str()));
    let ready = Instant::now();
    while ready.elapsed() < BOUND {
        if snapshot(&fixtures.observed, private.info_hash()).connected > 0
            && snapshot(&fixtures.observed, public.info_hash()).connected > 0
        {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let connected = snapshot(&fixtures.observed, private.info_hash()).connected > 0
        && snapshot(&fixtures.observed, public.info_hash()).connected > 0;
    let started = Instant::now();
    while started.elapsed() < OBSERVE_LIMIT {
        if started.elapsed() >= OBSERVE && snapshot(&fixtures.observed, public.info_hash()).pex > 0
        {
            break;
        }
        thread::sleep(IO_TICK);
    }
    let private_counts = snapshot(&fixtures.observed, private.info_hash());
    let no_tracker_counts = snapshot(&fixtures.observed, no_tracker.info_hash());
    let public_counts = snapshot(&fixtures.observed, public.info_hash());
    // Always drain the real session and fixtures before checking observations.
    let stopped = runtime.block_on(rqbit_runtime::time::timeout(&cx, BOUND, session.stop(&cx)));
    let leaks = runtime.diagnostics().find_leaked_obligations().len();
    let joined = runtime.shutdown_timeout(BOUND);
    let engine_log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let dht_lines: Vec<_> = engine_log
        .lines()
        .filter(|line| {
            line.contains("librqbit_dht")
                && line.contains("sending")
                && (line.contains("GetPeersRequest") || line.contains("AnnouncePeer"))
        })
        .collect();
    for line in &dht_lines {
        println!("ENGINE-LOG {line}");
    }
    let private_dht_lines = dht_lines
        .iter()
        .filter(|line| line.contains(&format!("{:?}", private.info_hash())))
        .count();
    let public_dht_lines = dht_lines
        .iter()
        .filter(|line| line.contains(&format!("{:?}", public.info_hash())))
        .count();
    println!(
        "PRIVATE-RULES {}",
        serde_json::json!({
            "private_hash": format!("{:?}", private.info_hash()),
            "private_dht_log_queries": private_dht_lines,
            "private_dht_queries": private_counts.dht_queries,
            "private_dht_announces": private_counts.dht_announces,
            "private_pex_messages": private_counts.pex,
            "private_extra_tracker_announces": private_counts.extra_tracker,
            "no_tracker_extra_announces": no_tracker_counts.extra_tracker,
            "public_dht_log_queries": public_dht_lines,
            "public_dht_queries": public_counts.dht_queries,
            "public_dht_announces": public_counts.dht_announces,
            "public_pex_messages": public_counts.pex,
            "public_extra_tracker_announces": public_counts.extra_tracker,
            "observe_ms": started.elapsed().as_millis()
        })
    );
    assert!(matches!(stopped, Ok(Ok(()))), "session drains: {stopped:?}");
    assert_eq!(leaks, 0);
    assert!(joined, "runtime workers join");
    assert!(
        connected,
        "both torrents connect to the tracker-returned wire peer"
    );
    assert!(
        private_counts.connected > 0 && public_counts.connected > 0,
        "wire peers remain live throughout the PeX observation window"
    );
    assert!(private_counts.tracker > 0 && public_counts.tracker > 0);
    assert!(
        public_counts.dht_queries > 0 && public_dht_lines > 0,
        "DHT positive control"
    );
    assert!(
        public_counts.dht_announces > 0,
        "public DHT announcement positive control"
    );
    assert!(
        public_counts.pex > 0,
        "public PeX positive control after the real 10 s delay"
    );
    assert!(
        public_counts.extra_tracker > 0,
        "public torrents keep session trackers"
    );
    assert_eq!(private_dht_lines, 0, "private DHT query log lines");
    assert_eq!(private_counts.dht_queries, 0, "private DHT wire queries");
    assert_eq!(private_counts.pex, 0, "private PeX wire messages");
    assert_eq!(no_tracker_counts.dht_queries, 0);
    assert_eq!(
        private_counts.extra_tracker, 0,
        "private torrents only announce to their tracker"
    );
    assert_eq!(
        no_tracker_counts.extra_tracker, 0,
        "private torrents without a tracker stay undiscoverable"
    );
}

struct NativeSession {
    runtime: Option<Runtime>,
    cx: Cx,
    session: Arc<Session>,
    _output: tempfile::TempDir,
}

impl NativeSession {
    fn new() -> Self {
        let output = tempfile::tempdir().unwrap();
        let runtime = RuntimeBuilder::new()
            .worker_threads(2)
            .blocking_threads(1, 2)
            .enable_time()
            .build()
            .unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let session = runtime
            .block_on(Session::new_with_opts(
                &cx,
                output.path().to_owned(),
                SessionOptions {
                    peer_encryption: PeerEncryption::Off,
                    dht: None,
                    disable_local_service_discovery: true,
                    runtime_worker_threads: Some(2),
                    ..Default::default()
                },
            ))
            .unwrap();
        Self {
            runtime: Some(runtime),
            cx,
            session,
            _output: output,
        }
    }

    fn add(&self, torrent: Vec<u8>, opts: AddTorrentOptions) -> ManagedTorrentHandle {
        self.runtime
            .as_ref()
            .unwrap()
            .block_on(self.session.add_torrent(
                &self.cx,
                AddTorrent::from_bytes(torrent),
                Some(opts),
            ))
            .unwrap()
            .into_handle()
            .unwrap()
    }

    fn restart(&self, handle: &ManagedTorrentHandle) {
        self.runtime.as_ref().unwrap().block_on(async {
            self.session.pause(&self.cx, handle).await.unwrap();
            self.session.unpause(&self.cx, handle).await.unwrap();
        });
    }

    fn pause(&self, handle: &ManagedTorrentHandle) {
        self.runtime
            .as_ref()
            .unwrap()
            .block_on(self.session.pause(&self.cx, handle))
            .unwrap();
    }

    fn finish(mut self) {
        let runtime = self.runtime.take().unwrap();
        let stopped = runtime.block_on(rqbit_runtime::time::timeout(
            &self.cx,
            BOUND,
            self.session.stop(&self.cx),
        ));
        let leaks = runtime.diagnostics().find_leaked_obligations().len();
        let joined = runtime.shutdown_timeout(BOUND);
        assert!(matches!(stopped, Ok(Ok(()))), "session drains: {stopped:?}");
        assert_eq!(leaks, 0);
        assert!(joined, "runtime workers join");
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.block_on(rqbit_runtime::time::timeout(
                &self.cx,
                BOUND,
                self.session.stop(&self.cx),
            ));
            let _ = runtime.shutdown_timeout(BOUND);
        }
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
    let began = Instant::now();
    while began.elapsed() < BOUND {
        if condition() {
            return true;
        }
        thread::sleep(IO_TICK);
    }
    condition()
}

#[test]
fn private_initial_peers_require_tracker_provenance() {
    let mut fixtures = Fixtures::new();
    let (injected_peer, private_contacts) = fixtures.peer();
    let (public_peer, public_contacts) = fixtures.peer();
    let native = NativeSession::new();
    let add = |name, private, peer| {
        native.add(
            metainfo(name, private, Some(&fixtures.empty_tracker)),
            AddTorrentOptions {
                initial_peers: Some(vec![peer]),
                ..Default::default()
            },
        )
    };
    let private = add("private-initial.bin", true, injected_peer);
    let public = add("public-initial.bin", false, public_peer);
    let ready = wait_for(|| {
        private.live().is_some()
            && snapshot(&fixtures.observed, private.info_hash()).empty_tracker > 0
            && public_contacts.load(Ordering::Acquire) > 0
    });
    // Exercise untrusted live-peer insertion too, rather than relying only on
    // the options sanitization to prevent non-tracker outgoing connections.
    let direct_accepted = private
        .live()
        .map(|live| live.add_peer_if_not_seen(injected_peer).unwrap())
        .unwrap_or(true);
    thread::sleep(Duration::from_secs(2));
    native.restart(&private);
    native.restart(&public);
    let restarted = wait_for(|| {
        snapshot(&fixtures.observed, private.info_hash()).empty_tracker >= 2
            && public_contacts.load(Ordering::Acquire) >= 2
    });
    thread::sleep(Duration::from_secs(2));
    native.finish();
    let private_contacts = private_contacts.load(Ordering::Acquire);
    let public_contacts = public_contacts.load(Ordering::Acquire);
    println!(
        "PRIVATE-INITIAL {}",
        serde_json::json!({
            "private_tcp_contacts": private_contacts, "public_tcp_contacts": public_contacts,
            "direct_untrusted_peer_accepted": direct_accepted, "ready": ready, "restarted": restarted,
        })
    );
    assert!(
        ready && restarted,
        "empty tracker and public initial-peer control work before and after restart"
    );
    assert_eq!(
        private_contacts, 0,
        "the empty private tracker did not authorize the injected peer"
    );
    assert!(
        !direct_accepted,
        "non-tracker live-peer insertion is refused for a private torrent"
    );
    assert!(
        public_contacts >= 2,
        "public initial peers still connect after restart"
    );
}

#[test]
fn private_per_add_trackers_are_ignored() {
    let fixtures = Fixtures::new();
    let native = NativeSession::new();
    let add = |name, private, tracker| {
        native.add(
            metainfo(name, private, tracker),
            AddTorrentOptions {
                trackers: Some(vec![fixtures.injected_tracker.clone()]),
                ..Default::default()
            },
        )
    };
    let private_none = add("private-injected-none.bin", true, None);
    let private_one = add("private-injected-one.bin", true, Some(&fixtures.tracker));
    let public_none = add("public-injected-none.bin", false, None);
    let public_one = add("public-injected-one.bin", false, Some(&fixtures.tracker));
    let ready = wait_for(|| {
        private_none.live().is_some()
            && snapshot(&fixtures.observed, private_one.info_hash()).peer_handshakes > 0
            && snapshot(&fixtures.observed, public_none.info_hash()).injected_tracker > 0
            && snapshot(&fixtures.observed, public_one.info_hash()).injected_tracker > 0
    });
    thread::sleep(Duration::from_secs(2));
    native.finish();
    let private_none = snapshot(&fixtures.observed, private_none.info_hash());
    let private_one = snapshot(&fixtures.observed, private_one.info_hash());
    let public_none = snapshot(&fixtures.observed, public_none.info_hash());
    let public_one = snapshot(&fixtures.observed, public_one.info_hash());
    println!(
        "PRIVATE-PER-ADD {}",
        serde_json::json!({
            "private_zero_metainfo_tracker_injected_announces": private_none.injected_tracker,
            "private_one_metainfo_tracker_injected_announces": private_one.injected_tracker,
            "private_metainfo_tracker_announces": private_one.tracker,
            "public_zero_metainfo_tracker_injected_announces": public_none.injected_tracker,
            "public_one_metainfo_tracker_injected_announces": public_one.injected_tracker,
        })
    );
    assert!(
        ready,
        "ordinary private tracker and both public injected-tracker controls are live"
    );
    assert_eq!(
        private_none.injected_tracker, 0,
        "per-add tracker cannot supply a missing private tracker"
    );
    assert_eq!(
        private_one.injected_tracker, 0,
        "per-add tracker cannot extend private metainfo"
    );
    assert!(private_one.tracker > 0 && private_one.peer_handshakes > 0);
    assert!(public_none.injected_tracker > 0 && public_one.injected_tracker > 0);
}

#[test]
fn private_tracker_uses_first_metainfo_tier() {
    let fixtures = Fixtures::new();
    let native = NativeSession::new();
    // Independent collections catch selection after an unordered conversion.
    // Run each before/after pair to completion before starting the next one;
    // this is a tier-selection test rather than a concurrent-init load test.
    let public = native.add(
        metainfo("healthy-backup.bin", false, Some(&fixtures.backup_tracker)),
        AddTorrentOptions::default(),
    );
    let mut ready =
        wait_for(|| snapshot(&fixtures.observed, public.info_hash()).peer_handshakes > 0);
    let mut restarted = true;
    let mut private = Vec::new();
    for index in 0..16 {
        let handle = native.add(
            metainfo_with_tiers(
                &format!("private-tiers-{index}.bin"),
                true,
                // BEP 12 ignores the legacy announce key when tiers are present.
                Some(&fixtures.extra_tracker),
                &[&[&fixtures.tracker], &[&fixtures.backup_tracker]],
            ),
            AddTorrentOptions::default(),
        );
        let announced = |minimum| {
            let counts = snapshot(&fixtures.observed, handle.info_hash());
            counts.tracker + counts.backup_tracker + counts.extra_tracker >= minimum
                && counts.peer_handshakes >= minimum
        };
        ready &= wait_for(|| announced(1));
        native.restart(&handle);
        restarted &= wait_for(|| announced(2));
        native.pause(&handle);
        private.push(handle);
    }
    thread::sleep(Duration::from_secs(1));
    native.finish();
    let observations: Vec<_> = private
        .iter()
        .map(|handle| snapshot(&fixtures.observed, handle.info_hash()))
        .collect();
    let backup_control = snapshot(&fixtures.observed, public.info_hash());
    println!(
        "PRIVATE-TIERS {}",
        serde_json::json!({
            "private_torrents": private.len(),
            "first_tier_announces": observations.iter().map(|c| c.tracker).sum::<usize>(),
            "second_tier_announces": observations.iter().map(|c| c.backup_tracker).sum::<usize>(),
            "legacy_announce_requests": observations.iter().map(|c| c.extra_tracker).sum::<usize>(),
            "healthy_backup_control_announces": backup_control.backup_tracker,
            "ready": ready, "restarted": restarted,
        })
    );
    assert!(
        ready && restarted,
        "both healthy tiers are reachable and private torrents restart"
    );
    for counts in observations {
        assert!(
            counts.tracker >= 2,
            "first tier before/after restart: {counts:?}"
        );
        assert_eq!(
            counts.backup_tracker, 0,
            "second tier must not replace a healthy first tier"
        );
        assert_eq!(
            counts.extra_tracker, 0,
            "announce-list takes precedence over announce"
        );
    }
    assert!(backup_control.backup_tracker > 0 && backup_control.peer_handshakes > 0);
}
