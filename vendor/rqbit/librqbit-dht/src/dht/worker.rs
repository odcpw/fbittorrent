//! UDP framing, bootstrap and background routing-table maintenance.

use super::*;

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
        .instrument(debug_span!(target: super::LOG_TARGET, "v4"));

        let v6 = RecursiveRequest::find_node_for_routing_table(
            cx,
            self.dht.clone(),
            self.dht.id,
            addrs.iter().copied().filter(|a| a.is_ipv6()),
        )
        .instrument(debug_span!(target: super::LOG_TARGET, "v6"));

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
                    warn!(target: super::LOG_TARGET, ?retry, ?addr, "error in bootstrap: {err:#}");
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
                    debug!(target: super::LOG_TARGET, "error refreshing DHT bucket: {err:#}");
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
                            debug!(target: super::LOG_TARGET, "error pinging DHT node: {err:#}");
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
                    trace!(target: super::LOG_TARGET, "{}: sending {:?}", addr, &message);
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
                    debug!(target: super::LOG_TARGET, "error sending to {addr}: {e:#}");
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
                    Err(e) => debug!(target: super::LOG_TARGET, "{}: error deserializing incoming message: {}", addr, e),
                }
            }
        };
        futures::pin_mut!(writer, reader);
        match futures::future::select(writer, reader).await {
            futures::future::Either::Left((result, _))
            | futures::future::Either::Right((result, _)) => result,
        }
    }

    pub(super) async fn start(
        self,
        cx: &Cx,
        in_rx: UnboundedReceiver<WorkerSendRequest>,
        bootstrap_addrs: &[String],
    ) -> crate::Result<()> {
        cx.checkpoint().map_err(|_| Error::Cancelled)?;
        let (out_tx, mut out_rx) = channel(1);
        let framer = self
            .framer(cx, &self.socket, in_rx, out_tx)
            .instrument(debug_span!(target: super::LOG_TARGET, "dht_framer"));

        let bootstrap = self.bootstrap(cx, bootstrap_addrs).fuse();

        let response_reader = {
            let this = &self;
            async move {
                while let Ok((response, addr)) = out_rx.recv(cx).await {
                    if let Err(e) = this.dht.on_received_message(cx, response, addr) {
                        debug!(target: super::LOG_TARGET, "error in on_response, addr={:?}: {}", addr, e)
                    }
                }
                Err(Error::DhtDead)
            }
        }
        .instrument(debug_span!(target: super::LOG_TARGET, "dht_response_reader"));

        let pinger_v4 = self.pinger(cx, true).instrument(debug_span!(target: super::LOG_TARGET, "pinger_v4"));
        let bucket_refresher_v4 = self
            .bucket_refresher(cx, true)
            .instrument(debug_span!(target: super::LOG_TARGET, "bucket_refresher_v4"));

        let pinger_v6 = self.pinger(cx, false).instrument(debug_span!(target: super::LOG_TARGET, "pinger_v6"));
        let bucket_refresher_v6 = self
            .bucket_refresher(cx, false)
            .instrument(debug_span!(target: super::LOG_TARGET, "bucket_refresher_v6"));

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
