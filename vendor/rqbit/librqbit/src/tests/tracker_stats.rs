//! A real HTTP tracker drives the public handle API, including paused retention.
use crate::{AddTorrent, AddTorrentOptions, Session, SessionOptions, TrackerStatus};
use rqbit_runtime::{Budget, RuntimeBuilder};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);
impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
thread_local! {
    static TRACE:std::cell::RefCell<Option<tracing::dispatcher::DefaultGuard>>=const {std::cell::RefCell::new(None)};
}
fn traced_runtime() -> (
    rqbit_runtime::runtime::Runtime,
    Log,
    tracing::dispatcher::DefaultGuard,
) {
    let log = Log::default();
    let writer = log.clone();
    let dispatch = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish(),
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    let runtime = RuntimeBuilder::new()
        .worker_threads(2)
        .blocking_threads(1, 2)
        .enable_time()
        .on_thread_start(move || {
            TRACE.with(|guard| {
                *guard.borrow_mut() = Some(tracing::dispatcher::set_default(&dispatch))
            });
        })
        .build()
        .unwrap();
    (runtime, log, guard)
}
fn assert_redacted(log: Log) {
    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(
        !text.contains("fixture-secret")
            && !text.contains("private-secondary")
            && !text.contains("private-unsupported")
            && !text.contains("passkey=")
            && !text.contains("?key=")
            && !text.contains("0123456789abcdef0123456789abcdef")
            && !text.contains("QUJDREVGR0hJSktMTU5PUFFS"),
        "tracker TRACE diagnostics must redact query and path credentials"
    );
    assert!(
        text.contains("?…"),
        "positive control: actual tracker traces were captured"
    );
    println!("TRACKER-TRACE bytes={} query_redaction=true", text.len());
}
struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<(Instant, bool)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "http://{}/0123456789abcdef0123456789abcdef/announce?passkey=fixture-secret",
            listener.local_addr().unwrap()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (observed, stopping) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 1024];
                while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                    let size = socket.read(&mut chunk).unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                    assert!(bytes.len() < 8192);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.contains("passkey=fixture-secret"));
                assert!(request.contains("/0123456789abcdef0123456789abcdef/announce"));
                let failed = {
                    let mut requests = observed.lock().unwrap();
                    requests.push((Instant::now(), request.contains("event=")));
                    requests.len() == 1
                };
                let body = if failed {
                    format!("d14:failure reason{}:{}e", "fixture: not yet 0123456789abcdef0123456789abcdef".len(),"fixture: not yet 0123456789abcdef0123456789abcdef").into_bytes()
                } else {
                    b"d8:intervali600e5:peers6:\x7f\x00\x00\x01\x00\x01e".to_vec()
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn wait(mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    ready()
}
#[test]
fn tracker_stats_records_error_then_ok() {
    let fixture = Fixture::new();
    let output = tempfile::tempdir().unwrap();
    let (runtime, log, _trace) = traced_runtime();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let session = runtime
        .block_on(Session::new_with_opts(
            &cx,
            output.path().into(),
            SessionOptions {
                dht: None,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        ))
        .unwrap();
    let extra = format!(
        "{}/spare?key=private-secondary",
        fixture.url.split('?').next().unwrap()
    );
    let unsupported = "wss://127.0.0.1/announce?key=private-unsupported";
    let mut meta=format!("d13:announce-listll{}:{}el{}:{}el{}:{}ee4:infod6:lengthi4096e4:name11:payload.bin12:piece lengthi4096e6:pieces20:",fixture.url.len(),fixture.url,extra.len(),extra,unsupported.len(),unsupported).into_bytes();
    meta.extend([1u8; 20]);
    meta.extend(b"7:privatei1eee");
    let handle = runtime
        .block_on(session.add_torrent(
            &cx,
            AddTorrent::from_bytes(meta),
            Some(AddTorrentOptions::default()),
        ))
        .unwrap()
        .into_handle()
        .unwrap();
    let error_observed = wait(|| {
        handle
            .tracker_stats()
            .iter()
            .find(|row| row.url.as_str() == fixture.url)
            .is_some_and(|row| row.status == TrackerStatus::Error)
    });
    let before = handle.tracker_stats();
    let began = Instant::now();
    handle.announce_now();
    let success = wait(|| {
        handle
            .tracker_stats()
            .iter()
            .find(|row| row.url.as_str() == fixture.url)
            .is_some_and(|row| row.status == TrackerStatus::Ok)
    });
    let elapsed = began.elapsed();
    let after = handle.tracker_stats();
    handle.announce_now();
    thread::sleep(Duration::from_millis(250));
    runtime.block_on(session.pause(&cx, &handle)).unwrap();
    let paused = handle.tracker_stats();
    let requests = fixture.requests.lock().unwrap().clone();
    let stopped = runtime.block_on(rqbit_runtime::time::timeout(
        &cx,
        Duration::from_secs(5),
        session.stop(&cx),
    ));
    let joined = runtime.shutdown_timeout(Duration::from_secs(5));
    println!(
        "TRACKER-STATS error={error_observed} ok={success} forced_ms={} requests={} paused={:?}",
        elapsed.as_millis(),
        requests.len(),
        paused
    );
    assert!(matches!(stopped, Ok(Ok(()))) && joined);
    assert_redacted(log);
    assert_eq!(after.len(), 3);
    let secondary = after.iter().find(|row| row.url.as_str() == extra).unwrap();
    let unsupported_row = after
        .iter()
        .find(|row| row.url.as_str() == unsupported)
        .unwrap();
    let before = before
        .iter()
        .find(|row| row.url.as_str() == fixture.url)
        .unwrap();
    let after = after
        .iter()
        .find(|row| row.url.as_str() == fixture.url)
        .unwrap();
    let paused = paused
        .iter()
        .find(|row| row.url.as_str() == fixture.url)
        .unwrap();
    assert_eq!(secondary.status, TrackerStatus::NotContacted);
    assert_eq!(
        secondary.message.as_deref(),
        Some("private torrent: only the first tracker is announced")
    );
    assert_eq!(unsupported_row.status, TrackerStatus::NotContacted);
    assert_eq!(
        unsupported_row.message.as_deref(),
        Some("unsupported scheme")
    );
    assert!(error_observed && success);
    assert!(
        before
            .message
            .as_deref()
            .unwrap()
            .contains("fixture: not yet")
    );
    assert!(elapsed < Duration::from_secs(2));
    assert_eq!(after.peers, Some(1));
    assert_eq!(requests.len(), 2, "second force within 60 s is a no-op");
    assert!(requests[0].1);
    assert!(!requests[1].1, "forced event is none");
    assert_eq!(paused.status, after.status);
    assert_eq!(paused.last_announce_at, after.last_announce_at);
}

#[test]
fn tracker_stats_udp_force_wakes_error_backoff() {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let url = format!(
        "udp://{}/QUJDREVGR0hJSktMTU5PUFFS/announce?passkey=udp-fixture-secret",
        socket.local_addr().unwrap()
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let worker = thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            let mut packet = [0; 512];
            let (len, remote) = match socket.recv_from(&mut packet) {
                Ok(value) => value,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(error) => panic!("UDP fixture: {error}"),
            };
            assert!(len >= 16);
            let action = u32::from_be_bytes(packet[8..12].try_into().unwrap());
            let mut response = Vec::new();
            if action == 0 {
                response.extend(0u32.to_be_bytes());
                response.extend(&packet[12..16]);
                response.extend(123u64.to_be_bytes());
            } else {
                assert_eq!(action, 1);
                assert_eq!(len, 98);
                let first = {
                    let mut rows = observed.lock().unwrap();
                    rows.push(u32::from_be_bytes(packet[80..84].try_into().unwrap()));
                    rows.len() == 1
                };
                if first {
                    response.extend(3u32.to_be_bytes());
                    response.extend(&packet[12..16]);
                    response.extend(
                        b"fixture: not yet QUJDREVGR0hJSktMTU5PUFFS udp://other/QUJDREVGR0hJSktMTU5PUFFS/announce?passkey=udp-fixture-secret",
                    );
                } else {
                    response.extend(1u32.to_be_bytes());
                    response.extend(&packet[12..16]);
                    response.extend(600u32.to_be_bytes());
                    response.extend(0u32.to_be_bytes());
                    response.extend(1u32.to_be_bytes());
                    response.extend([127, 0, 0, 1, 0, 1]);
                }
            }
            socket.send_to(&response, remote).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();
    let (runtime, log, _trace) = traced_runtime();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let session = runtime
        .block_on(Session::new_with_opts(
            &cx,
            output.path().into(),
            SessionOptions {
                dht: None,
                disable_local_service_discovery: true,
                ..Default::default()
            },
        ))
        .unwrap();
    let mut meta = format!(
        "d8:announce{}:{}4:infod6:lengthi4096e4:name7:udp.bin12:piece lengthi4096e6:pieces20:",
        url.len(),
        url
    )
    .into_bytes();
    meta.extend([1u8; 20]);
    meta.extend(b"ee");
    let handle = runtime
        .block_on(session.add_torrent(&cx, AddTorrent::from_bytes(meta), None))
        .unwrap()
        .into_handle()
        .unwrap();
    let failed = wait(|| handle.tracker_stats()[0].status == TrackerStatus::Error);
    let error = handle.tracker_stats();
    let began = Instant::now();
    handle.announce_now();
    let ok = wait(|| handle.tracker_stats()[0].status == TrackerStatus::Ok);
    let elapsed = began.elapsed();
    let success = handle.tracker_stats();
    handle.announce_now();
    thread::sleep(Duration::from_millis(250));
    runtime.block_on(session.stop(&cx)).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(5)));
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    println!(
        "UDP-TRACKER error={failed} ok={ok} forced_ms={} requests={:?}",
        elapsed.as_millis(),
        requests.lock().unwrap()
    );
    assert_redacted(log);
    assert!(failed && ok && elapsed < Duration::from_secs(2));
    assert!(error[0].message.as_ref().unwrap().contains("?…"));
    assert!(
        !error[0]
            .message
            .as_ref()
            .unwrap()
            .contains("udp-fixture-secret")
    );
    assert_eq!(success[0].peers, Some(1));
    assert_eq!(*requests.lock().unwrap(), vec![2, 0]);
}
