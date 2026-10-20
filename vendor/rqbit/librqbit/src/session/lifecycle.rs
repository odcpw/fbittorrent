//! Session construction and shutdown of its owned runtime region.

use super::*;

impl Session {
    /// Create a new session with default options.
    /// The passed in folder will be used as a default unless overridden per torrent.
    /// It will run a DHT server/client, a TCP listener and .
    #[inline(never)]
    pub fn new(cx: &Cx, default_output_folder: PathBuf) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        Self::new_with_opts(cx, default_output_folder, SessionOptions::default())
    }

    /// Create a new session with options.
    #[inline(never)]
    pub fn new_with_opts(
        cx: &Cx,
        default_output_folder: PathBuf,
        mut opts: SessionOptions,
    ) -> BoxFuture<'static, anyhow::Result<Arc<Self>>> {
        let parent_cx = cx.clone();
        async move {
            anyhow::ensure!(opts.persistence.is_none() && !opts.fastresume,
                "fbittorrent owns persistence; upstream persistence and fastresume are unavailable");
            anyhow::ensure!(!opts.listen.as_ref().is_some_and(|listen| listen.enable_upnp_port_forwarding),
                "UPnP port forwarding is unavailable in the native integration");
            let region = parent_cx.open_child_region(ChildRegionSpec::inherit()).await?;
            let cx = region.cx().clone();
            cx.checkpoint()?;
            let peer_id = opts
                .peer_id
                .unwrap_or_else(|| generate_azereus_style(*b"rQ", crate_version!()));
            let token = opts.cancellation_token.take().unwrap_or_default();

            #[cfg(feature = "disable-upload")]
            if opts.disable_upload {
                warn!(target: super::LOG_TARGET, "uploading disabled");
            }

            let bind_device = match opts.bind_device_name.as_ref() {
                Some(name) => Some(
                    BindDevice::new_from_name(name)
                        .with_context(|| format!("error creating bind device {name}"))?,
                ),
                None => None,
            };

            let listen_result = if let Some(listen_opts) = opts.listen.take() {
                Some(
                    listen_opts
                        .start(
                            &cx,
                            opts.root_span.as_ref().and_then(|s| s.id()),
                            token.child_token(),
                            bind_device.as_ref(),
                        )
                        .await
                        .context("error starting listeners")?,
                )
            } else {
                None
            };

            let dht = if let Some(dht_config) = opts.dht.take() {
                let dht = if let Some(persistence_config) = dht_config.persistence {
                    PersistentDht::create(
                        &cx,
                        persistence_config,
                        dht_config.port,
                        opts.ipv4_only,
                        dht_config.bootstrap_addrs,
                        Some(token.clone()),
                        bind_device.as_ref(),
                    )
                    .await
                    .context("error initializing persistent DHT")?
                } else {
                    let listen_addr = dht_listen_addr(dht_config.port, None, opts.ipv4_only);
                    DhtBuilder::with_config(&cx, DhtConfig {
                        bootstrap_addrs: dht_config.bootstrap_addrs,
                        cancellation_token: Some(token.child_token()),
                        bind_device: bind_device.as_ref(),
                        listen_addr: Some(listen_addr),
                        ..Default::default()
                    })
                    .await
                    .context("error initializing DHT")?
                };

                Some(dht)
            } else {
                None
            };
            let mut peer_opts = opts
                .connect
                .as_ref()
                .and_then(|p| p.peer_opts)
                .unwrap_or_default();
            peer_opts.encryption = Some(if opts.peer_encryption == crate::PeerEncryption::Require {
                crate::PeerEncryption::Require
            } else { peer_opts.encryption.unwrap_or(opts.peer_encryption) });

            const DEFAULT_BLOCKING_THREADS_IF_NOT_SET: usize = 8;
            let spawner = BlockingSpawner::new(
                opts.runtime_worker_threads
                    .unwrap_or(DEFAULT_BLOCKING_THREADS_IF_NOT_SET),
            );

            let bitv_factory: Arc<dyn BitVFactory> = Arc::new(NonPersistentBitVFactory {});
            if opts.connect.as_ref().and_then(|s| s.proxy_url.as_ref()).is_some() {
                bail!("proxy connections are unavailable in the native TCP engine");
            }
            let client_name_and_version = opts
                .client_name_and_version
                .unwrap_or_else(|| crate::client_name_and_version().to_owned());

            let http_client = Client::new_bound(&client_name_and_version,
                bind_device.as_ref().map(|device| device.binding().clone()));
            let stream_connector = Arc::new(
                StreamConnector::new(&cx, StreamConnectorArgs {
                    enable_tcp: opts.connect.as_ref().map(|c| c.enable_tcp).unwrap_or(true),

                    bind_device: bind_device.clone(),
                    ipv4_only: opts.ipv4_only,
                })
                .await
                .context("error creating stream connector")?,
            );

            let blocklist = if let Some(blocklist_url) = opts.blocklist_url {
                info!(target: super::LOG_TARGET, url = blocklist_url, "loading p2p blocklist");
                let bl = IpRanges::load_from_url_with_client(&cx, &blocklist_url, &http_client)
                    .await
                    .with_context(|| format!("error reading blocklist from {blocklist_url}"))?;
                info!(target: super::LOG_TARGET, len = bl.len(), "loaded blocklist");
                bl
            } else {
                IpRanges::default()
            };

            let allowlist = if let Some(allowlist_url) = opts.allowlist_url {
                info!(target: super::LOG_TARGET, url = allowlist_url, "loading p2p allowlist");
                let al = IpRanges::load_from_url_with_client(&cx, &allowlist_url, &http_client)
                    .await
                    .with_context(|| format!("error reading allowlist from {allowlist_url}"))?;
                info!(target: super::LOG_TARGET, len = al.len(), "loaded allowlist");
                Some(al)
            } else {
                None
            };

            let udp_tracker_client = UdpTrackerClient::new(&cx, token.clone(), bind_device.as_ref())
                .await
                .context("error creating UDP tracker client")?;

            let lsd = {
                if opts.disable_local_service_discovery {
                    None
                } else {
                    LocalServiceDiscovery::new(&cx, LocalServiceDiscoveryOptions {
                        cancel_token: token.clone(),
                        bind_device: bind_device.as_ref(),
                        ..Default::default()
                    })
                    .await
                    .inspect_err(|e| warn!(target: super::LOG_TARGET, "error starting local service discovery: {e:#}"))
                    .ok()
                }
            };

            let session = Arc::new(Self {
                cx: cx.clone(),
                region: parking_lot::Mutex::new(Some(region)),
                bitv_factory,
                peer_id,
                dht,
                peer_opts,
                spawner: spawner.clone(),
                output_folder: default_output_folder,
                next_id: AtomicUsize::new(0),
                db: RwLock::new(Default::default()),
                _cancellation_token_drop_guard: token.clone().drop_guard(),
                cancellation_token: token,
                announce_port: listen_result.as_ref().and_then(|l| l.announce_port),
                listen_addr: listen_result.as_ref().map(|l| l.addr),
                default_storage_factory: opts.default_storage_factory,
                http_client,
                connector: stream_connector,
                root_span: opts.root_span,
                stats: Arc::new(SessionStats::new()),
                concurrent_initialize_semaphore: Arc::new(Semaphore::new(
                    opts.concurrent_init_limit.unwrap_or(3),
                )),
                udp_tracker_client,
                ratelimits: Limits::new(opts.ratelimits),
                ipv4_only: opts.ipv4_only,
                trackers: opts.trackers,
                disable_trackers: opts.disable_trackers,
                peer_limit: opts.peer_limit,
                client_name_and_version,

                #[cfg(feature = "disable-upload")]
                _disable_upload: opts.disable_upload,
                upload_enabled: Arc::new(AtomicBool::new(opts.upload_enabled)),
                blocklist,
                allowlist,
                lsd,
            });

            if let Some(mut listen) = listen_result {
                if let Some(tcp) = listen.tcp_socket.take() {
                    let max_pending_incoming_handshake_checks =
                        listen.max_pending_incoming_handshake_checks;
                    session.spawn(
                        debug_span!(target: super::LOG_TARGET, parent: session.rs(), "tcp_listen", addr = ?listen.addr),
                        "tcp_listen",
                        {
                            let this = session.clone();
                            move |cx| Box::pin(async move {
                                this.task_listener(&cx, tcp, max_pending_incoming_handshake_checks)
                                    .await
                            })
                        },
                    );
                }
            }

            session.start_speed_estimator_updater();

            Ok(session)
        }
        .boxed()
    }

    /// Stop the session and all managed tasks.
    pub async fn stop(&self, _cx: &Cx) -> anyhow::Result<()> {
        self.cancellation_token.cancel();
        let torrents = self.db.read().torrents.values().cloned().collect::<Vec<_>>();
        for torrent in torrents {
            let _ = torrent.pause();
            torrent.close_run().await?;
        }
        self.cancellation_token.cancel();
        let region = self.region.lock().take();
        if let Some(region) = region {
            region.cancel(rqbit_runtime::CancelReason::user("session stopped"))?;
            region.close().await?;
        }
        Ok(())
    }
}
