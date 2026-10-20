//! Daemon-owned, job-scoped SOCKS5 endpoints. Only authenticated CONNECT is
//! supported (RFC 1928/1929); DNS and TCP use the app's bound transport.
//! Each runner owns its endpoint so cancellation also closes idle relays.
use super::*;
use crate::search::torznab::bound;
use ring::rand::{SecureRandom, SystemRandom};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicUsize;

const MAX_CONNECTIONS: usize = 32; // across all Nova jobs, including handshakes
const HANDSHAKE: Duration = Duration::from_secs(3);
// Bound DNS may try an unreachable primary for both A and AAAA before a
// working fallback. Match Nova's request timeout; cancellation stays prompt.
const CONNECT: Duration = Duration::from_secs(20);
const IDLE: Duration = Duration::from_secs(20);
const LIFETIME: Duration = Duration::from_secs(120);
static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);
static CREDENTIALS: OnceLock<std::result::Result<Credentials, &'static str>> = OnceLock::new();

struct Credentials {
    username: String,
    password: String,
}
impl Credentials {
    fn daemon() -> Result<&'static Self> {
        CREDENTIALS
            .get_or_init(|| {
                let mut random = [0; 32];
                SystemRandom::new()
                    .fill(&mut random)
                    .map_err(|_| "Nova proxy entropy unavailable")?;
                let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect();
                Ok(Self {
                    username: hex(&random[..16]),
                    password: hex(&random[16..]),
                })
            })
            .as_ref()
            .map_err(|error| anyhow!("{error}"))
    }
    fn matches(&self, username: &[u8], password: &[u8]) -> bool {
        // Always compare both fixed-length secrets; credentials never appear
        // in protocol errors or logs.
        let equal = |a: &[u8], b: &[u8]| {
            a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
        };
        equal(username, self.username.as_bytes()) & equal(password, self.password.as_bytes())
    }
}

pub(super) struct Proxy {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    credentials: &'static Credentials,
}
impl Proxy {
    pub(super) fn configured() -> Result<Option<Self>> {
        let Some(context) =
            bound::Context::for_config(&crate::outbound::network_config()?, "Nova search VPN")?
        else {
            return Ok(None);
        };
        Self::start(context).map(Some)
    }
    fn start(context: bound::Context) -> Result<Self> {
        let credentials = Credentials::daemon()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let closing = stopped.clone();
        let worker = thread::Builder::new()
            .name("nova-proxy".into())
            .spawn(move || {
                let context = Arc::new(context);
                let mut workers: Vec<JoinHandle<()>> = Vec::new();
                while !closing.load(Ordering::Acquire) {
                    // Reap completed workers continuously; no unbounded history.
                    let mut index = 0;
                    while index < workers.len() {
                        if workers[index].is_finished() {
                            let _ = workers.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                    match listener.accept() {
                        Ok((client, _)) => {
                            let Some(permit) = Permit::take() else {
                                continue;
                            };
                            let context = context.clone();
                            let closing = closing.clone();
                            if let Ok(worker) = thread::Builder::new()
                                .name("nova-relay".into())
                                .spawn(move || {
                                    let _permit = permit;
                                    let _ = serve(client, &context, &closing, credentials);
                                })
                            {
                                workers.push(worker);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
                drop(listener);
                closing.store(true, Ordering::Release);
                for worker in workers {
                    let _ = worker.join();
                }
            })?;
        Ok(Self {
            address,
            stopped,
            worker: Some(worker),
            credentials,
        })
    }
    pub(super) fn url(&self) -> String {
        format!(
            "socks5h://{}:{}@{}",
            self.credentials.username, self.credentials.password, self.address
        )
    }
    pub(super) fn redact(&self, text: &str) -> String {
        String::from_utf8(self.redact_bytes(text.as_bytes())).expect("redaction preserves UTF-8")
    }
    pub(super) fn redact_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let mut result = bytes.to_vec();
        for secret in [
            self.url(),
            self.credentials.username.clone(),
            self.credentials.password.clone(),
        ] {
            let mut redacted = Vec::new();
            let mut remaining = result.as_slice();
            while let Some(index) = remaining
                .windows(secret.len())
                .position(|part| part == secret.as_bytes())
            {
                redacted.extend_from_slice(&remaining[..index]);
                redacted.extend_from_slice(b"[redacted]");
                remaining = &remaining[index + secret.len()..];
            }
            redacted.extend_from_slice(remaining);
            result = redacted;
        }
        result
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Permit;
impl Permit {
    fn take() -> Option<Self> {
        CONNECTIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_CONNECTIONS).then_some(count + 1)
            })
            .ok()
            .map(|_| Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        CONNECTIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Nova VPN proxy refused connection",
    )
}
fn read(
    client: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
    stopped: &AtomicBool,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if stopped.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(refused());
        }
        match client.read(&mut bytes[offset..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn bytes<const N: usize>(
    client: &mut TcpStream,
    deadline: Instant,
    stopped: &AtomicBool,
) -> io::Result<[u8; N]> {
    let mut value = [0; N];
    read(client, &mut value, deadline, stopped)?;
    Ok(value)
}
fn field(client: &mut TcpStream, deadline: Instant, stopped: &AtomicBool) -> io::Result<Vec<u8>> {
    let [length] = bytes(client, deadline, stopped)?;
    if length == 0 {
        return Err(refused());
    }
    let mut value = vec![0; length as usize];
    read(client, &mut value, deadline, stopped)?;
    Ok(value)
}
fn reply(client: &mut TcpStream, status: u8, address: SocketAddr) -> io::Result<()> {
    let mut response = vec![5, status, 0];
    match address.ip() {
        std::net::IpAddr::V4(ip) => {
            response.push(1);
            response.extend(ip.octets());
        }
        std::net::IpAddr::V6(ip) => {
            response.push(4);
            response.extend(ip.octets());
        }
    }
    response.extend(address.port().to_be_bytes());
    client.write_all(&response)
}
fn serve(
    mut client: TcpStream,
    context: &bound::Context,
    stopped: &AtomicBool,
    credentials: &Credentials,
) -> io::Result<()> {
    client.set_read_timeout(Some(Duration::from_millis(100)))?;
    client.set_write_timeout(Some(Duration::from_millis(100)))?;
    let deadline = Instant::now() + HANDSHAKE;
    if bytes::<1>(&mut client, deadline, stopped)? != [5] {
        return Err(refused());
    }
    let methods = field(&mut client, deadline, stopped)?;
    if !methods.contains(&2) {
        client.write_all(&[5, 255])?;
        return Err(refused());
    }
    client.write_all(&[5, 2])?;
    if bytes::<1>(&mut client, deadline, stopped)? != [1] {
        return Err(refused());
    }
    let username = field(&mut client, deadline, stopped)?;
    let password = field(&mut client, deadline, stopped)?;
    let authenticated = credentials.matches(&username, &password);
    client.write_all(&[1, u8::from(!authenticated)])?;
    if !authenticated {
        return Err(refused());
    }
    let [version, command, reserved, kind] = bytes(&mut client, deadline, stopped)?;
    let nowhere = SocketAddr::from(([0, 0, 0, 0], 0));
    if version != 5 || command != 1 || reserved != 0 {
        reply(&mut client, 7, nowhere)?;
        return Err(refused());
    }
    let host = match kind {
        1 => Ipv4Addr::from(bytes::<4>(&mut client, deadline, stopped)?).to_string(),
        4 => Ipv6Addr::from(bytes::<16>(&mut client, deadline, stopped)?).to_string(),
        3 => String::from_utf8(field(&mut client, deadline, stopped)?).map_err(|_| refused())?,
        _ => {
            reply(&mut client, 8, nowhere)?;
            return Err(refused());
        }
    };
    let port = u16::from_be_bytes(bytes(&mut client, deadline, stopped)?);
    let (remote, _owner) = match context.connect(&host, port, CONNECT, stopped) {
        Ok(connection) => connection,
        Err(error) => {
            reply(&mut client, 4, nowhere)?;
            return Err(error);
        }
    };
    context.check_ready()?;
    reply(&mut client, 0, remote.local_addr()?)?;
    relay(client, remote, context, stopped)
}

struct Pipe {
    buffer: [u8; 16384],
    start: usize,
    end: usize,
    eof: bool,
}
impl Pipe {
    fn new() -> Self {
        Self {
            buffer: [0; 16384],
            start: 0,
            end: 0,
            eof: false,
        }
    }
    fn pump(&mut self, source: &mut TcpStream, dest: &mut TcpStream) -> io::Result<bool> {
        let mut moved = false;
        if self.start == self.end && !self.eof {
            match source.read(&mut self.buffer) {
                Ok(count) => {
                    self.start = 0;
                    self.end = count;
                    self.eof = count == 0;
                    moved = count > 0;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        if self.start < self.end {
            match dest.write(&self.buffer[self.start..self.end]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => {
                    self.start += count;
                    moved = true;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        if self.eof && self.start == self.end {
            let _ = dest.shutdown(Shutdown::Write);
        }
        Ok(moved)
    }
    fn events(&self, reverse: &Self) -> i16 {
        (if !self.eof && self.start == self.end {
            libc::POLLIN
        } else {
            0
        }) | (if reverse.start < reverse.end {
            libc::POLLOUT
        } else {
            0
        })
    }
}
fn relay(
    mut client: TcpStream,
    mut remote: TcpStream,
    context: &bound::Context,
    stopped: &AtomicBool,
) -> io::Result<()> {
    client.set_nonblocking(true)?;
    remote.set_nonblocking(true)?;
    let mut outgoing = Pipe::new();
    let mut incoming = Pipe::new();
    let began = Instant::now();
    let mut last_work = began;
    while !stopped.load(Ordering::Acquire)
        && began.elapsed() < LIFETIME
        && last_work.elapsed() < IDLE
    {
        context.check_ready()?;
        let sent = outgoing.pump(&mut client, &mut remote)?;
        let received = incoming.pump(&mut remote, &mut client)?;
        if outgoing.eof && incoming.eof {
            break;
        }
        if sent || received {
            last_work = Instant::now();
            continue;
        }
        let mut fds = [
            libc::pollfd {
                fd: client.as_raw_fd(),
                events: outgoing.events(&incoming),
                revents: 0,
            },
            libc::pollfd {
                fd: remote.as_raw_fd(),
                events: incoming.events(&outgoing),
                revents: 0,
            },
        ];
        for fd in &mut fds {
            if fd.events == 0 {
                fd.fd = -1;
            }
        }
        // SAFETY: both descriptors and the two-element array remain live.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 100) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    let _ = remote.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
mod tests;
