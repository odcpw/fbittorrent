use super::*;
use crate::outbound::with_interface;
use crate::search::tests::{scratch, session};
use std::sync::atomic::AtomicUsize;

struct Origin {
    address: SocketAddr,
    hits: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Origin {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok((mut socket, _)) = listener.accept() {
                    count.fetch_add(1, Ordering::AcqRel);
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let _ = socket.read(&mut [0; 2048]);
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                } else {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            address,
            hits,
            stopped,
            worker: Some(worker),
        }
    }
}
impl Drop for Origin {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn proxy() -> Proxy {
    with_interface(Some("lo"), Proxy::configured)
        .unwrap()
        .unwrap()
}

#[test]
fn outbound_socket_really_has_the_kernel_interface_binding() {
    let context = bound::Context::for_config(
        &crate::config::Config {
            network_interface: Some("lo".into()),
            ..Default::default()
        },
        "Nova search VPN",
    )
    .unwrap()
    .unwrap();
    let origin = Origin::new();
    let (socket, _owner) = context
        .connect("127.0.0.1", origin.address.port(), CONNECT)
        .unwrap();
    let mut device = [0u8; libc::IFNAMSIZ];
    let mut length = device.len() as libc::socklen_t;
    // SAFETY: this reads the owned socket option into a correctly sized buffer.
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            device.as_mut_ptr().cast(),
            &mut length,
        )
    };
    assert_eq!(result, 0);
    assert_eq!(
        &device[..length as usize],
        b"lo\0",
        "CONNECT must use a kernel-bound socket"
    );
}
fn client(proxy: &Proxy, password: &str) -> TcpStream {
    let mut client = TcpStream::connect(proxy.address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.write_all(&[5, 1, 2]).unwrap();
    let mut answer = [0; 2];
    client.read_exact(&mut answer).unwrap();
    assert_eq!(answer, [5, 2]);
    let username = proxy.credentials.username.as_bytes();
    let mut auth = vec![1, username.len() as u8];
    auth.extend(username);
    auth.push(password.len() as u8);
    auth.extend(password.as_bytes());
    client.write_all(&auth).unwrap();
    client
}

#[test]
fn proxy_requires_credentials_and_rejects_non_connect_without_outbound_traffic() {
    let proxy = proxy();
    let origin = Origin::new();
    let mut anonymous = TcpStream::connect(proxy.address).unwrap();
    anonymous
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    anonymous.write_all(&[5, 1, 0]).unwrap();
    let mut answer = [0; 2];
    anonymous.read_exact(&mut answer).unwrap();
    assert_eq!(answer, [5, 255]);
    let mut wrong = client(&proxy, "wrong");
    wrong.read_exact(&mut answer).unwrap();
    assert_eq!(answer, [1, 1]);
    for command in [2, 3] {
        let mut socket = client(&proxy, &proxy.credentials.password);
        socket.read_exact(&mut answer).unwrap();
        assert_eq!(answer, [1, 0]);
        let mut request = vec![5, command, 0, 1, 127, 0, 0, 1];
        request.extend(origin.address.port().to_be_bytes());
        socket.write_all(&request).unwrap();
        let mut reply = [0; 10];
        socket.read_exact(&mut reply).unwrap();
        assert_eq!(reply[1], 7);
    }
    drop(proxy);
    assert_eq!(
        origin.hits.load(Ordering::Acquire),
        0,
        "unauthorized requests never connect"
    );
}

#[test]
fn proxy_drop_closes_listener_handshakes_and_active_relays() {
    let proxy = proxy();
    let address = proxy.address;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut socket = client(&proxy, &proxy.credentials.password);
    let mut auth = [0; 2];
    socket.read_exact(&mut auth).unwrap();
    assert_eq!(auth, [1, 0]);
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend(b"localhost");
    request.extend(listener.local_addr().unwrap().port().to_be_bytes());
    socket.write_all(&request).unwrap();
    let mut reply = [0; 10];
    socket.read_exact(&mut reply).unwrap();
    assert_eq!(reply[1], 0);
    let (mut remote, _) = listener.accept().unwrap();
    remote
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut unfinished = TcpStream::connect(address).unwrap();
    unfinished
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    unfinished.write_all(&[5]).unwrap();
    let began = Instant::now();
    drop(proxy);
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(remote.read(&mut [0]).unwrap(), 0);
    assert_eq!(socket.read(&mut [0]).unwrap(), 0);
    assert!(matches!(unfinished.read(&mut [0]), Ok(0) | Err(_)));
    assert!(TcpStream::connect(address).is_err());
}

const PROVIDER: &str = r#"
import os, socket, urllib.request, urllib.parse
import helpers
from novaprinter import prettyPrinter

class vpnprobe:
    name = 'VPN probe'
    url = 'https://vpn-probe.invalid'
    supported_categories = {'all': '0'}

    def search(self, query, cat='all'):
        host, port, expected = urllib.parse.unquote(query).split(':')
        port = int(port)
        assert (os.environ.get('FBT_NOVA_BOUND') == '1') == (expected == 'bound')
        if os.environ.get('FBT_NOVA_BOUND') == '1':
            # The Python resolver must never be used, including indirectly by
            # urllib/create_connection. This is not a fake proxy: the real
            # daemon-owned SOCKS listener and bound connector handle requests.
            import _socket
            def forbidden(*args, **kwargs):
                raise AssertionError('local DNS resolver used')
            _socket.getaddrinfo = forbidden
            helpers.enable_socks_proxy(False)  # cannot disable owner's policy
            for lookup in [socket.gethostbyname, socket.gethostbyname_ex,
                           socket.gethostbyaddr, socket.getnameinfo]:
                try:
                    lookup(host)
                    raise AssertionError('DNS bypass')
                except socket.gaierror:
                    pass
            try:
                socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                raise AssertionError('UDP bypass')
            except OSError:
                pass
        # Explicit getaddrinfo must preserve a name, not leak its DNS locally.
        address = socket.getaddrinfo(host, port, family=socket.AF_INET, type=socket.SOCK_STREAM)[0][4]
        with socket.socket() as stream:
            stream.connect(address)
            stream.sendall(b'GET /raw HTTP/1.0\r\n\r\n')
            response = b''
            while chunk := stream.recv(4096):
                response += chunk
            assert response.endswith(b'ok'), response
        with urllib.request.urlopen(f'http://{host}:{port}/urllib', timeout=3) as response:
            assert response.read() == b'ok'
        prettyPrinter({'link': 'magnet:?xt=urn:btih:' + 'a'*40,
                       'name': 'VPN urllib and raw worked', 'size': '123',
                       'seeds': 1, 'leech': 0, 'engine_url': self.url})

    def download_torrent(self, parameter):
        with urllib.request.urlopen(parameter, timeout=3) as response:
            assert response.read() == b'ok'
        print('magnet:?xt=urn:btih:' + 'a'*40)
"#;

#[test]
fn nova_urllib_raw_socket_and_download_hooks_use_bound_proxy_and_unbound_still_works() {
    let _session = session();
    let (_scratch, store) = scratch();
    store.ensure_runtime().unwrap();
    std::fs::write(store.runtime().join("engines/vpnprobe.py"), PROVIDER).unwrap();
    let origin = Origin::new();
    for interface in [Some("lo"), None] {
        let id = with_interface(interface, || {
            jobs::start_in(
                &store,
                &format!(
                    "localhost:{}:{}",
                    origin.address.port(),
                    if interface.is_some() {
                        "bound"
                    } else {
                        "unbound"
                    }
                ),
            )
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let result = results(id, -1, 0).unwrap();
            if result.status == "Stopped" {
                assert!(result
                    .results
                    .iter()
                    .any(|row| row.file_name == "VPN urllib and raw worked"));
                assert!(result.error.is_none(), "{:?}", result.error);
                break;
            }
            assert!(Instant::now() < deadline, "provider did not finish");
            thread::sleep(Duration::from_millis(20));
        }
        delete(id).unwrap();
        let download = with_interface(interface, || {
            download::resolve_download_in(
                &store,
                &format!("http://localhost:{}/details", origin.address.port()),
                "vpnprobe",
                5,
            )
        })
        .unwrap();
        assert!(
            matches!(download, download::ResolvedDownload::Url(link) if link.starts_with("magnet:"))
        );
    }
    assert_eq!(
        origin.hits.load(Ordering::Acquire),
        6,
        "two fetches and a download hook in each mode"
    );
    assert!(jobs().lock().unwrap().processes.is_empty());
}

#[test]
fn missing_interface_refuses_search_before_any_provider_or_direct_connection() {
    let _session = session();
    let (_scratch, store) = scratch();
    store.ensure_runtime().unwrap();
    std::fs::write(store.runtime().join("engines/vpnprobe.py"), PROVIDER).unwrap();
    let origin = Origin::new();
    let error = with_interface(Some("fbmissing0"), || {
        jobs::start_in(&store, &format!("127.0.0.1:{}", origin.address.port()))
    })
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("Nova search VPN interface unavailable"),
        "{error:#}"
    );
    assert!(jobs().lock().unwrap().processes.is_empty());
    assert_eq!(origin.hits.load(Ordering::Acquire), 0);
}

#[test]
fn nova_proxy_failure_never_falls_back_and_secrets_are_redacted() {
    let _session = session();
    let (_scratch, store) = scratch();
    store.ensure_runtime().unwrap();
    let origin = Origin::new();
    let mut command = python(&store.runtime());
    command
        .args([
            "-c",
            r#"
import helpers, os, socket, urllib.request, urllib.parse
helpers.enable_socks_proxy(True)
p = urllib.parse.urlsplit(os.environ['qbt_socks_proxy'])
import socks
# Simulate invalid credentials in both library entry paths.
socks.socksocket.default_proxy = (socks.SOCKS5, p.hostname, p.port, True, b'bad', b'bad')
for raw in [False, True]:
    try:
        if raw:
            with socks.socksocket() as s:
                s.connect(('localhost', int(os.environ['ORIGIN_PORT'])))
        else:
            with socks.create_connection(('localhost', int(os.environ['ORIGIN_PORT'])),
                 timeout=2, proxy_type=socks.SOCKS5, proxy_addr=p.hostname, proxy_port=p.port,
                 proxy_username='bad', proxy_password='bad'):
                pass
        raise AssertionError('authentication bypass')
    except socks.ProxyError:
        pass
print(os.environ['qbt_socks_proxy'])
print(p.password)
"#,
        ])
        .env("ORIGIN_PORT", origin.address.port().to_string());
    let output = with_interface(Some("lo"), || {
        process::capture(command, Duration::from_secs(5))
    })
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let credentials = Credentials::daemon().unwrap();
    assert!(!output.contains(&credentials.username));
    assert!(!output.contains(&credentials.password));
    assert_eq!(origin.hits.load(Ordering::Acquire), 0);
}
