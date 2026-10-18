use std::{
    cmp::Reverse,
    net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    task::Poll,
    time::{Duration, Instant},
};

use crate::{
    Error, INACTIVITY_TIMEOUT, REQUERY_INTERVAL, RESPONSE_TIMEOUT,
    bprotocol::{
        self, AnnouncePeer, CompactNodeInfo, CompactNodeInfoOwned, ErrorDescription,
        FindNodeRequest, GetPeersRequest, Message, MessageKind, Node, PingRequest, Response, Want,
    },
    peer_store::PeerStore,
    routing_table::{InsertResult, NodeStatus, RoutingTable},
};
use bencode::ByteBufOwned;
use dashmap::DashMap;
use futures::{
    FutureExt, Stream, StreamExt, TryFutureExt, future::BoxFuture, stream::FuturesUnordered,
};

use librqbit_core::{
    compact_ip::{CompactSerialize, CompactSerializeFixedLen},
    crate_version,
    hash_id::Id20,
    peer_id::generate_azereus_style,
    spawn_utils::{spawn, spawn_with_cancel},
};
use librqbit_dualstack_sockets::{BindDevice, UdpSocket};
use parking_lot::RwLock;

use rqbit_runtime::sync::mpsc::{
    Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
};
use serde::Serialize;

use rqbit_runtime::{CancellationToken, Cx};
use tracing::{Instrument, debug, debug_span, error, info, trace, warn};

fn now() -> Instant {
    Instant::now()
}

#[derive(Debug, Serialize)]
pub struct DhtStats {
    #[serde(serialize_with = "crate::utils::serialize_id20")]
    pub id: Id20,
    pub outstanding_requests: usize,
    pub routing_table_size: usize,
    pub routing_table_size_v6: usize,
}

struct OutstandingRequest {
    done: rqbit_runtime::sync::oneshot::Sender<crate::Result<ResponseOrError>>,
}

pub struct WorkerSendRequest {
    // If this is set, we are tracking the response in inflight_by_transaction_id
    our_tid: Option<u16>,
    message: Message<ByteBufOwned>,
    addr: SocketAddr,
}

#[derive(Debug)]
struct MaybeUsefulNode {
    id: Id20,
    addr: SocketAddr,
    last_request: Instant,
    last_response: Option<Instant>,
    errors_in_a_row: usize,
    returned_peers: bool,
}

struct RateLimiter {
    next: std::sync::Mutex<Instant>,
    period: Duration,
}
impl RateLimiter {
    async fn acquire_one(&self, cx: &Cx) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let wait = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let at = (*next).max(Instant::now());
            *next = at + self.period;
            at.saturating_duration_since(Instant::now())
        };
        if !wait.is_zero() {
            rqbit_runtime::time::sleep(cx, wait).await;
        }
        cx.checkpoint().map_err(|_| Error::Cancelled)
    }
}
fn make_rate_limiter() -> RateLimiter {
    let qps = std::env::var("DHT_QUERIES_PER_SECOND")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(250)
        .max(1);
    RateLimiter {
        next: std::sync::Mutex::new(Instant::now()),
        period: Duration::from_secs(1) / qps,
    }
}

trait RecursiveRequestCallbacks: Sized + Send + Sync + 'static {
    fn on_request_start(&self, req: &RecursiveRequest<Self>, target_node: Id20, addr: SocketAddr);
    fn on_request_end(
        &self,
        req: &RecursiveRequest<Self>,
        target_node: Id20,
        addr: SocketAddr,
        resp: &crate::Result<ResponseOrError>,
    );
}

struct RecursiveRequestCallbacksGetPeers {
    // Id20::from_str("00000fffffffffffffffffffffffffffffffffff").unwrap()
    min_distance_to_announce: Id20,
    announce_port: Option<u16>,
}

impl RecursiveRequestCallbacks for RecursiveRequestCallbacksGetPeers {
    fn on_request_start(&self, _: &RecursiveRequest<Self>, _: Id20, _: SocketAddr) {}

    fn on_request_end(
        &self,
        req: &RecursiveRequest<Self>,
        target_node: Id20,
        addr: SocketAddr,
        resp: &crate::Result<ResponseOrError>,
    ) {
        let announce_port = match self.announce_port {
            Some(a) => a,
            None => return,
        };
        let resp = match resp {
            Ok(ResponseOrError::Response(resp)) => resp,
            _ => return,
        };
        let token = match &resp.token {
            Some(token) => token,
            None => return,
        };
        if req.info_hash.distance(&target_node) > self.min_distance_to_announce {
            trace!(
                "not announcing, {:?} is too far from {:?}",
                target_node, req.info_hash
            );
            return;
        }
        let (tid, message) = req.dht.create_request(
            Request::Announce {
                info_hash: req.info_hash,
                token: token.clone(),
                port: announce_port,
            },
            addr,
        );

        let _ = req.dht.worker_sender.send(WorkerSendRequest {
            our_tid: Some(tid),
            message,
            addr,
        });
    }
}

struct RecursiveRequestCallbacksFindNodes {}
impl RecursiveRequestCallbacks for RecursiveRequestCallbacksFindNodes {
    fn on_request_start(&self, req: &RecursiveRequest<Self>, target_node: Id20, addr: SocketAddr) {
        let mut rt = req.dht.get_table_for_addr(addr).write();
        match rt.add_node(target_node, addr) {
            InsertResult::WasExisting | InsertResult::ReplacedBad(_) | InsertResult::Added => {
                rt.mark_outgoing_request(&target_node, now());
            }
            InsertResult::Ignored => {}
        }
    }

    fn on_request_end(
        &self,
        req: &RecursiveRequest<Self>,
        target_node: Id20,
        addr: SocketAddr,
        resp: &crate::Result<ResponseOrError>,
    ) {
        let mut table = req.dht.get_table_for_addr(addr).write();
        if resp.is_ok() {
            table.mark_response(&target_node, now());
        } else {
            table.mark_error(&target_node);
        }
    }
}

struct RecursiveRequest<C: RecursiveRequestCallbacks> {
    max_depth: usize,
    useful_nodes_limit: usize,
    info_hash: Id20,
    request: Request,
    dht: Arc<DhtState>,
    useful_nodes: RwLock<Vec<MaybeUsefulNode>>,
    peer_tx: rqbit_runtime::sync::mpsc::UnboundedSender<SocketAddr>,
    node_tx: rqbit_runtime::sync::mpsc::UnboundedSender<(Option<Id20>, SocketAddr, usize)>,
    callbacks: C,
}

pub struct RequestPeersStream {
    rx: rqbit_runtime::sync::mpsc::UnboundedReceiver<SocketAddr>,
    cx: Cx,
    cancel_join_handle_v4: rqbit_runtime::task::JoinHandle<()>,
    cancel_join_handle_v6: rqbit_runtime::task::JoinHandle<()>,
}

impl RequestPeersStream {
    fn new(cx: &Cx, dht: Arc<DhtState>, info_hash: Id20, announce_port: Option<u16>) -> Self {
        let (peer_tx, peer_rx) = unbounded_channel();
        let make = |is_v4: bool, dht: Arc<DhtState>, peer_tx: UnboundedSender<SocketAddr>| {
            let (node_tx, node_rx) = unbounded_channel();
            let rp = Arc::new(RecursiveRequest {
                max_depth: 4,
                info_hash,
                useful_nodes_limit: 256,
                request: Request::GetPeers(info_hash),
                dht,
                useful_nodes: RwLock::new(Vec::new()),
                peer_tx,
                node_tx,
                callbacks: RecursiveRequestCallbacksGetPeers {
                    min_distance_to_announce: Id20::from_str(
                        "0000ffffffffffffffffffffffffffffffffffff",
                    )
                    .unwrap(),
                    announce_port,
                },
            });
            rp.request_peers_forever(cx, node_rx, is_v4)
        };

        let v4 = make(true, dht.clone(), peer_tx.clone());
        let v6 = make(false, dht, peer_tx);

        Self {
            rx: peer_rx,
            cx: cx.clone(),
            cancel_join_handle_v4: v4,
            cancel_join_handle_v6: v6,
        }
    }
}

impl Drop for RequestPeersStream {
    fn drop(&mut self) {
        self.cancel_join_handle_v4.abort();
        self.cancel_join_handle_v6.abort();
    }
}

impl Stream for RequestPeersStream {
    type Item = SocketAddr;

    #[inline(never)]
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let owner = self.cx.clone();
        self.rx.poll_recv(&owner, cx).map(|r| r.ok())
    }
}

impl RecursiveRequest<RecursiveRequestCallbacksFindNodes> {
    async fn find_node_for_routing_table(
        cx: &Cx,
        dht: Arc<DhtState>,
        target: Id20,
        addrs: impl Iterator<Item = SocketAddr>,
    ) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let (node_tx, mut node_rx) = unbounded_channel();
        let req = RecursiveRequest {
            max_depth: 4,
            info_hash: target,
            request: Request::FindNode(target),
            dht,
            useful_nodes_limit: 32,
            useful_nodes: RwLock::new(Vec::new()),
            peer_tx: unbounded_channel().0,
            node_tx,
            callbacks: RecursiveRequestCallbacksFindNodes {},
        };

        let request_one = |id, addr, depth| {
            req.request_one(cx, id, addr, depth)
                .map_err(|e| {
                    debug!("error: {e:#}");
                    e
                })
                .instrument(debug_span!(
                    "find_node",
                    target = format!("{target:?}"),
                    addr = addr.to_string()
                ))
        };

        let mut futs = FuturesUnordered::new();

        let mut initial_addrs = 0;
        for addr in addrs {
            futs.push(request_one(None, addr, 0));
            initial_addrs += 1;
        }

        let mut successes = 0;
        let mut errors = 0;

        loop {
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            enum Next<T> {
                Node(
                    Result<(Option<Id20>, SocketAddr, usize), rqbit_runtime::sync::mpsc::RecvError>,
                ),
                Done(Option<T>),
            }
            let next = {
                let node = node_rx.recv(cx).fuse();
                let done = futs.next().fuse();
                futures::pin_mut!(node, done);
                futures::select_biased! { value = node => Next::Node(value), value = done => Next::Done(value) }
            };
            match next {
                Next::Node(Ok((id, addr, depth))) => futs.push(request_one(id, addr, depth)),
                Next::Node(Err(_)) => return Err(Error::DhtDead),
                Next::Done(Some(f)) => {
                    if f.is_ok() {
                        successes += 1
                    } else {
                        errors += 1
                    }
                }
                Next::Done(None) => break,
            }
        }

        if successes == 0 {
            return Err(Error::NoSuccessfulLookups { errors });
        }
        debug!(
            "finished, successes = {successes}, errors = {errors}, initial_addrs = {initial_addrs}"
        );
        Ok(())
    }
}

impl RecursiveRequest<RecursiveRequestCallbacksGetPeers> {
    fn request_peers_forever(
        self: &Arc<Self>,
        cx: &Cx,
        mut node_rx: rqbit_runtime::sync::mpsc::UnboundedReceiver<(
            Option<Id20>,
            SocketAddr,
            usize,
        )>,
        is_v4: bool,
    ) -> rqbit_runtime::task::JoinHandle<()> {
        let this = self.clone();
        spawn(
            cx,
            debug_span!(parent: None, "get_peers", is_v4, info_hash = format!("{:?}", self.info_hash)),
            "get_peers",
            move |child_cx| async move {
                let cx = &child_cx;
                let this = &this;
                // Looper adds root nodes to the queue every 60 seconds.
                let looper = {
                    async move {
                        let mut iteration = 0;
                        loop {
                            cx.checkpoint().map_err(|_| Error::Cancelled)?;
                            trace!("iteration {}", iteration);
                            let sleep = match this.get_peers_root(is_v4) {
                                Ok(0) => Duration::from_secs(1),
                                Ok(n) if n < 8 => REQUERY_INTERVAL / 8 * (n as u32),
                                Ok(_) => REQUERY_INTERVAL,
                                Err(e) => {
                                    error!("dht: error in get_peers_root(): {e:#}");
                                    return Err::<(), crate::Error>(e);
                                }
                            };
                            rqbit_runtime::time::sleep(cx, sleep).await;
                            iteration += 1;
                        }
                    }
                };
                futures::pin_mut!(looper);

                let mut futs = FuturesUnordered::new();
                loop {
                    cx.checkpoint().map_err(|_| Error::Cancelled)?;
                    enum Next<T> {
                        Node(
                            Result<
                                (Option<Id20>, SocketAddr, usize),
                                rqbit_runtime::sync::mpsc::RecvError,
                            >,
                        ),
                        Done(Option<T>),
                        Loop(crate::Result<()>),
                    }
                    let next = {
                        let node = node_rx.recv(cx).fuse();
                        let done = async {
                            if futs.is_empty() {
                                futures::future::pending().await
                            } else {
                                futs.next().await
                            }
                        }
                        .fuse();
                        let looper_wait = looper.as_mut().fuse();
                        futures::pin_mut!(node, done, looper_wait);
                        futures::select! { value = node => Next::Node(value), value = done => Next::Done(value), value = looper_wait => Next::Loop(value) }
                    };
                    match next {
                        Next::Node(Ok((id, addr, depth))) => futs.push(
                            this.request_one(cx, id, addr, depth)
                                .map_err(|e| debug!("error: {e:#}"))
                                .instrument(debug_span!("addr", addr = addr.to_string())),
                        ),
                        Next::Node(Err(_)) => return Err(Error::DhtDead),
                        Next::Loop(result) => return result,
                        _ => {}
                    }
                }
            },
        )
    }

    fn get_peers_root(&self, is_v4: bool) -> crate::Result<usize> {
        let mut count = 0;
        let table = if is_v4 {
            &self.dht.routing_table_v4
        } else {
            &self.dht.routing_table_v6
        };
        for (id, addr) in table
            .read()
            .sorted_by_distance_from(self.info_hash, now())
            .iter()
            .map(|n| (n.id(), n.addr()))
            .take(8)
        {
            count += 1;
            self.node_tx
                .send((Some(id), addr, 0))
                .ok()
                .ok_or(Error::DhtDead)?;
        }

        Ok(count)
    }
}

impl<C: RecursiveRequestCallbacks> RecursiveRequest<C> {
    async fn request_one(
        &self,
        cx: &Cx,
        id: Option<Id20>,
        addr: SocketAddr,
        depth: usize,
    ) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        if let Some(id) = id {
            self.callbacks.on_request_start(self, id, addr);
        }

        let response = self.dht.request(cx, self.request.clone(), addr).await;
        if let Ok(ref value) = response {
            self.mark_node_responded(addr, value);
        }
        if let Some(id) = id {
            self.callbacks.on_request_end(self, id, addr, &response);
        }
        let response = match response {
            Ok(ResponseOrError::Response(r)) => r,
            Ok(ResponseOrError::Error(e)) => {
                debug!("error response: {e:?}");
                return Err(Error::ErrorResponse);
            }
            Err(e) => {
                self.mark_node_error(addr);
                return Err(e);
            }
        };
        if let Some(peers) = response.values {
            for peer in peers {
                self.peer_tx.send(peer.0).ok().ok_or(Error::ReceiverDead)?;
            }
        }

        let node_it = response
            .nodes
            .iter()
            .flat_map(|n| n.iter().map(|n| n.as_socketaddr()))
            .chain(
                response
                    .nodes6
                    .iter()
                    .flat_map(|n| n.iter().map(|n| n.as_socketaddr())),
            )
            .filter(|node| addr.is_ipv4() == node.addr.is_ipv4());

        let now = now();

        for node in node_it {
            let should_request = self.should_request_node(node.id, node.addr, depth, now);
            trace!(
                "should_request={}, id={:?}, addr={}, depth={}/{}",
                should_request, node.id, node.addr, depth, self.max_depth
            );
            if should_request {
                self.node_tx
                    .send((Some(node.id), node.addr, depth + 1))
                    .ok()
                    .ok_or(Error::ReceiverDead)?;
            }
        }
        Ok(())
    }

    fn mark_node_error(&self, addr: SocketAddr) -> bool {
        self.useful_nodes
            .write()
            .iter_mut()
            .find(|n| n.addr == addr)
            .map(|n| {
                n.errors_in_a_row += 1;
            })
            .is_some()
    }

    fn mark_node_responded(&self, addr: SocketAddr, response: &ResponseOrError) -> bool {
        self.useful_nodes
            .write()
            .iter_mut()
            .find(|n| n.addr == addr)
            .map(|node| {
                node.last_response = Some(now());
                node.errors_in_a_row = 0;
                match response {
                    ResponseOrError::Response(r) => {
                        node.returned_peers =
                            r.values.as_ref().map(|c| !c.is_empty()).unwrap_or(false)
                    }
                    ResponseOrError::Error(_) => {
                        node.returned_peers = false;
                    }
                }
            })
            .is_some()
    }

    fn should_request_node(
        &self,
        node_id: Id20,
        addr: SocketAddr,
        depth: usize,
        now: Instant,
    ) -> bool {
        if depth >= self.max_depth {
            return false;
        }

        let mut closest_nodes = self.useful_nodes.write();

        // If recently requested, ignore
        if let Some(existing) = closest_nodes.iter_mut().find(|n| n.id == node_id) {
            if now - existing.last_request > Duration::from_secs(60) {
                existing.last_request = now;
                return true;
            }
            return false;
        }

        closest_nodes.push(MaybeUsefulNode {
            id: node_id,
            addr,
            last_request: now,
            last_response: None,
            returned_peers: false,
            errors_in_a_row: 0,
        });

        closest_nodes.sort_by_key(|n| {
            let has_returned_peers_desc = Reverse(n.returned_peers);
            let has_responded_desc = Reverse(n.last_response.is_some() as u8);
            let distance = n.id.distance(&self.info_hash);
            let freshest_response = n.last_response.map(|r| now - r).unwrap_or(Duration::MAX);
            (
                has_returned_peers_desc,
                has_responded_desc,
                distance,
                freshest_response,
            )
        });
        if closest_nodes.len() > self.useful_nodes_limit {
            let popped = closest_nodes.pop().unwrap();
            if popped.id == node_id {
                return false;
            }
        }
        true
    }
}

pub struct DhtState {
    id: Id20,
    next_transaction_id: AtomicU16,

    // Created requests: (transaction_id, addr) => Requests.
    // If we get a response, it gets removed from here.
    inflight_by_transaction_id: DashMap<(u16, SocketAddr), OutstandingRequest>,

    routing_table_v4: RwLock<RoutingTable>,
    routing_table_v6: RwLock<RoutingTable>,
    listen_addr: SocketAddr,

    // Sending requests to the worker.
    rate_limiter: RateLimiter,
    // This is to send raw messages
    worker_sender: UnboundedSender<WorkerSendRequest>,

    cancellation_token: CancellationToken,

    pub(crate) peer_store: PeerStore,
}

impl DhtState {
    fn new_internal(
        id: Id20,
        sender: UnboundedSender<WorkerSendRequest>,
        routing_table_v4: Option<RoutingTable>,
        routing_table_v6: Option<RoutingTable>,
        listen_addr: SocketAddr,
        peer_store: PeerStore,
        cancellation_token: CancellationToken,
    ) -> Self {
        let routing_table_v4 = routing_table_v4.unwrap_or_else(|| RoutingTable::new(id, None));
        let routing_table_v6 = routing_table_v6.unwrap_or_else(|| RoutingTable::new(id, None));
        Self {
            id,
            next_transaction_id: AtomicU16::new(0),
            inflight_by_transaction_id: Default::default(),
            routing_table_v4: RwLock::new(routing_table_v4),
            routing_table_v6: RwLock::new(routing_table_v6),
            worker_sender: sender,
            listen_addr,
            rate_limiter: make_rate_limiter(),
            peer_store,
            cancellation_token,
        }
    }

    async fn request(
        &self,
        cx: &Cx,
        request: Request,
        addr: SocketAddr,
    ) -> crate::Result<ResponseOrError> {
        self.rate_limiter.acquire_one(cx).await?;
        let (tid, message) = self.create_request(request, addr);
        let key = (tid, addr);
        let (tx, mut rx) = rqbit_runtime::sync::oneshot::channel();
        self.inflight_by_transaction_id
            .insert(key, OutstandingRequest { done: tx });
        struct RequestGuard<'a> {
            state: &'a DhtState,
            key: (u16, SocketAddr),
        }
        impl Drop for RequestGuard<'_> {
            fn drop(&mut self) {
                self.state.inflight_by_transaction_id.remove(&self.key);
            }
        }
        let _request_guard = RequestGuard { state: self, key };
        trace!("sending {message:?}");
        match self.worker_sender.send(WorkerSendRequest {
            our_tid: Some(tid),
            message,
            addr,
        }) {
            Ok(_) => {}
            Err(_) => {
                self.inflight_by_transaction_id.remove(&key);
                return Err(Error::DhtDead);
            }
        };
        match rqbit_runtime::time::timeout(cx, RESPONSE_TIMEOUT, rx.recv(cx)).await {
            Ok(Ok(r)) => r.map(|r| {
                trace!("received {r:?}");
                r
            }),
            Ok(Err(_)) => {
                self.inflight_by_transaction_id.remove(&key);
                Err(Error::DhtDead)
            }
            Err(_) => {
                self.inflight_by_transaction_id.remove(&key);
                Err(Error::ResponseTimeout(RESPONSE_TIMEOUT))
            }
        }
    }

    fn create_request(&self, request: Request, addr: SocketAddr) -> (u16, Message<ByteBufOwned>) {
        let transaction_id = self.next_transaction_id.fetch_add(1, Ordering::Relaxed);
        let transaction_id_buf = [(transaction_id >> 8) as u8, (transaction_id & 0xff) as u8];

        let want = if addr.is_ipv6() {
            Some(Want::V6)
        } else {
            Some(Want::V4)
        };

        let message = match request {
            Request::GetPeers(info_hash) => Message {
                transaction_id: ByteBufOwned::from(transaction_id_buf.as_ref()),
                version: None,
                ip: None,
                kind: MessageKind::GetPeersRequest(GetPeersRequest {
                    id: self.id,
                    info_hash,
                    want,
                }),
            },
            Request::FindNode(target) => Message {
                transaction_id: ByteBufOwned::from(transaction_id_buf.as_ref()),
                version: None,
                ip: None,
                kind: MessageKind::FindNodeRequest(FindNodeRequest {
                    id: self.id,
                    target,
                    want,
                }),
            },
            Request::Ping => Message {
                transaction_id: ByteBufOwned::from(transaction_id_buf.as_ref()),
                version: None,
                ip: None,
                kind: MessageKind::PingRequest(PingRequest { id: self.id }),
            },
            Request::Announce {
                info_hash,
                token,
                port,
            } => Message {
                kind: MessageKind::AnnouncePeer(AnnouncePeer {
                    id: self.id,
                    implied_port: 0,
                    info_hash,
                    port,
                    token,
                }),
                transaction_id: ByteBufOwned::from(transaction_id_buf.as_ref()),
                version: None,
                ip: None,
            },
        };
        (transaction_id, message)
    }

    fn generate_compact_nodes_both(
        &self,
        target: Id20,
        want: Want,
    ) -> (
        Option<CompactNodeInfoOwned<SocketAddrV4>>,
        Option<CompactNodeInfoOwned<SocketAddrV6>>,
    ) {
        let now = now();
        match want {
            Want::V4 => (
                Some(self.generate_compact_nodes(target, &self.routing_table_v4.read(), now)),
                None,
            ),
            Want::V6 => (
                None,
                Some(self.generate_compact_nodes(target, &self.routing_table_v6.read(), now)),
            ),
            Want::Both => (
                Some(self.generate_compact_nodes(target, &self.routing_table_v4.read(), now)),
                Some(self.generate_compact_nodes(target, &self.routing_table_v6.read(), now)),
            ),
            Want::None => (None, None),
        }
    }

    fn get_table_for_addr(&self, addr: SocketAddr) -> &RwLock<RoutingTable> {
        if addr.is_ipv4() {
            &self.routing_table_v4
        } else {
            &self.routing_table_v6
        }
    }

    fn generate_compact_nodes<A>(
        &self,
        target: Id20,
        table: &RoutingTable,
        now: Instant,
    ) -> CompactNodeInfo<ByteBufOwned, A>
    where
        A: CompactSerialize + CompactSerializeFixedLen + FromSocketAddr,
        Node<A>: CompactSerialize + CompactSerializeFixedLen,
    {
        let it = table
            .sorted_by_distance_from(target, now)
            .into_iter()
            .filter_map(|r| {
                Some(Node {
                    id: r.id(),
                    addr: A::from_socket_addr(r.addr())?,
                })
            })
            .take(8);
        CompactNodeInfo::new_from_iter(it)
    }

    fn on_received_message(
        self: &Arc<Self>,
        cx: &Cx,
        msg: Message<ByteBufOwned>,
        addr: SocketAddr,
    ) -> crate::Result<()> {
        match &msg.kind {
            // If it's a response to a request we made, find the request task, notify it with the response,
            // and let it handle it.
            MessageKind::Error(_) | MessageKind::Response(_) => {
                let tid = msg
                    .get_our_transaction_id()
                    .ok_or(Error::BadTransactionId)?;
                let request = match self
                    .inflight_by_transaction_id
                    .remove(&(tid, addr))
                    .map(|(_, v)| v)
                {
                    Some(req) => req,
                    None => {
                        trace!(?msg, "outstanding request not found");
                        return Err(Error::RequestNotFound);
                    }
                };

                let response_or_error = match msg.kind {
                    MessageKind::Error(e) => ResponseOrError::Error(e),
                    MessageKind::Response(r) => ResponseOrError::Response(r),
                    _ => unreachable!(),
                };
                match request.done.send(cx, Ok(response_or_error)) {
                    Ok(_) => {}
                    Err(e) => {
                        debug!(
                            "received response, but the receiver task is closed: {:?}",
                            e
                        );
                    }
                }
                return Ok(());
            }
            _ => {}
        };

        trace!("received query from {addr}: {msg:?}");

        match &msg.kind {
            // Otherwise, respond to a query.
            MessageKind::PingRequest(req) => {
                let message = Message {
                    transaction_id: msg.transaction_id,
                    version: None,
                    ip: Some(addr),
                    kind: MessageKind::Response(bprotocol::Response {
                        id: self.id,
                        ..Default::default()
                    }),
                };
                self.get_table_for_addr(addr)
                    .write()
                    .mark_last_query(&req.id, now());
                self.worker_sender
                    .send(WorkerSendRequest {
                        our_tid: None,
                        message,
                        addr,
                    })
                    .ok()
                    .ok_or(Error::DhtDead)?;
                Ok(())
            }
            MessageKind::AnnouncePeer(ann) => {
                self.get_table_for_addr(addr)
                    .write()
                    .mark_last_query(&ann.id, now());
                let added = self.peer_store.store_peer(ann, addr);
                trace!("{addr}: added_peer={added}, announce={ann:?}");
                let message = Message {
                    transaction_id: msg.transaction_id,
                    version: None,
                    ip: Some(addr),
                    kind: MessageKind::Response(bprotocol::Response {
                        id: self.id,
                        ..Default::default()
                    }),
                };
                self.worker_sender
                    .send(WorkerSendRequest {
                        our_tid: None,
                        message,
                        addr,
                    })
                    .ok()
                    .ok_or(Error::DhtDead)?;
                Ok(())
            }
            MessageKind::GetPeersRequest(req) => {
                let want = req
                    .want
                    .unwrap_or(if addr.is_ipv6() { Want::V6 } else { Want::V4 });
                let (nodes, nodes6) = self.generate_compact_nodes_both(req.info_hash, want);
                let compact_peer_info = self.peer_store.get_for_info_hash(req.info_hash, want);
                self.get_table_for_addr(addr)
                    .write()
                    .mark_last_query(&req.id, now());
                let message = Message {
                    transaction_id: msg.transaction_id,
                    version: None,
                    ip: Some(addr),
                    kind: MessageKind::Response(bprotocol::Response {
                        id: self.id,
                        nodes,
                        nodes6,
                        values: Some(compact_peer_info),
                        token: Some(ByteBufOwned::from(
                            &self.peer_store.gen_token_for(req.id, addr)[..],
                        )),
                    }),
                };
                self.worker_sender
                    .send(WorkerSendRequest {
                        our_tid: None,
                        message,
                        addr,
                    })
                    .ok()
                    .ok_or(Error::DhtDead)?;
                Ok(())
            }
            MessageKind::FindNodeRequest(req) => {
                let want = req
                    .want
                    .unwrap_or(if addr.is_ipv6() { Want::V6 } else { Want::V4 });
                let (nodes, nodes6) = self.generate_compact_nodes_both(req.target, want);
                self.get_table_for_addr(addr)
                    .write()
                    .mark_last_query(&req.id, now());
                let message = Message {
                    transaction_id: msg.transaction_id,
                    version: None,
                    ip: Some(addr),
                    kind: MessageKind::Response(bprotocol::Response {
                        id: self.id,
                        nodes,
                        nodes6,
                        ..Default::default()
                    }),
                };
                self.worker_sender
                    .send(WorkerSendRequest {
                        our_tid: None,
                        message,
                        addr,
                    })
                    .ok()
                    .ok_or(Error::DhtDead)?;
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    pub fn get_stats(&self) -> DhtStats {
        DhtStats {
            id: self.id,
            outstanding_requests: self.inflight_by_transaction_id.len(),
            routing_table_size: self.routing_table_v4.read().len(),
            routing_table_size_v6: self.routing_table_v6.read().len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Request {
    GetPeers(Id20),
    FindNode(Id20),
    Announce {
        info_hash: Id20,
        token: ByteBufOwned,
        port: u16,
    },
    Ping,
}

enum ResponseOrError {
    Response(Response<ByteBufOwned>),
    Error(ErrorDescription<ByteBufOwned>),
}

impl core::fmt::Debug for ResponseOrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Response(r) => write!(f, "{r:?}"),
            Self::Error(e) => write!(f, "{e:?}"),
        }
    }
}

struct DhtWorker {
    socket: UdpSocket,
    dht: Arc<DhtState>,
}

impl DhtWorker {
    fn on_send_error(&self, cx: &Cx, tid: u16, addr: SocketAddr, err: crate::Error) {
        if let Some((_, OutstandingRequest { done })) =
            self.dht.inflight_by_transaction_id.remove(&(tid, addr))
        {
            let _ = done.send(cx, Err(err)).is_err();
        };
    }

    async fn bootstrap_hostname(&self, cx: &Cx, hostname: &str) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let addrs = if let Ok(address) = hostname.parse::<SocketAddr>() {
            vec![address]
        } else {
            let (host, port) = hostname.rsplit_once(':').ok_or_else(||
                Error::lookup(hostname, std::io::Error::new(std::io::ErrorKind::InvalidInput, "DHT bootstrap address has no port")))?;
            let port = port.parse::<u16>().map_err(|error| Error::lookup(hostname,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, error)))?;
            self.socket.resolve_host(cx, host, port).await.map_err(|error| Error::lookup(hostname, error))?
        };
        let v4 = RecursiveRequest::find_node_for_routing_table(
            cx,
            self.dht.clone(),
            self.dht.id,
            addrs.iter().copied().filter(|a| a.is_ipv4()),
        )
        .instrument(debug_span!("v4"));

        let v6 = RecursiveRequest::find_node_for_routing_table(
            cx,
            self.dht.clone(),
            self.dht.id,
            addrs.iter().copied().filter(|a| a.is_ipv6()),
        )
        .instrument(debug_span!("v6"));

        let (v4, v6) = futures::join!(v4, v6);
        v4.or(v6)
    }

    async fn bootstrap_hostname_with_backoff(&self, cx: &Cx, addr: &str) -> crate::Result<()> {
        let started = Instant::now();
        let mut retry = Duration::from_secs(1);
        loop {
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            match self.bootstrap_hostname(cx, addr).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if started.elapsed() > Duration::from_secs(86400) {
                        return Err(err);
                    }
                    warn!(?retry, ?addr, "error in bootstrap: {err:#}");
                    rqbit_runtime::time::sleep(cx, retry).await;
                    retry = (retry * 2).min(Duration::from_secs(60));
                }
            }
        }
    }

    async fn bootstrap(&self, cx: &Cx, bootstrap_addrs: &[String]) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let mut futs = FuturesUnordered::new();

        for addr in bootstrap_addrs.iter() {
            futs.push(self.bootstrap_hostname_with_backoff(cx, addr));
        }
        let mut successes = 0;
        while let Some(resp) = futs.next().await {
            if resp.is_ok() {
                successes += 1
            }
        }
        if successes == 0 {
            return Err(Error::BootstrapFailed);
        }
        Ok(())
    }

    async fn bucket_refresher(&self, cx: &Cx, is_v4: bool) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let table = if is_v4 {
            &self.dht.routing_table_v4
        } else {
            &self.dht.routing_table_v6
        };
        let mut interval = rqbit_runtime::time::interval(cx, INACTIVITY_TIMEOUT);
        interval.tick(cx).await;
        loop {
            interval.tick(cx).await;
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            let targets = table
                .read()
                .iter_buckets()
                .filter(|b| {
                    now().saturating_duration_since(b.leaf.last_refreshed) >= INACTIVITY_TIMEOUT
                })
                .map(|b| b.random_within())
                .collect::<Vec<_>>();
            for target in targets {
                cx.checkpoint().map_err(|_| Error::Cancelled)?;
                let addrs = table
                    .read()
                    .sorted_by_distance_from(target, now())
                    .iter()
                    .map(|n| n.addr())
                    .take(8)
                    .collect::<Vec<_>>();
                if let Err(err) = RecursiveRequest::find_node_for_routing_table(
                    cx,
                    self.dht.clone(),
                    target,
                    addrs.into_iter(),
                )
                .await
                {
                    debug!("error refreshing DHT bucket: {err:#}");
                }
            }
        }
    }

    async fn pinger(&self, cx: &Cx, is_v4: bool) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let table = if is_v4 {
            &self.dht.routing_table_v4
        } else {
            &self.dht.routing_table_v6
        };
        let mut interval = rqbit_runtime::time::interval(cx, INACTIVITY_TIMEOUT / 4);
        loop {
            interval.tick(cx).await;
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            let nodes = table
                .read()
                .iter()
                .filter(|n| {
                    matches!(
                        n.status(now()),
                        NodeStatus::Questionable | NodeStatus::Unknown
                    )
                })
                .map(|n| (n.id(), n.addr()))
                .collect::<Vec<_>>();
            futures::stream::iter(nodes)
                .map(|(id, addr)| async move {
                    table.write().mark_outgoing_request(&id, now());
                    match self.dht.request(cx, Request::Ping, addr).await {
                        Ok(_) => {
                            table.write().mark_response(&id, now());
                        }
                        Err(err) => {
                            table.write().mark_error(&id);
                            debug!("error pinging DHT node: {err:#}");
                        }
                    }
                })
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;
        }
    }

    async fn framer(
        &self,
        cx: &Cx,
        socket: &UdpSocket,
        mut input_rx: UnboundedReceiver<WorkerSendRequest>,
        output_tx: Sender<(Message<ByteBufOwned>, SocketAddr)>,
    ) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let writer = async {
            let mut buf = Vec::new();
            while let Ok(WorkerSendRequest {
                our_tid,
                message,
                addr,
            }) = input_rx.recv(cx).await
            {
                cx.checkpoint().map_err(|_| Error::Cancelled)?;
                if our_tid.is_none() {
                    trace!("{}: sending {:?}", addr, &message);
                }
                buf.clear();
                bprotocol::serialize_message(
                    &mut buf,
                    message.transaction_id,
                    message.version,
                    message.ip,
                    message.kind,
                )
                .unwrap();
                if let Err(e) = socket.send_to(cx, &buf, addr).await {
                    debug!("error sending to {addr}: {e:#}");
                    if let Some(tid) = our_tid {
                        self.on_send_error(cx, tid, addr, Error::Send(e));
                    }
                }
            }
            Err(Error::DhtDead)
        };
        let reader = async {
            let mut buf = vec![0u8; 16384];
            loop {
                cx.checkpoint().map_err(|_| Error::Cancelled)?;
                let (size, addr) = socket.recv_from(cx, &mut buf).await.map_err(Error::Recv)?;
                match bprotocol::deserialize_message::<ByteBufOwned>(&buf[..size]) {
                    Ok(msg) => match output_tx.send(cx, (msg, addr)).await {
                        Ok(_) => {}
                        Err(_) => return Err(Error::DhtDead),
                    },
                    Err(e) => debug!("{}: error deserializing incoming message: {}", addr, e),
                }
            }
        };
        futures::pin_mut!(writer, reader);
        match futures::future::select(writer, reader).await {
            futures::future::Either::Left((result, _))
            | futures::future::Either::Right((result, _)) => result,
        }
    }

    async fn start(
        self,
        cx: &Cx,
        in_rx: UnboundedReceiver<WorkerSendRequest>,
        bootstrap_addrs: &[String],
    ) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let (out_tx, mut out_rx) = channel(1);
        let framer = self
            .framer(cx, &self.socket, in_rx, out_tx)
            .instrument(debug_span!("dht_framer"));

        let bootstrap = self.bootstrap(cx, bootstrap_addrs).fuse();

        let response_reader = {
            let this = &self;
            async move {
                while let Ok((response, addr)) = out_rx.recv(cx).await {
                    if let Err(e) = this.dht.on_received_message(cx, response, addr) {
                        debug!("error in on_response, addr={:?}: {}", addr, e)
                    }
                }
                Err(Error::DhtDead)
            }
        }
        .instrument(debug_span!("dht_response_reader"));

        let pinger_v4 = self.pinger(cx, true).instrument(debug_span!("pinger_v4"));
        let bucket_refresher_v4 = self
            .bucket_refresher(cx, true)
            .instrument(debug_span!("bucket_refresher_v4"));

        let pinger_v6 = self.pinger(cx, false).instrument(debug_span!("pinger_v6"));
        let bucket_refresher_v6 = self
            .bucket_refresher(cx, false)
            .instrument(debug_span!("bucket_refresher_v6"));

        let framer = framer.fuse();
        futures::pin_mut!(framer);
        futures::pin_mut!(bootstrap);
        let response_reader = response_reader.fuse();
        futures::pin_mut!(response_reader);
        let pinger_v4 = pinger_v4.fuse();
        futures::pin_mut!(pinger_v4);
        let bucket_refresher_v4 = bucket_refresher_v4.fuse();
        futures::pin_mut!(bucket_refresher_v4);
        let pinger_v6 = pinger_v6.fuse();
        futures::pin_mut!(pinger_v6);
        let bucket_refresher_v6 = bucket_refresher_v6.fuse();
        futures::pin_mut!(bucket_refresher_v6);

        loop {
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            futures::select! {
                err = framer => {
                    return Error::task_finished(&"framer", err);
                },
                result = bootstrap => {
                    result?;
                },
                err = pinger_v4 => {
                    return Error::task_finished(&"pinger_v4", err);
                },
                err = bucket_refresher_v4 => {
                    return Error::task_finished(&"bucket_refresher_v4", err);
                },
                err = pinger_v6 => {
                    return Error::task_finished(&"pinger_v6", err);
                },
                err = bucket_refresher_v6 => {
                    return Error::task_finished(&"bucket_refresher_v6", err);
                },
                err = response_reader => {
                    return Error::task_finished(&"response_reader", err);
                }
            }
        }
    }
}

#[derive(Default)]
pub struct DhtConfig<'a> {
    pub peer_id: Option<Id20>,
    pub bootstrap_addrs: Option<Vec<String>>,
    pub routing_table: Option<RoutingTable>,
    pub routing_table_v6: Option<RoutingTable>,
    pub listen_addr: Option<SocketAddr>,
    pub peer_store: Option<PeerStore>,
    pub cancellation_token: Option<CancellationToken>,
    pub bind_device: Option<&'a BindDevice>,
}

impl DhtState {
    pub async fn new(cx: &Cx) -> crate::Result<Arc<Self>> {
        Self::with_config(cx, DhtConfig::default()).await
    }
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    #[inline(never)]
    pub fn with_config<'a>(
        cx: &'a Cx,
        mut config: DhtConfig<'a>,
    ) -> BoxFuture<'a, crate::Result<Arc<Self>>> {
        async move {
            cx.checkpoint().map_err(|_| Error::Cancelled)?;
            let addr = config
                .listen_addr
                .unwrap_or((Ipv6Addr::UNSPECIFIED, 0).into());
            let socket = UdpSocket::bind_udp(
                addr,
                librqbit_dualstack_sockets::BindOpts {
                    request_dualstack: true,
                    reuseport: false,
                    device: config.bind_device,
                },
            )
            .map_err(|e| Error::Bind(Box::new(e)))?;

            let listen_addr = socket.bind_addr();
            info!("DHT listening on {:?}", listen_addr);

            let peer_id = config
                .peer_id
                .unwrap_or_else(|| generate_azereus_style(*b"rQ", crate_version!()));
            info!("starting up DHT with peer id {:?}", peer_id);
            let bootstrap_addrs = config
                .bootstrap_addrs
                .unwrap_or_else(|| crate::DHT_BOOTSTRAP.iter().map(|v| v.to_string()).collect());

            let token = config.cancellation_token.take().unwrap_or_default();

            let (in_tx, in_rx) = unbounded_channel();
            let state = Arc::new(Self::new_internal(
                peer_id,
                in_tx,
                config.routing_table,
                config.routing_table_v6,
                listen_addr,
                config.peer_store.unwrap_or_else(|| PeerStore::new(peer_id)),
                token,
            ));

            spawn_with_cancel(
                cx,
                debug_span!("dht"),
                "dht",
                state.cancellation_token.clone(),
                {
                    let state = state.clone();
                    move |child_cx| async move {
                        let worker = DhtWorker { socket, dht: state };
                        worker.start(&child_cx, in_rx, &bootstrap_addrs).await
                    }
                },
            );
            Ok(state)
        }
        .boxed()
    }

    pub fn get_peers(
        self: &Arc<Self>,
        cx: &Cx,
        info_hash: Id20,
        announce_port: Option<u16>,
    ) -> RequestPeersStream {
        RequestPeersStream::new(cx, self.clone(), info_hash, announce_port)
    }

    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    pub fn stats(&self) -> DhtStats {
        self.get_stats()
    }

    pub fn with_routing_tables<R, F: FnOnce(&RoutingTable, &RoutingTable) -> R>(&self, f: F) -> R {
        f(&self.routing_table_v4.read(), &self.routing_table_v6.read())
    }

    // pub fn clone_routing_table(&self) -> RoutingTable {
    //     self.routing_table.read().clone()
    // }
}

trait FromSocketAddr: Sized {
    fn from_socket_addr(addr: SocketAddr) -> Option<Self>;
}

impl FromSocketAddr for SocketAddrV4 {
    fn from_socket_addr(addr: SocketAddr) -> Option<Self> {
        match addr {
            SocketAddr::V4(a) => Some(a),
            _ => None,
        }
    }
}

impl FromSocketAddr for SocketAddrV6 {
    fn from_socket_addr(addr: SocketAddr) -> Option<Self> {
        match addr {
            SocketAddr::V6(a) => Some(a),
            _ => None,
        }
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use librqbit_core::compact_ip::Compact;
    use rqbit_runtime::{Budget, RuntimeBuilder};
    use std::{future::Future, net::Ipv4Addr};

    #[test]
    fn native_dht_bootstrap_and_get_peers_use_udp() {
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
            let region = cx
                .open_child_region(rqbit_runtime::cx::ChildRegionSpec::inherit())
                .await
                .unwrap();
            let cx = region.cx().clone();
            let seed =
                UdpSocket::bind_udp((Ipv4Addr::LOCALHOST, 0).into(), Default::default()).unwrap();
            let seed_addr = seed.bind_addr();
            let hash = Id20::new([3; 20]);
            let peer = SocketAddr::from((Ipv4Addr::LOCALHOST, 4567));
            let (discovered, mut observed) = rqbit_runtime::sync::oneshot::channel();
            let seed_task = rqbit_runtime::task::spawn_with(&cx, move |cx| async move {
                let mut discovered = Some(discovered);
                let mut buffer = [0; 2048];
                loop {
                    let (size, source) = match seed.recv_from(&cx, &mut buffer).await {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return,
                        Err(error) => panic!("DHT seed receive failed: {error}"),
                    };
                    let message =
                        bprotocol::deserialize_message::<ByteBufOwned>(&buffer[..size]).unwrap();
                    let values = match message.kind {
                        MessageKind::GetPeersRequest(request) => {
                            assert_eq!(request.info_hash, hash);
                            if let Some(tx) = discovered.take() {
                                tx.send(&cx, ()).unwrap();
                            }
                            Some(vec![Compact(peer)])
                        }
                        MessageKind::FindNodeRequest(_) | MessageKind::PingRequest(_) => None,
                        other => panic!("unexpected DHT request {other:?}"),
                    };
                    let response = Response::<ByteBufOwned> {
                        id: Id20::new([4; 20]),
                        values,
                        nodes: Some(CompactNodeInfo::new_from_iter(std::iter::once(Node {
                            id: Id20::new([4; 20]),
                            addr: match seed_addr {
                                SocketAddr::V4(addr) => addr,
                                _ => unreachable!(),
                            },
                        }))),
                        token: Some(ByteBufOwned::from(b"seed".as_slice())),
                        ..Default::default()
                    };
                    let mut bytes = Vec::new();
                    bprotocol::serialize_message(
                        &mut bytes,
                        message.transaction_id,
                        None,
                        None,
                        MessageKind::Response(response),
                    )
                    .unwrap();
                    seed.send_to(&cx, &bytes, source).await.unwrap();
                }
            });
            let dht = DhtState::with_config(
                &cx,
                DhtConfig {
                    listen_addr: Some((Ipv4Addr::LOCALHOST, 0).into()),
                    bootstrap_addrs: Some(vec![seed_addr.to_string()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            // A bootstrap response puts the seed in the routing table. get_peers
            // starts immediately and retries roots after one second if needed.
            let mut peers = dht.get_peers(&cx, hash, None);
            let found = rqbit_runtime::time::timeout(&cx, Duration::from_secs(3), peers.next())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(found, peer);
            observed.recv(&cx).await.unwrap();
            peers.cancel_join_handle_v4.abort();
            peers.cancel_join_handle_v6.abort();
            let (v4, v6) = futures::join!(
                &mut peers.cancel_join_handle_v4,
                &mut peers.cancel_join_handle_v6
            );
            v4.expect("IPv4 discovery drains after cancellation acknowledgement");
            v6.expect("IPv6 discovery drains after cancellation acknowledgement");
            drop(peers);
            dht.cancellation_token().cancel();
            seed_task.abort();
            seed_task
                .await
                .expect("seed shutdown preserves its acknowledged completion");
            region.close().await.unwrap();
            assert_eq!(
                Arc::strong_count(&dht),
                1,
                "all protocol child owners are drained"
            );
            let rebound = std::net::UdpSocket::bind(dht.listen_addr())
                .expect("drained DHT releases its socket");
            drop(rebound);
        });
        runtime.block_on(task).unwrap();
    }

    #[test]
    fn dropped_dht_request_releases_pending_transaction() {
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
            let (sender, _receiver) = unbounded_channel();
            let state = DhtState::new_internal(
                Id20::new([5; 20]),
                sender,
                None,
                None,
                (Ipv4Addr::LOCALHOST, 0).into(),
                PeerStore::new(Id20::new([5; 20])),
                CancellationToken::new(),
            );
            let mut request =
                Box::pin(state.request(&cx, Request::Ping, (Ipv4Addr::LOCALHOST, 9999).into()));
            std::future::poll_fn(|task| {
                assert!(request.as_mut().poll(task).is_pending());
                Poll::Ready(())
            })
            .await;
            assert_eq!(state.inflight_by_transaction_id.len(), 1);
            drop(request);
            assert!(state.inflight_by_transaction_id.is_empty());
        });
        runtime.block_on(task).unwrap();
    }
}
