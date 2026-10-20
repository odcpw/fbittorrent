//! Private loopback HTTP ranges over rqbit's verified streaming reader.
//! Tokens authorize only one selected video, never arbitrary filesystem paths.
use super::{slot::Slot, Daemon};
use crate::engine::playback::Video;
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_TARGETS: usize = 32;
const MAX_CLIENTS: usize = 8;
struct Target {
    hash: String,
    video: Video,
    epoch: u64,
    created: Instant,
}
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    targets: Mutex<BTreeMap<String, Arc<Target>>>,
}
struct Server {
    shared: Arc<Shared>,
    address: SocketAddr,
    task: thread::JoinHandle<()>,
}
#[derive(Default)]
pub(crate) struct Service {
    owner: Mutex<Weak<Daemon>>,
    server: Mutex<Option<Server>>,
}

impl Service {
    pub fn init(&self, owner: Weak<Daemon>) {
        *self.owner.lock().unwrap() = owner;
    }
    pub fn target(&self, hash: &str, video: Video, epoch: u64) -> Result<String> {
        let mut server = self.server.lock().unwrap();
        anyhow::ensure!(
            self.owner
                .lock()
                .unwrap()
                .upgrade()
                .is_some_and(|o| !o.is_stopping()),
            "playback stopped"
        );
        if server.is_none() {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
            listener.set_nonblocking(true)?;
            let address = listener.local_addr()?;
            let shared = Arc::new(Shared::default());
            let state = shared.clone();
            let owner = self.owner.lock().unwrap().clone();
            let task = thread::Builder::new()
                .name("fbittorrent-playback".into())
                .spawn(move || accept(listener, address, state, owner))?;
            *server = Some(Server {
                shared,
                address,
                task,
            });
        }
        let server = server.as_ref().unwrap();
        let mut targets = server.shared.targets.lock().unwrap();
        targets.retain(|_, target| {
            target.epoch == epoch && target.created.elapsed() < Duration::from_secs(12 * 3600)
        });
        let token = if let Some((token, _)) = targets
            .iter()
            .find(|(_, t)| t.hash == hash && t.video.file == video.file)
        {
            token.clone()
        } else {
            if targets.len() >= MAX_TARGETS {
                let oldest = targets
                    .iter()
                    .filter(|(_, t)| Arc::strong_count(t) == 1)
                    .min_by_key(|(_, t)| t.created)
                    .map(|(key, _)| key.clone())
                    .context("too many active playback targets")?;
                targets.remove(&oldest);
            }
            let mut random = [0u8; 16];
            std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
            let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
            targets.insert(
                token.clone(),
                Arc::new(Target {
                    hash: hash.into(),
                    video,
                    epoch,
                    created: Instant::now(),
                }),
            );
            token
        };
        Ok(format!("http://{}/media/{token}", server.address))
    }
    pub fn stop(&self) {
        if let Some(server) = self.server.lock().unwrap().take() {
            server.shared.stop.store(true, Ordering::Release);
            let _ = server.task.join();
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept(listener: TcpListener, address: SocketAddr, shared: Arc<Shared>, owner: Weak<Daemon>) {
    let mut clients: Vec<thread::JoinHandle<()>> = Vec::new();
    while !shared.stop.load(Ordering::Acquire) {
        let mut i = 0;
        while i < clients.len() {
            if clients[i].is_finished() {
                let _ = clients.swap_remove(i).join();
            } else {
                i += 1;
            }
        }
        match listener.accept() {
            Ok((socket, peer)) if peer.ip().is_loopback() && clients.len() < MAX_CLIENTS => {
                let shared = shared.clone();
                let owner = owner.clone();
                if let Ok(task) = thread::Builder::new()
                    .name("fbittorrent-media-read".into())
                    .spawn(move || {
                        let _ = serve(socket, address, &shared, &owner);
                    })
                {
                    clients.push(task);
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(_) => break,
        }
    }
    for client in clients {
        let _ = client.join();
    }
}

fn reject(socket: &mut TcpStream, code: &str) -> Result<()> {
    write!(
        socket,
        "HTTP/1.1 {code}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    Ok(())
}
fn serve(
    mut socket: TcpStream,
    address: SocketAddr,
    shared: &Shared,
    owner: &Weak<Daemon>,
) -> Result<()> {
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    socket.set_write_timeout(Some(Duration::from_millis(250)))?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(
            !shared.stop.load(Ordering::Acquire) && header.len() < 8192,
            "invalid media request"
        );
        let mut byte = [0];
        anyhow::ensure!(socket.read(&mut byte)? == 1, "player disconnected");
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header)?;
    let mut lines = header.split("\r\n");
    let request: Vec<_> = lines
        .next()
        .unwrap_or("")
        .split_ascii_whitespace()
        .collect();
    if request.len() != 3 || !matches!(request[0], "GET" | "HEAD") {
        return reject(&mut socket, "400 Bad Request");
    }
    let mut range_header = None;
    let mut host = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            return reject(&mut socket, "400 Bad Request");
        };
        if key.eq_ignore_ascii_case("origin") {
            return reject(&mut socket, "403 Forbidden");
        }
        if key.eq_ignore_ascii_case("host") {
            host = value.trim() == address.to_string();
        }
        if key.eq_ignore_ascii_case("range") {
            if range_header.replace(value.trim()).is_some() {
                return reject(&mut socket, "416 Range Not Satisfiable");
            }
        }
    }
    if !host {
        return reject(&mut socket, "403 Forbidden");
    }
    let Some(token) = request[1]
        .strip_prefix("/media/")
        .filter(|t| t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return reject(&mut socket, "404 Not Found");
    };
    let target = shared.targets.lock().unwrap().get(token).cloned();
    let Some(target) = target else {
        return reject(&mut socket, "404 Not Found");
    };
    let Some(owner) = owner.upgrade().filter(|o| !o.is_stopping()) else {
        return reject(&mut socket, "503 Service Unavailable");
    };
    let dispatcher = {
        let slot = owner.slot.read().unwrap_or_else(|e| e.into_inner());
        match &slot.slot {
            Slot::Running(app) if slot.engine_epoch == target.epoch => app.dispatcher(),
            _ => return reject(&mut socket, "503 Service Unavailable"),
        }
    };
    let Some(range) = byte_range(range_header, target.video.len) else {
        write!(socket, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.video.len)?;
        return Ok(());
    };
    let _presence = (request[0] == "GET")
        .then(|| super::policy::Playback::start(owner.policy.clone()));
    write!(socket, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        if range_header.is_some() {"206 Partial Content"} else {"200 OK"}, target.video.mime, range.end - range.start)?;
    if range_header.is_some() {
        write!(
            socket,
            "Content-Range: bytes {}-{}/{}\r\n",
            range.start,
            range.end - 1,
            target.video.len
        )?;
    }
    write!(socket, "\r\n")?;
    if request[0] == "GET" && !shared.stop.load(Ordering::Acquire) {
        dispatcher.stream_video(
            &target.hash,
            &target.video,
            range,
            &mut socket,
            &shared.stop,
        )?;
    }
    Ok(())
}

fn byte_range(header: Option<&str>, len: u64) -> Option<std::ops::Range<u64>> {
    if len == 0 {
        return None;
    }
    let Some(header) = header else {
        return Some(0..len);
    };
    let (start, end) = header.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        return (suffix > 0).then_some(len.saturating_sub(suffix)..len);
    }
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() {
        len
    } else {
        end.parse::<u64>().ok()?.saturating_add(1).min(len)
    };
    (start < end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_are_bounded_to_the_authorized_video() {
        for (raw, expected) in [
            (None, Some(0..100)),
            (Some("bytes=10-19"), Some(10..20)),
            (Some("bytes=90-"), Some(90..100)),
            (Some("bytes=-12"), Some(88..100)),
            (Some("bytes=90-999999"), Some(90..100)),
            (Some("bytes=100-"), None),
            (Some("bytes=9-2"), None),
            (Some("bytes=0-1,4-5"), None),
            (Some("bytes=-0"), None),
        ] {
            assert_eq!(byte_range(raw, 100), expected, "{raw:?}");
        }
        assert_eq!(byte_range(None, 0), None);
    }
}
