//! Loopback seed peers and HTTP tracker.
//!
//! Every listener seeds every hosted torrent. Blocks are generated on demand;
//! responses are batched while further requests are already buffered, so one
//! seed thread per connection outpaces the engine under test. Plaintext
//! BitTorrent only: an MSE handshake is closed so the client falls back.

use crate::bencode::{self, put_bytes, put_int};
use crate::payload;
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OUR_UT_METADATA: u8 = 3;
const MAX_BLOCK: u32 = 128 * 1024;
const FLUSH_AT: usize = 1 << 20;

#[derive(Default)]
struct Stats {
    bytes: AtomicU64,
    blocks: AtomicU64,
    first_ns: AtomicU64,
    last_ns: AtomicU64,
    connections: AtomicU64,
    metadata_pieces: AtomicU64,
    uploaded_bytes: AtomicU64,
    unchokes: AtomicU64,
    probes: AtomicU64,
    first_requests: Mutex<Vec<u32>>,
}

pub struct Hosted {
    name: String,
    info_hash: [u8; 20],
    info: Vec<u8>,
    size: u64,
    piece_len: u64,
    pieces: u32,
    key: u64,
    prefix: Vec<u8>,
    stats: Stats,
}

impl Hosted {
    pub fn load(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
        let parsed = bencode::parse(&data)?;
        let (info, info_span) = parsed.root.get(&data, b"info").ok_or("no info dict")?;
        let key_hex = parsed
            .root
            .get(&data, b"x-fbt-key")
            .and_then(|(v, _)| v.bytes(&data))
            .ok_or("not a fbt-bench torrent (no x-fbt-key)")?;
        let key = u64::from_str_radix(std::str::from_utf8(key_hex).map_err(|e| e.to_string())?, 16)
            .map_err(|e| e.to_string())?;
        let field = |name: &[u8]| info.get(&data, name).and_then(|(v, _)| v.int());
        let size = if let Some(size) = field(b"length") {
            u64::try_from(size).map_err(|_| "negative length")?
        } else {
            let Some((bencode::Value::List(files), _)) = info.get(&data, b"files") else {
                return Err("no length or files".into());
            };
            files.iter().try_fold(0u64, |total, (file, _)| {
                let size = file.get(&data, b"length").and_then(|(v, _)| v.int())
                    .and_then(|n| u64::try_from(n).ok()).ok_or("invalid file length")?;
                total.checked_add(size).ok_or("file lengths overflow")
            })?
        };
        let piece_len = field(b"piece length").ok_or("no piece length")? as u64;
        let name = info
            .get(&data, b"name")
            .and_then(|(v, _)| v.bytes(&data))
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default();
        let info = data[info_span].to_vec();
        Ok(Self {
            name,
            info_hash: crate::sha1::digest(&info),
            info,
            size,
            piece_len,
            pieces: size.div_ceil(piece_len) as u32,
            key,
            prefix: parsed.root.get(&data, b"x-fbt-prefix").and_then(|(v, _)| v.bytes(&data)).unwrap_or_default().to_vec(),
            stats: Stats::default(),
        })
    }

    fn piece_size(&self, index: u32) -> u64 {
        (self.size - u64::from(index) * self.piece_len).min(self.piece_len)
    }
}

pub struct Server {
    torrents: Vec<Arc<Hosted>>,
    by_hash: HashMap<[u8; 20], usize>,
    peers: Vec<SocketAddr>,
    rate: u64,
    probe: bool,
    epoch: Instant,
    rejected: AtomicU64,
}

fn now_ns(epoch: Instant) -> u64 {
    (epoch.elapsed().as_nanos() as u64).max(1)
}

fn message(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32 + 1).to_be_bytes());
    out.push(id);
    out.extend_from_slice(body);
}

fn extended(out: &mut Vec<u8>, ext: u8, body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32 + 2).to_be_bytes());
    out.push(20);
    out.push(ext);
    out.extend_from_slice(body);
}

impl Server {
    pub fn new(torrents: Vec<Hosted>, rate: u64, probe: bool) -> Self {
        let torrents: Vec<_> = torrents.into_iter().map(Arc::new).collect();
        let by_hash = torrents.iter().enumerate().map(|(i, t)| (t.info_hash, i)).collect();
        Self { torrents, by_hash, peers: Vec::new(), rate, probe, epoch: Instant::now(), rejected: AtomicU64::new(0) }
    }

    pub fn run(mut self, peer_count: usize, tracker_port: u16, ready: Option<&Path>) -> std::io::Result<()> {
        let listeners: Vec<TcpListener> =
            (0..peer_count).map(|_| TcpListener::bind("127.0.0.1:0")).collect::<Result<_, _>>()?;
        self.peers = listeners.iter().map(|l| l.local_addr()).collect::<Result<_, _>>()?;
        let tracker = TcpListener::bind(("127.0.0.1", tracker_port))?;
        let server = Arc::new(self);
        for listener in listeners {
            let server = server.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let server = server.clone();
                    std::thread::spawn(move || {
                        let _ = server.serve(stream);
                    });
                }
            });
        }
        if let Some(path) = ready {
            let ports: Vec<String> = server.peers.iter().map(|a| a.port().to_string()).collect();
            let text = format!(
                "{{\"pid\":{},\"tracker_port\":{},\"peer_ports\":[{}],\"torrents\":{}}}\n",
                std::process::id(),
                tracker.local_addr()?.port(),
                ports.join(","),
                server.torrents.len()
            );
            let partial = path.with_extension("partial");
            std::fs::write(&partial, text)?;
            std::fs::rename(partial, path)?;
        }
        for stream in tracker.incoming().flatten() {
            if server.tracker(stream)? {
                return Ok(());
            }
        }
        Ok(())
    }

    fn stats_json(&self) -> String {
        let rows: Vec<String> = self
            .torrents
            .iter()
            .map(|t| {
                let s = &t.stats;
                format!(
                    "{{\"info_hash\":\"{}\",\"name\":{:?},\"bytes\":{},\"blocks\":{},\"first_ns\":{},\"last_ns\":{},\"connections\":{},\"metadata_pieces\":{},\"uploaded_bytes\":{},\"unchokes\":{},\"probes\":{},\"first_requests\":{:?}}}",
                    payload::hex(&t.info_hash), t.name, s.bytes.load(Relaxed), s.blocks.load(Relaxed),
                    s.first_ns.load(Relaxed), s.last_ns.load(Relaxed), s.connections.load(Relaxed),
                    s.metadata_pieces.load(Relaxed), s.uploaded_bytes.load(Relaxed),
                    s.unchokes.load(Relaxed), s.probes.load(Relaxed), *s.first_requests.lock().unwrap()
                )
            })
            .collect();
        format!(
            "{{\"now_ns\":{},\"rejected_handshakes\":{},\"torrents\":[{}]}}",
            now_ns(self.epoch),
            self.rejected.load(Relaxed),
            rows.join(",")
        )
    }

    /// One HTTP request per connection. Returns true for /quit.
    fn tracker(&self, mut stream: TcpStream) -> std::io::Result<bool> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16384 {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let line = String::from_utf8_lossy(&request);
        let path = line.split_whitespace().nth(1).unwrap_or("/");
        let (body, kind, quit) = if path.starts_with("/announce") {
            let mut peers = Vec::with_capacity(self.peers.len() * 6);
            for addr in &self.peers {
                if let SocketAddr::V4(v4) = addr {
                    peers.extend_from_slice(&v4.ip().octets());
                    peers.extend_from_slice(&v4.port().to_be_bytes());
                }
            }
            let mut body = b"d".to_vec();
            put_bytes(&mut body, b"complete");
            put_int(&mut body, self.peers.len() as i64);
            put_bytes(&mut body, b"incomplete");
            put_int(&mut body, 0);
            put_bytes(&mut body, b"interval");
            put_int(&mut body, 1800);
            put_bytes(&mut body, b"peers");
            put_bytes(&mut body, &peers);
            body.push(b'e');
            (body, "text/plain", false)
        } else if path.starts_with("/stats") {
            (self.stats_json().into_bytes(), "application/json", false)
        } else if path.starts_with("/quit") {
            (b"bye".to_vec(), "text/plain", true)
        } else {
            (b"not found".to_vec(), "text/plain", false)
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)?;
        Ok(quit)
    }

    fn serve(&self, stream: TcpStream) -> std::io::Result<()> {
        stream.set_nodelay(true)?;
        let mut reader = BufReader::with_capacity(256 * 1024, stream.try_clone()?);
        let mut writer = stream;
        let mut hello = [0u8; 68];
        reader.read_exact(&mut hello)?;
        let torrent = match (&hello[..20] == b"\x13BitTorrent protocol")
            .then(|| self.by_hash.get(&hello[28..48]))
            .flatten()
        {
            Some(index) => self.torrents[*index].clone(),
            None => {
                self.rejected.fetch_add(1, Relaxed);
                return Ok(());
            }
        };
        torrent.stats.connections.fetch_add(1, Relaxed);
        let peer_extended = hello[25] & 0x10 != 0;
        let mut out = Vec::with_capacity(FLUSH_AT + 2 * MAX_BLOCK as usize);
        out.extend_from_slice(b"\x13BitTorrent protocol");
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0x10, 0, 0]);
        out.extend_from_slice(&torrent.info_hash);
        out.extend_from_slice(b"-FB0BEN-");
        out.extend_from_slice(format!("{:06}{:06}", std::process::id() % 1_000_000, writer.local_addr()?.port()).as_bytes());
        if peer_extended {
            let mut hs = b"d".to_vec();
            put_bytes(&mut hs, b"m");
            hs.push(b'd');
            put_bytes(&mut hs, b"ut_metadata");
            put_int(&mut hs, i64::from(OUR_UT_METADATA));
            hs.push(b'e');
            put_bytes(&mut hs, b"metadata_size");
            put_int(&mut hs, torrent.info.len() as i64);
            put_bytes(&mut hs, b"v");
            put_bytes(&mut hs, b"fbt-bench-seed");
            hs.push(b'e');
            extended(&mut out, 0, &hs);
        }
        let mut bitfield = vec![0xffu8; (torrent.pieces as usize).div_ceil(8)];
        if torrent.pieces % 8 != 0 {
            *bitfield.last_mut().unwrap() = 0xffu8 << (8 - torrent.pieces % 8);
        }
        message(&mut out, 5, &bitfield);
        message(&mut out, 1, &[]);
        message(&mut out, 2, &[]); // Interested: client must keep payload uploads choked.
        writer.write_all(&out)?;
        out.clear();

        let started = Instant::now();
        let mut sent: u64 = 0;
        let mut probed = false;
        let mut peer_ut_metadata: Option<u8> = None;
        let mut msg = Vec::with_capacity(64);
        loop {
            if reader.buffer().is_empty() && !out.is_empty() {
                self.flush(&mut writer, &mut out, started, &mut sent)?;
            }
            let mut len = [0u8; 4];
            reader.read_exact(&mut len)?;
            let len = u32::from_be_bytes(len) as usize;
            if len == 0 {
                continue;
            }
            if len > 1 << 20 {
                return Ok(());
            }
            msg.resize(len, 0);
            reader.read_exact(&mut msg)?;
            match msg[0] {
                1 => { torrent.stats.unchokes.fetch_add(1, Relaxed); }
                7 if len >= 9 => { torrent.stats.uploaded_bytes.fetch_add((len - 9) as u64, Relaxed); }
                6 if len == 13 => {
                    let field = |i: usize| u32::from_be_bytes(msg[i..i + 4].try_into().unwrap());
                    let (index, begin, length) = (field(1), field(5), field(9));
                    if index >= torrent.pieces
                        || length == 0
                        || length > MAX_BLOCK
                        || u64::from(begin) + u64::from(length) > torrent.piece_size(index)
                    {
                        return Ok(());
                    }
                    if begin == 0 {
                        let mut order = torrent.stats.first_requests.lock().unwrap();
                        if order.len() < 128 && !order.contains(&index) { order.push(index); }
                    }
                    if self.probe && !probed && sent >= 1024*1024
                        && torrent.stats.probes.compare_exchange(0, 1, Relaxed, Relaxed).is_ok() {
                        // After progress, request piece zero even though choked.
                        // Upload-disabled clients need not send Have messages.
                        let mut request = vec![0u8; 8];
                        request.extend_from_slice(&16384u32.to_be_bytes());
                        message(&mut out, 6, &request);
                        probed = true;
                    }
                    let absolute = u64::from(index) * torrent.piece_len + u64::from(begin);
                    out.extend_from_slice(&(length + 9).to_be_bytes());
                    out.push(7);
                    out.extend_from_slice(&index.to_be_bytes());
                    out.extend_from_slice(&begin.to_be_bytes());
                    let start = out.len();
                    out.resize(start + length as usize, 0);
                    payload::fill_with_prefix(torrent.key, absolute, &mut out[start..], &torrent.prefix);
                    let stats = &torrent.stats;
                    let now = now_ns(self.epoch);
                    let _ = stats.first_ns.compare_exchange(0, now, Relaxed, Relaxed);
                    stats.last_ns.fetch_max(now, Relaxed);
                    stats.bytes.fetch_add(u64::from(length), Relaxed);
                    stats.blocks.fetch_add(1, Relaxed);
                    // At low rates, a 1 MiB batch would withhold all bytes for
                    // 32 seconds and trigger the client's legitimate timeout.
                    let flush_at = if self.rate > 0 { MAX_BLOCK as usize } else { FLUSH_AT };
                    if out.len() >= flush_at {
                        self.flush(&mut writer, &mut out, started, &mut sent)?;
                    }
                }
                20 if len >= 2 => {
                    let body = &msg[2..];
                    let Ok(parsed) = bencode::parse(body) else { return Ok(()) };
                    if msg[1] == 0 {
                        peer_ut_metadata = parsed
                            .root
                            .get(body, b"m")
                            .and_then(|(m, _)| m.get(body, b"ut_metadata"))
                            .and_then(|(v, _)| v.int())
                            .and_then(|v| u8::try_from(v).ok())
                            .filter(|v| *v != 0);
                    } else if msg[1] == OUR_UT_METADATA {
                        let get = |k: &[u8]| parsed.root.get(body, k).and_then(|(v, _)| v.int());
                        let (Some(0), Some(piece), Some(ext)) = (get(b"msg_type"), get(b"piece"), peer_ut_metadata)
                        else {
                            continue;
                        };
                        let start = (piece as usize) * 16384;
                        if piece < 0 || start >= torrent.info.len() {
                            return Ok(());
                        }
                        let end = (start + 16384).min(torrent.info.len());
                        let mut reply = b"d".to_vec();
                        put_bytes(&mut reply, b"msg_type");
                        put_int(&mut reply, 1);
                        put_bytes(&mut reply, b"piece");
                        put_int(&mut reply, piece);
                        put_bytes(&mut reply, b"total_size");
                        put_int(&mut reply, torrent.info.len() as i64);
                        reply.push(b'e');
                        reply.extend_from_slice(&torrent.info[start..end]);
                        extended(&mut out, ext, &reply);
                        torrent.stats.metadata_pieces.fetch_add(1, Relaxed);
                    }
                }
                _ => {}
            }
        }
    }

    fn flush(&self, writer: &mut TcpStream, out: &mut Vec<u8>, started: Instant, sent: &mut u64) -> std::io::Result<()> {
        *sent += out.len() as u64;
        if self.rate > 0 {
            let due = Duration::from_secs_f64(*sent as f64 / self.rate as f64);
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        writer.write_all(out)?;
        out.clear();
        Ok(())
    }
}
