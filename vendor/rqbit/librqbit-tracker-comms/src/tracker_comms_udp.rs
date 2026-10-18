use rqbit_runtime::Cx;
use std::{
    collections::{HashMap, hash_map::Entry},
    ffi::CStr,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use librqbit_core::{hash_id::Id20, spawn_utils::spawn_with_cancel};
use librqbit_dualstack_sockets::{BindDevice, UdpSocket};
use parking_lot::RwLock;
use rand::RngExt;
use rqbit_runtime::CancellationToken;
use tracing::{debug, debug_span, trace, warn};

const ACTION_CONNECT: u32 = 0;
const ACTION_ANNOUNCE: u32 = 1;
// const ACTION_SCRAPE: u32 = 2;
const ACTION_ERROR: u32 = 3;

pub const EVENT_NONE: u32 = 0;
pub const EVENT_COMPLETED: u32 = 1;
pub const EVENT_STARTED: u32 = 2;
pub const EVENT_STOPPED: u32 = 3;

pub type ConnectionId = u64;
const CONNECTION_ID_MAGIC: ConnectionId = 0x41727101980;

pub type TransactionId = u32;

pub fn new_transaction_id() -> TransactionId {
    rand::rng().random()
}

#[derive(Debug)]
pub struct AnnounceFields {
    pub info_hash: Id20,
    pub peer_id: Id20,
    pub downloaded: u64,
    pub left: u64,
    pub uploaded: u64,
    pub event: u32,
    pub key: u32,
    pub port: u16,
}

#[derive(Debug)]
pub enum Request {
    Connect,
    Announce(ConnectionId, AnnounceFields),
}

impl Request {
    pub fn serialize(
        &self,
        transaction_id: TransactionId,
        buf: &mut [u8],
    ) -> anyhow::Result<usize> {
        struct W<'a> {
            buf: &'a mut [u8],
            offset: usize,
        }
        impl W<'_> {
            fn extend_from_slice(&mut self, s: &[u8]) -> anyhow::Result<()> {
                if self.buf.len() < self.offset + s.len() {
                    bail!("not enough space in buffer")
                }
                self.buf[self.offset..self.offset + s.len()].copy_from_slice(s);
                self.offset += s.len();
                Ok(())
            }
        }

        let mut w = W { buf, offset: 0 };

        match self {
            Request::Connect => {
                w.extend_from_slice(&CONNECTION_ID_MAGIC.to_be_bytes())?;
                w.extend_from_slice(&ACTION_CONNECT.to_be_bytes())?;
                w.extend_from_slice(&transaction_id.to_be_bytes())?;
            }
            Request::Announce(connection_id, fields) => {
                w.extend_from_slice(&connection_id.to_be_bytes())?;
                w.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes())?;
                w.extend_from_slice(&transaction_id.to_be_bytes())?;
                w.extend_from_slice(&fields.info_hash.0)?;
                w.extend_from_slice(&fields.peer_id.0)?;
                w.extend_from_slice(&fields.downloaded.to_be_bytes())?;
                w.extend_from_slice(&fields.left.to_be_bytes())?;
                w.extend_from_slice(&fields.uploaded.to_be_bytes())?;
                w.extend_from_slice(&fields.event.to_be_bytes())?;
                w.extend_from_slice(&0u32.to_be_bytes())?; // ip address 0
                w.extend_from_slice(&fields.key.to_be_bytes())?;
                w.extend_from_slice(&(-1i32).to_be_bytes())?; // num want -1
                w.extend_from_slice(&fields.port.to_be_bytes())?;
            }
        }
        Ok(w.offset)
    }
}

#[derive(Debug)]
pub struct AnnounceResponse {
    pub interval: u32,
    #[allow(dead_code)]
    pub leechers: u32,
    #[allow(dead_code)]
    pub seeders: u32,
    pub addrs: Vec<SocketAddr>,
}

#[derive(Debug)]
pub enum Response {
    Connect(ConnectionId),
    Announce(AnnounceResponse),
    #[allow(dead_code)]
    Error(String),
    Unknown,
}

fn split_slice(s: &[u8], first_len: usize) -> Option<(&[u8], &[u8])> {
    if s.len() < first_len {
        return None;
    }
    Some(s.split_at(first_len))
}

fn s_to_arr<const T: usize>(buf: &[u8]) -> [u8; T] {
    let mut arr = [0u8; T];
    arr.copy_from_slice(buf);
    arr
}

trait ParseNum: Sized {
    fn parse_num(buf: &[u8]) -> anyhow::Result<(Self, &[u8])>;
}

macro_rules! parse_impl {
    ($ty:tt, $size:expr) => {
        impl ParseNum for $ty {
            fn parse_num(buf: &[u8]) -> anyhow::Result<($ty, &[u8])> {
                let (bytes, rest) =
                    split_slice(buf, $size).with_context(|| format!("expected {} bytes", $size))?;
                let num = $ty::from_be_bytes(s_to_arr(bytes));
                Ok((num, rest))
            }
        }
    };
}

parse_impl!(u32, 4);
parse_impl!(u64, 8);
parse_impl!(u128, 16);
parse_impl!(u16, 2);
parse_impl!(i32, 4);
parse_impl!(i64, 8);
parse_impl!(i16, 2);

impl Response {
    pub fn parse(buf: &[u8], is_ipv6: bool) -> anyhow::Result<(TransactionId, Self)> {
        let (action, buf) = u32::parse_num(buf).context("can't parse action")?;
        let (tid, buf) = u32::parse_num(buf).context("can't parse transaction id")?;

        let response = match Self::parse_response(action, is_ipv6, buf) {
            Ok(r) => r,
            Err(e) => {
                debug!("error parsing: {e:#}");
                Response::Unknown
            }
        };

        Ok((tid, response))
    }

    fn parse_response(action: u32, is_ipv6: bool, mut buf: &[u8]) -> anyhow::Result<Self> {
        let response = match action {
            ACTION_CONNECT => {
                let (connection_id, b) =
                    u64::parse_num(buf).context("can't parse connection id")?;
                buf = b;
                Response::Connect(connection_id)
            }
            ACTION_ANNOUNCE => {
                let (interval, b) = u32::parse_num(buf).context("can't parse interval")?;
                let (leechers, b) = u32::parse_num(b).context("can't parse leechers")?;
                let (seeders, mut b) = u32::parse_num(b).context("can't parse seeders")?;
                let mut addrs = Vec::new();
                while !b.is_empty() {
                    let (addr, b2) = if is_ipv6 {
                        let (ip, b2) = u128::parse_num(b)?;
                        (IpAddr::V6(Ipv6Addr::from(ip)), b2)
                    } else {
                        let (ip, b2) = u32::parse_num(b)?;
                        (IpAddr::V4(Ipv4Addr::from(ip)), b2)
                    };
                    b = b2;

                    let (port, b2) = u16::parse_num(b)?;
                    b = b2;
                    addrs.push(SocketAddr::new(addr, port));
                }
                buf = b;
                Response::Announce(AnnounceResponse {
                    interval,
                    leechers,
                    seeders,
                    addrs,
                })
            }
            ACTION_ERROR => {
                let msg = CStr::from_bytes_with_nul(buf)
                    .ok()
                    .and_then(|s| s.to_str().ok())
                    .or_else(|| std::str::from_utf8(buf).ok())
                    .unwrap_or("<invalid UTF-8>")
                    .to_owned();
                return Ok(Response::Error(msg));
            }
            _ => bail!("unsupported action {action}"),
        };

        if !buf.is_empty() {
            bail!(
                "parsed {response:?} so far, but got {} remaining bytes",
                buf.len()
            );
        }

        Ok(response)
    }
}

struct ConnectionIdMeta {
    id: ConnectionId,
    created: Instant,
}

#[derive(Default)]
struct ClientLocked {
    connections: HashMap<SocketAddr, ConnectionIdMeta>,
    transactions: HashMap<TransactionId, rqbit_runtime::sync::oneshot::Sender<Response>>,
}

struct ClientShared {
    sock: UdpSocket,
    locked: RwLock<ClientLocked>,
}

#[derive(Clone)]
pub struct UdpTrackerClient {
    state: Arc<ClientShared>,
}

struct TransactionIdGuard<'a> {
    tid: TransactionId,
    state: &'a ClientShared,
}

impl Drop for TransactionIdGuard<'_> {
    fn drop(&mut self) {
        let mut g = self.state.locked.write();
        g.transactions.remove(&self.tid);
    }
}

impl UdpTrackerClient {
    pub async fn new(
        cx: &Cx,
        cancel_token: CancellationToken,
        bind_device: Option<&BindDevice>,
    ) -> anyhow::Result<Self> {
        cx.checkpoint()?;
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
        let sock = UdpSocket::bind_udp(
            addr,
            librqbit_dualstack_sockets::BindOpts {
                device: bind_device,
                ..Default::default()
            },
        )
        .with_context(|| format!("error creating UDP socket at {addr}"))?;

        let client = Self {
            state: Arc::new(ClientShared {
                sock,
                locked: RwLock::new(Default::default()),
            }),
        };

        spawn_with_cancel(
            cx,
            debug_span!("udp_tracker"),
            "udp_tracker",
            cancel_token,
            {
                let client = client.clone();
                move |child_cx| async move { client.run(&child_cx).await }
            },
        );

        Ok(client)
    }

    async fn run(self, cx: &Cx) -> anyhow::Result<()> {
        let mut buf = [0u8; 16384];
        loop {
            cx.checkpoint()?;
            let (len, addr) = match self.state.sock.recv_from(cx, &mut buf).await {
                Ok(r) => r,
                Err(e) => {
                    warn!("error in UdpSocket::recv_from: {e:#}");
                    rqbit_runtime::time::sleep(cx, std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            let (tid, response) = match Response::parse(&buf[..len], addr.is_ipv6()) {
                Ok(r) => r,
                Err(e) => {
                    debug!(?addr, "error parsing UDP response: {e:#}");
                    continue;
                }
            };

            trace!(?tid, ?response, ?addr, "received");

            let t = self.state.locked.write().transactions.remove(&tid);
            match t {
                Some(tx) => match tx.send(cx, response) {
                    Ok(_) => {}
                    Err(_) => {
                        debug!(tid, "reader dead");
                    }
                },
                None => {
                    debug!(tid, "nowhere to send response");
                }
            };
        }
    }

    async fn get_connection_id(&self, cx: &Cx, addr: SocketAddr) -> anyhow::Result<ConnectionId> {
        cx.checkpoint()?;
        if let Some(m) = self.state.locked.read().connections.get(&addr)
            && m.created.elapsed() < Duration::from_secs(60)
        {
            return Ok(m.id);
        }

        let response = self.request(cx, addr, Request::Connect).await?;
        match response {
            Response::Connect(connection_id) => {
                self.state.locked.write().connections.insert(
                    addr,
                    ConnectionIdMeta {
                        id: connection_id,
                        created: Instant::now(),
                    },
                );
                Ok(connection_id)
            }
            _ => anyhow::bail!("expected connect response"),
        }
    }

    async fn request(
        &self,
        cx: &Cx,
        addr: SocketAddr,
        request: Request,
    ) -> anyhow::Result<Response> {
        cx.checkpoint()?;
        let (tx, mut rx) = rqbit_runtime::sync::oneshot::channel();
        let tid_g = self.reserve_transaction_id(tx)?;

        let mut write_buf = [0u8; 1024];
        let len = request.serialize(tid_g.tid, &mut write_buf)?;
        self.state
            .sock
            .send_to(cx, &write_buf[..len], addr)
            .await
            .with_context(|| format!("error sending to {addr:?}"))?;

        let response = rqbit_runtime::time::timeout(cx, Duration::from_secs(10), rx.recv(cx))
            .await
            .context("timeout connecting")?
            .context("sender dead")?;
        match &response {
            Response::Error(e) => {
                anyhow::bail!("remote errored: {e}")
            }
            Response::Unknown => {
                anyhow::bail!("remote replied with something we could not parse")
            }
            _ => {}
        }
        Ok(response)
    }

    fn reserve_transaction_id(
        &self,
        tx: rqbit_runtime::sync::oneshot::Sender<Response>,
    ) -> anyhow::Result<TransactionIdGuard<'_>> {
        let mut g = self.state.locked.write();
        for _ in 0..10 {
            let t = new_transaction_id();
            match g.transactions.entry(t) {
                Entry::Occupied(_) => continue,
                Entry::Vacant(vac) => {
                    vac.insert(tx);
                    return Ok(TransactionIdGuard {
                        tid: t,
                        state: &self.state,
                    });
                }
            }
        }
        bail!("cant generate transaction id")
    }

    pub async fn announce(
        &self,
        cx: &Cx,
        tracker: SocketAddr,
        fields: AnnounceFields,
    ) -> anyhow::Result<AnnounceResponse> {
        cx.checkpoint()?;
        let connection_id = self.get_connection_id(cx, tracker).await?;
        let request = Request::Announce(connection_id, fields);
        let response = self.request(cx, tracker, request).await?;
        match response {
            Response::Announce(r) => Ok(r),
            other => bail!("unexpected response {other:?}, expected announce"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rqbit_runtime::{Budget, CancellationToken, RuntimeBuilder};
    use std::{
        net::{Ipv4Addr, SocketAddr},
        time::Duration,
    };

    #[test]
    fn native_udp_tracker_connect_and_announce() {
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
            let region = cx
                .open_child_region(rqbit_runtime::cx::ChildRegionSpec::inherit())
                .await
                .unwrap();
            let cx = region.cx().clone();
            let socket =
                UdpSocket::bind_udp((Ipv4Addr::LOCALHOST, 0).into(), Default::default()).unwrap();
            let tracker_addr = socket.bind_addr();
            let peer_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 4444));
            let server = async {
                let mut packet = [0; 512];
                let (length, remote) = socket.recv_from(&cx, &mut packet).await.unwrap();
                assert_eq!(length, 16);
                assert_eq!(&packet[..8], &CONNECTION_ID_MAGIC.to_be_bytes());
                assert_eq!(&packet[8..12], &0u32.to_be_bytes());
                let mut response = Vec::new();
                response.extend_from_slice(&0u32.to_be_bytes());
                response.extend_from_slice(&packet[12..16]);
                response.extend_from_slice(&123u64.to_be_bytes());
                socket.send_to(&cx, &response, remote).await.unwrap();
                let (length, remote) = socket.recv_from(&cx, &mut packet).await.unwrap();
                assert_eq!(length, 98);
                assert_eq!(&packet[..8], &123u64.to_be_bytes());
                assert_eq!(&packet[8..12], &1u32.to_be_bytes());
                assert_eq!(&packet[16..36], &[7; 20]);
                assert_eq!(&packet[96..98], &7777u16.to_be_bytes());
                let mut response = Vec::new();
                response.extend_from_slice(&1u32.to_be_bytes());
                response.extend_from_slice(&packet[12..16]);
                response.extend_from_slice(&30u32.to_be_bytes());
                response.extend_from_slice(&0u32.to_be_bytes());
                response.extend_from_slice(&1u32.to_be_bytes());
                response.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
                response.extend_from_slice(&peer_addr.port().to_be_bytes());
                socket.send_to(&cx, &response, remote).await.unwrap();
            };
            let cancel = CancellationToken::new();
            let tracker_client = UdpTrackerClient::new(&cx, cancel.clone(), None)
                .await
                .unwrap();
            let client_bind_addr = tracker_client.state.sock.bind_addr();
            let client = async {
                let client = &tracker_client;
                let result = client
                    .announce(
                        &cx,
                        tracker_addr,
                        AnnounceFields {
                            info_hash: Id20::new([7; 20]),
                            peer_id: Id20::new([8; 20]),
                            downloaded: 12,
                            left: 50,
                            uploaded: 6,
                            event: EVENT_STARTED,
                            key: 5,
                            port: 7777,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(result.interval, 30);
                assert_eq!(result.addrs, vec![peer_addr]);
                assert!(client.state.locked.read().transactions.is_empty());
                cancel.cancel();
            };
            rqbit_runtime::time::timeout(&cx, Duration::from_secs(3), async {
                futures::join!(server, client);
            })
            .await
            .unwrap();
            region.close().await.unwrap();
            assert_eq!(
                Arc::strong_count(&tracker_client.state),
                1,
                "UDP response task releases its owner"
            );
            drop(tracker_client);
            let rebound = std::net::UdpSocket::bind(client_bind_addr)
                .expect("drained tracker client releases its socket");
            drop(rebound);
        });
        runtime.block_on(task).unwrap();
    }
}
