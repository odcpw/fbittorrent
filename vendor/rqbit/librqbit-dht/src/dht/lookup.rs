//! Recursive node and peer lookups, including their cancellable result stream.

use super::*;

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
            trace!(target: super::LOG_TARGET,
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

impl RequestPeersStream {
    pub(super) fn new(cx: &Cx, dht: Arc<DhtState>, info_hash: Id20, announce_port: Option<u16>) -> Self {
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
    pub(super) async fn find_node_for_routing_table(
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
                    debug!(target: super::LOG_TARGET, "error: {e:#}");
                    e
                })
                .instrument(debug_span!(target: super::LOG_TARGET,
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
        debug!(target: super::LOG_TARGET,
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
            debug_span!(target: super::LOG_TARGET, parent: None, "get_peers", is_v4, info_hash = format!("{:?}", self.info_hash)),
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
                            trace!(target: super::LOG_TARGET, "iteration {}", iteration);
                            let sleep = match this.get_peers_root(is_v4) {
                                Ok(0) => Duration::from_secs(1),
                                Ok(n) if n < 8 => REQUERY_INTERVAL / 8 * (n as u32),
                                Ok(_) => REQUERY_INTERVAL,
                                Err(e) => {
                                    error!(target: super::LOG_TARGET, "dht: error in get_peers_root(): {e:#}");
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
                                .map_err(|e| debug!(target: super::LOG_TARGET, "error: {e:#}"))
                                .instrument(debug_span!(target: super::LOG_TARGET, "addr", addr = addr.to_string())),
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
                debug!(target: super::LOG_TARGET, "error response: {e:?}");
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
            trace!(target: super::LOG_TARGET,
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
