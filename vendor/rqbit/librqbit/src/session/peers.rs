//! Incoming connections and peer discovery through trackers, DHT and LSD.

use super::*;

fn merge_two_optional_streams<T>(
    s1: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
    s2: Option<impl Stream<Item = T> + Unpin + Send + 'static>,
) -> Option<BoxStream<'static, T>> {
    match (s1, s2) {
        (Some(s1), None) => Some(Box::pin(s1)),
        (None, Some(s2)) => Some(Box::pin(s2)),
        (Some(s1), Some(s2)) => Some(Box::pin(merge_streams(s1, s2))),
        (None, None) => None,
    }
}

// Ad adapter for converting stats into the format that tracker_comms accepts.
struct PeerRxTorrentInfo {
    info_hash: Id20,
    session: Arc<Session>,
}

impl tracker_comms::TorrentStatsProvider for PeerRxTorrentInfo {
    fn get(&self) -> tracker_comms::TrackerCommsStats {
        let mt = self.session.with_torrents(|torrents| {
            for (_, mt) in torrents {
                if mt.info_hash() == self.info_hash {
                    return Some(mt.clone());
                }
            }
            None
        });
        let mt = match mt {
            Some(mt) => mt,
            None => {
                trace!(target: super::LOG_TARGET, info_hash=?self.info_hash, "can't find torrent in the session, using default stats");
                return Default::default();
            }
        };
        let stats = mt.stats();

        use crate::torrent_state::stats::TorrentStatsState as TS;
        use tracker_comms::TrackerCommsStatsState as S;

        tracker_comms::TrackerCommsStats {
            downloaded_bytes: stats.progress_bytes,
            total_bytes: stats.total_bytes,
            uploaded_bytes: stats.uploaded_bytes,
            torrent_state: match stats.state {
                TS::Initializing { .. } => S::Initializing,
                TS::Live => S::Live,
                TS::Paused => S::Paused,
                TS::Error => S::None,
            },
        }
    }
}

impl Session {
    async fn check_incoming_connection(
        self: Arc<Self>,
        cx: &Cx,
        addr: SocketAddr,
        kind: ConnectionKind,
        reader: BoxAsyncReadVectored,
        writer: BoxAsyncWrite,
    ) -> anyhow::Result<(Arc<TorrentStateLive>, CheckedIncomingConnection)> {
        let rwtimeout = self
            .peer_opts
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let incoming_ip = addr.ip();
        if self.blocklist.has(incoming_ip) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is in blocklist");
        }
        if self.allowlist.as_ref().is_some_and(|l| !l.has(incoming_ip)) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is not in allowlist");
        }

        let hashes = self.db.read().torrents.values().map(|torrent| torrent.info_hash()).collect::<Vec<_>>();
        let (reader, writer, read_buf, h, encrypted) = crate::mse::incoming_handshake(
            cx, reader, writer, self.peer_opts.encryption.unwrap_or_default(), &hashes, rwtimeout,
        ).await.context("error reading peer handshake")?;
        trace!(target: super::LOG_TARGET, "received handshake from {addr}: {:?}", h);

        if h.peer_id == self.peer_id {
            bail!("seems like we are connecting to ourselves, ignoring");
        }

        let (id, torrent) = self
            .db
            .read()
            .torrents
            .iter()
            .find(|(_, t)| t.info_hash() == h.info_hash)
            .map(|(id, t)| (*id, t.clone()))
            .with_context(|| format!("didn't find a matching torrent {:?}", h.info_hash))?;

        let live = torrent
            .live_wait_initializing(cx, Duration::from_secs(5))
            .await
            .with_context(|| format!("torrent {id} is not live, ignoring connection"))?;

        Ok((
            live,
            CheckedIncomingConnection {
                encrypted,
                addr,
                reader,
                writer,
                kind,
                handshake: h,
                read_buf,
            },
        ))
    }

    pub(super) async fn task_listener<A: Accept>(
        self: Arc<Self>,
        cx: &Cx,
        l: A,
        max_pending_incoming_handshake_checks: usize,
    ) -> anyhow::Result<()> {
        let mut futs = FuturesUnordered::new();
        let session = Arc::downgrade(&self);
        drop(self);

        loop {
            cx.checkpoint()?;
            rqbit_runtime::select! {
                r = l.accept(cx), if futs.len() < max_pending_incoming_handshake_checks => {
                    match r {
                        Ok((addr, (read, write))) => {
                            trace!(target: super::LOG_TARGET, "accepted connection from {addr}");
                            let session = session.upgrade().context("session is dead")?;
                            let span = debug_span!(target: super::LOG_TARGET, parent: session.rs(), "incoming", addr=%addr);
                            futs.push(
                                session.check_incoming_connection(cx, addr, A::KIND, Box::new(read), Box::new(write))
                                    .map_err(|e| {
                                        debug!(target: super::LOG_TARGET, "error checking incoming connection: {e:#}");
                                        e
                                    })
                                    .instrument(span)
                            );
                        }
                        Err(e) => {
                            warn!(target: super::LOG_TARGET, "error accepting: {e:#}");
                            // Whatever is the reason, ensure we are not stuck trying to
                            // accept indefinitely.
                            rqbit_runtime::time::sleep(cx, Duration::from_secs(10)).await;
                            cx.checkpoint()?;
                            continue
                        }
                    }
                },
                Some(Ok((live, checked))) = futs.next(), if !futs.is_empty() => {
                    let (addr, kind) = (checked.addr, checked.kind);
                    if let Err(e) = live.add_incoming_peer(cx, checked).await {
                        warn!(target: super::LOG_TARGET, ?addr, ?kind, "error handing over incoming connection: {e:#}");
                    }
                },
            }
        }
    }

    pub(super) fn merge_peer_opts(&self, other: Option<PeerConnectionOptions>) -> PeerConnectionOptions {
        let other = match other {
            Some(o) => o,
            None => self.peer_opts,
        };
        PeerConnectionOptions {
            encryption: Some(if self.peer_opts.encryption == Some(crate::PeerEncryption::Require) {
                crate::PeerEncryption::Require
            } else { other.encryption.or(self.peer_opts.encryption).unwrap_or_default() }),
            connect_timeout: other.connect_timeout.or(self.peer_opts.connect_timeout),
            read_write_timeout: other
                .read_write_timeout
                .or(self.peer_opts.read_write_timeout),
            keep_alive_interval: other
                .keep_alive_interval
                .or(self.peer_opts.keep_alive_interval),
        }
    }

    pub fn make_peer_rx_managed_torrent(
        self: &Arc<Self>,
        cx: &Cx,
        t: &Arc<ManagedTorrent>,
        announce: bool,
    ) -> Option<PeerStream> {
        let is_private = t.with_metadata(|m| m.info.info().private).unwrap_or(false);
        self.make_peer_rx(
            cx,
            t.info_hash(),
            t.tracker_urls(),
            announce,
            t.shared().options.force_tracker_interval,
            t.shared().options.initial_peers.clone(),
            is_private,
            Some(t.shared().trackers.clone()),
            t.shared().tracker_stats.clone(),
        )
    }

    // Get a peer stream from both DHT and trackers.
    pub(super) fn make_peer_rx(
        self: &Arc<Self>,
        cx: &Cx,
        info_hash: Id20,
        mut trackers: Vec<url::Url>,
        announce: bool,
        force_tracker_interval: Option<Duration>,
        initial_peers: Vec<SocketAddr>,
        is_private: bool,
        mutable_trackers: Option<Arc<tracker_comms::TrackerSet>>,
        tracker_state: Arc<tracker_comms::TrackerStats>,
    ) -> Option<PeerStream> {
        let dht_rx = if is_private {
            None
        } else {
            self.dht.as_ref().map(|dht| {
                dht.get_peers(cx, info_hash, if announce { self.announce_port } else { None })
            })
        };

        let lsd_rx = if is_private {
            None
        } else {
            self.lsd.as_ref().map(|lsd| {
                lsd.announce(cx, info_hash, if announce { self.announce_port } else { None })
            })
        };

        tracker_state.register(trackers.iter().cloned());
        for url in &trackers {
            if !matches!(url.scheme(), "http" | "https" | "udp") {
                tracker_state.not_contacted(url, "unsupported scheme");
            } else if self.disable_trackers {
                tracker_state.not_contacted(url, "trackers disabled");
            }
        }
        if self.disable_trackers {
            trackers.clear();
        }

        if is_private {
            if trackers.len() > 1 {
                warn!(target: super::LOG_TARGET,
                    ?info_hash,
                    "private trackers are not fully implemented, so using only the first tracker"
                );
            }
            // BEP 27: never add public/session trackers, including when the
            // private metainfo has only one tracker (or none).
            for url in trackers.iter().skip(1) {
                if matches!(url.scheme(), "http" | "https" | "udp") {
                    tracker_state.not_contacted(url, "private torrent: only the first tracker is announced");
                }
            }
            trackers.truncate(1);
        } else if !self.disable_trackers {
            trackers.extend(self.trackers.iter().cloned());
        }

        let tracker_rx_stats = PeerRxTorrentInfo {
            info_hash,
            session: self.clone(),
        };
        let tracker_rx = TrackerComms::start_with_stats(
            cx,
            info_hash,
            self.peer_id,
            mutable_trackers.unwrap_or_else(|| tracker_comms::TrackerSet::with_stats(trackers, tracker_state.clone())),
            is_private,
            if !is_private && !self.disable_trackers { self.trackers.iter().cloned().collect() } else { Vec::new() },
            !self.disable_trackers,
            Box::new(tracker_rx_stats),
            force_tracker_interval,
            self.announce_port().unwrap_or(4240),
            self.http_client.clone(),
            self.udp_tracker_client.clone(),
            tracker_state,
        );

        let initial_peers_rx = if is_private || initial_peers.is_empty() {
            None
        } else {
            Some(futures::stream::iter(initial_peers))
        };
        merge_two_optional_streams(
            merge_two_optional_streams(
                merge_two_optional_streams(dht_rx, tracker_rx),
                initial_peers_rx,
            ),
            lsd_rx,
        )
    }
}
