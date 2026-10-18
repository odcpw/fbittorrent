//! BEP 27 regression through the real add, tracker, DHT and peer wire paths.
//! Every remote address comes from these loopback fixtures. Public discovery
//! remains enabled as a positive control; no production discovery is mocked.

use crate::{
    AddTorrent, AddTorrentOptions, DhtSessionConfig, ListenerOptions, PeerConnectionOptions,
    PeerEncryption, Session, SessionOptions,
};
use bencode::{BencodeValue, BencodeValueOwned, ByteBufOwned};
use librqbit_core::hash_id::Id20;
use peer_binary_protocol::Handshake;
use rqbit_runtime::{Budget, RuntimeBuilder};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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
    dht: SocketAddr,
    observed: Observations,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}
impl Fixtures {
    fn new() -> Self {
        let observed = Observations::default();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let peer = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peer_addr = peer.local_addr().unwrap();
        peer.set_nonblocking(true).unwrap();
        threads.push(thread::spawn({
            let stop = stop.clone();
            let observed = observed.clone();
            move || {
                let mut peers = Vec::new();
                while !stop.load(Ordering::Acquire) {
                    match peer.accept() {
                        Ok((socket, _)) => {
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
            }
        }));
        let mut tracker = |extra| {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let url = format!("http://{}/announce", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = stop.clone();
            let observed = observed.clone();
            threads.push(thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((socket, _)) => serve_tracker(socket, peer_addr, extra, &observed),
                        Err(e) if retry(&e) => thread::sleep(Duration::from_millis(10)),
                        Err(e) => panic!("tracker accept: {e}"),
                    }
                }
            }));
            url
        };
        let tracker_url = tracker(false);
        let extra_tracker = tracker(true);
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
            dht: dht_addr,
            observed,
            stop,
            threads,
        }
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

fn serve_tracker(mut socket: TcpStream, peer: SocketAddr, extra: bool, observed: &Observations) {
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
    count(observed, hash, |c| {
        if extra {
            c.extra_tracker += 1
        } else {
            c.tracker += 1
        }
    });
    let mut response = b"d8:intervali60e5:peers".to_vec();
    let mut compact = Vec::new();
    if !extra {
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
    let mut torrent = b"d".to_vec();
    if let Some(tracker) = tracker {
        torrent.extend_from_slice(b"8:announce");
        bytes(&mut torrent, tracker.as_bytes());
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
