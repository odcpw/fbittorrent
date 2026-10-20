//! Torrent metadata, file selection, destinations and admission.

use super::*;

struct ParsedTorrentFile {
    meta: TorrentMetaV1Owned,
    torrent_bytes: Bytes,
}

fn torrent_from_bytes(bytes: Bytes) -> anyhow::Result<ParsedTorrentFile> {
    // Metainfo carries tracker passkeys. Never dump its raw announce fields.
    trace!(target: super::LOG_TARGET, bytes = bytes.len(), "parsing torrent metadata");
    let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&bytes)?;
    Ok(ParsedTorrentFile {
        meta: parsed.clone_to_owned(Some(&bytes)),
        torrent_bytes: bytes,
    })
}

async fn torrent_from_url(cx: &Cx, client: &Client, url: &str) -> anyhow::Result<ParsedTorrentFile> {
    let bytes = client.get(cx, url).await.context("error downloading torrent metadata")?;
    torrent_from_bytes(bytes.into()).context("error decoding torrent")
}

fn compute_only_files_regex<ByteBuf: AsRef<[u8]>>(
    torrent: &ValidatedTorrentMetaV1Info<ByteBuf>,
    filename_re: &str,
) -> anyhow::Result<Vec<usize>> {
    let filename_re = regex::Regex::new(filename_re).context("filename regex is incorrect")?;
    let mut only_files = Vec::new();
    for (idx, fd) in torrent.iter_file_details().enumerate() {
        let full_path = fd.filename.to_pathbuf();
        if filename_re.is_match(full_path.to_str().unwrap()) {
            only_files.push(idx);
        }
    }
    if only_files.is_empty() {
        bail!("none of the filenames match the given regex")
    }
    Ok(only_files)
}

fn compute_only_files(
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
    only_files: Option<Vec<usize>>,
    only_files_regex: Option<String>,
    list_only: bool,
) -> anyhow::Result<Option<Vec<usize>>> {
    match (only_files, only_files_regex) {
        (Some(_), Some(_)) => {
            bail!("only_files and only_files_regex are mutually exclusive");
        }
        (Some(only_files), None) => {
            let total_files = info.iter_file_lengths().count();
            for id in only_files.iter().copied() {
                if id >= total_files {
                    bail!("file id {} is out of range", id);
                }
            }
            Ok(Some(only_files))
        }
        (None, Some(filename_re)) => {
            let only_files = compute_only_files_regex(info, &filename_re)?;
            for (idx, fd) in info.iter_file_details().enumerate() {
                if !only_files.contains(&idx) {
                    continue;
                }
                if !list_only {
                    info!(target: super::LOG_TARGET, filename=?fd.filename, "will download");
                }
            }
            Ok(Some(only_files))
        }
        (None, None) => Ok(None),
    }
}

pub(super) fn torrent_file_from_info_bytes(info_bytes: &[u8], trackers: &[url::Url]) -> anyhow::Result<Bytes> {
    #[derive(Serialize)]
    struct Tmp<'a> {
        announce: &'a str,
        #[serde(rename = "announce-list")]
        announce_list: &'a [&'a [url::Url]],
        info: bencode::raw_value::RawValue<&'a [u8]>,
    }

    let mut w = Vec::new();
    let v = Tmp {
        info: bencode::raw_value::RawValue(info_bytes),
        announce: trackers.first().map(|s| s.as_str()).unwrap_or(""),
        announce_list: &[trackers],
    };
    bencode_serialize_to_writer(&v, &mut w)?;
    Ok(w.into())
}

struct InternalAddResult {
    info_hash: Id20,
    metadata: Option<TorrentMetadata>,
    trackers: Vec<url::Url>,
    name: Option<String>,
}

pub(super) fn derived_output_folder(session_folder: &Path, folder: &Path, subfolder: Option<PathBuf>) -> bool {
    subfolder.is_some_and(|name| {
        !name.as_os_str().is_empty()
            && folder != session_folder
            && session_folder.join(name) == folder
    })
}

impl Session {
    /// Add a torrent to the session.
    #[inline(never)]
    pub fn add_torrent<'a>(
        self: &'a Arc<Self>,
        cx: &'a Cx,
        add: AddTorrent<'a>,
        opts: Option<AddTorrentOptions>,
    ) -> BoxFuture<'a, anyhow::Result<AddTorrentResponse>> {
        async move {
            cx.checkpoint()?;
            let mut opts = opts.unwrap_or_default();
            let add_res = match add {
                AddTorrent::Url(magnet) if magnet.starts_with("magnet:") || magnet.len() == 40 => {
                    let magnet = Magnet::parse(&magnet)
                        .context("provided path is not a valid magnet URL")?;
                    let info_hash = magnet
                        .as_id20()
                        .context("magnet link didn't contain a BTv1 infohash")?;
                    if !opts.list_only {
                        let existing = self.with_torrents(|torrents| {
                            for (id, handle) in torrents {
                                if handle.info_hash() == info_hash {
                                    return Some((id, handle.clone()));
                                }
                            }
                            None
                        });
                        if let Some((id, handle)) = existing {
                            return Ok(AddTorrentResponse::AlreadyManaged(id, handle));
                        }
                    }
                    if let Some(so) = magnet.get_select_only() {
                        // Only overwrite opts.only_files if user didn't specify
                        if opts.only_files.is_none() {
                            opts.only_files = Some(so);
                        }
                    }

                    InternalAddResult {
                        info_hash,
                        trackers: magnet
                            .trackers
                            .into_iter()
                            .filter_map(|t| url::Url::parse(&t).ok())
                            .collect(),
                        metadata: None,
                        name: magnet.name,
                    }
                }
                other => {
                    let torrent = match other {
                        AddTorrent::Url(url)
                            if url.starts_with("http://") || url.starts_with("https://") =>
                        {
                            torrent_from_url(cx, &self.http_client, &url).await?
                        }
                        AddTorrent::Url(url) => {
                            bail!(
                                "unsupported URL {:?}. Supporting magnet:, http:, and https",
                                url
                            )
                        }
                        AddTorrent::TorrentFileBytes(bytes) => {
                            torrent_from_bytes(bytes).context("error decoding torrent")?
                        }
                    };

                    let mut trackers = torrent
                        .meta
                        .iter_announce()
                        .unique()
                        .filter_map(|tracker| match std::str::from_utf8(tracker.as_ref()) {
                            Ok(url) => Some(url.to_owned()),
                            Err(_) => {
                                warn!(target: super::LOG_TARGET, "cannot parse tracker url as utf-8, ignoring");
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    if !torrent.meta.info.data.private
                        && let Some(custom_trackers) = opts.trackers.clone()
                    {
                        trackers.extend(custom_trackers);
                    }

                    InternalAddResult {
                        info_hash: torrent.meta.info_hash,
                        metadata: Some(TorrentMetadata::new(
                            torrent.meta.info.data.validate()?,
                            torrent.torrent_bytes,
                            torrent.meta.info.raw_bytes.0,
                        )?),
                        trackers: trackers
                            .iter()
                            .filter_map(|t| url::Url::parse(t).ok())
                            .collect(),
                        name: None,
                    }
                }
            };

            self.add_torrent_internal(cx, add_res, opts).await
        }
        .instrument(debug_span!(target: super::LOG_TARGET, parent: self.rs(), "add_torrent"))
        .boxed()
    }

    /// The torrent-named folder this session derives under its own folder.
    /// Only that folder is rqbit's: it may be opened without following a
    /// planted symlink and removed when a delete-with-files empties it.
    fn is_derived_output_folder(
        &self,
        folder: &Path,
        info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
        magnet_name: Option<&str>,
    ) -> bool {
        derived_output_folder(
            &self.output_folder,
            folder,
            self.get_default_subfolder_for_torrent(info, magnet_name)
                .ok()
                .flatten(),
        )
    }

    fn get_default_subfolder_for_torrent(
        &self,
        info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
        magnet_name: Option<&str>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let files = info
            .iter_file_details()
            .map(|fd| Ok((fd.filename.to_pathbuf(), fd.len)))
            .collect::<anyhow::Result<Vec<(PathBuf, u64)>>>()?;
        if files.len() < 2 {
            return Ok(None);
        }

        fn check_valid(pb: &Path) -> anyhow::Result<()> {
            if pb.components().any(|x| !matches!(x, Component::Normal(_))) {
                bail!("path traversal in torrent name detected")
            }
            Ok(())
        }

        if let Some(name) = info.name()
            && !name.is_empty()
        {
            let pb = PathBuf::from(name.as_ref());
            check_valid(&pb)?;
            return Ok(Some(pb));
        };
        if let Some(name) = magnet_name {
            let pb = PathBuf::from(name);
            check_valid(&pb)?;
            return Ok(Some(pb));
        }
        // Let the subfolder name be the longest filename
        let longest = files
            .iter()
            .max_by_key(|(_, l)| l)
            .unwrap()
            .0
            .file_stem()
            .context("can't determine longest filename")?;
        Ok::<_, anyhow::Error>(Some(PathBuf::from(longest)))
    }

    async fn add_torrent_internal(
        self: &Arc<Self>,
        cx: &Cx,
        add_res: InternalAddResult,
        mut opts: AddTorrentOptions,
    ) -> anyhow::Result<AddTorrentResponse> {
        let InternalAddResult {
            info_hash,
            metadata,
            trackers,
            name,
        } = add_res;

        let private = metadata.as_ref().is_some_and(|m| m.info.info().private);

        let mut seen_peers = Vec::new();
        let metadata = match metadata {
            Some(metadata) => metadata,
            None => {
                let region = self.cx.open_child_region(ChildRegionSpec::inherit()).await?;
                let metadata_cx = region.cx().clone();
                let result = async {
                    let peer_rx = self.make_peer_rx(&metadata_cx, info_hash, trackers.clone(), false,
                        opts.force_tracker_interval, opts.initial_peers.clone().unwrap_or_default(), private, None, Arc::new(tracker_comms::TrackerStats::default()))
                        .context("no known way to resolve metadata peers")?;
                    self.resolve_magnet(&metadata_cx, info_hash, peer_rx, &trackers, opts.peer_opts).await
                }.await;
                region.cancel(rqbit_runtime::CancelReason::user("metadata resolution finished"))?;
                region.close().await?;
                let resolved = result?;
                seen_peers = resolved.seen_peers;
                let initial = opts.initial_peers.get_or_insert_with(Vec::new);
                initial.extend(seen_peers.iter().copied());
                initial.sort_unstable();
                initial.dedup();
                resolved.metadata
            }
        };
        let peer_rx = None;

        if metadata.info.info().private {
            // This also discards peers seen while resolving magnet metadata:
            // that merged stream did not preserve tracker provenance. The
            // managed private stream will obtain fresh peers from its tracker.
            opts.initial_peers = None;
            seen_peers.clear();
        }

        trace!(target: super::LOG_TARGET, "Torrent metadata: {:#?}", &metadata.info.info());

        let only_files = compute_only_files(
            &metadata.info,
            opts.only_files,
            opts.only_files_regex,
            opts.list_only,
        )?;

        let output_folder = match (opts.output_folder, opts.sub_folder) {
            (None, None) => self.output_folder.join(
                self.get_default_subfolder_for_torrent(&metadata.info, name.as_deref())?
                    .unwrap_or_default(),
            ),
            (Some(o), None) => PathBuf::from(o),
            (Some(_), Some(_)) => {
                bail!("you can't provide both output_folder and sub_folder")
            }
            (None, Some(s)) => self.output_folder.join(s),
        };

        let derived_output_folder =
            self.is_derived_output_folder(&output_folder, &metadata.info, name.as_deref());

        if opts.list_only {
            return Ok(AddTorrentResponse::ListOnly(ListOnlyResponse {
                info_hash,
                info: metadata.info,
                only_files,
                output_folder,
                seen_peers,
                torrent_bytes: metadata.torrent_bytes,
            }));
        }

        let storage_factory = opts
            .storage_factory
            .take()
            .or_else(|| self.default_storage_factory.as_ref().map(|f| f.clone_box()))
            .unwrap_or_else(|| FilesystemStorageFactory::default().boxed());

        let id = if let Some(id) = opts.preferred_id {
            id
        } else {
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        };

        let _permit = rqbit_runtime::sync::OwnedSemaphorePermit::acquire(self.spawner.semaphore(), cx, 1).await?;

        let (managed_torrent, metadata) = {
            let mut g = self.db.write();
            if let Some((id, handle)) = g.torrents.iter().find_map(|(eid, t)| {
                if t.info_hash() == info_hash || *eid == id {
                    Some((*eid, t.clone()))
                } else {
                    None
                }
            }) {
                return Ok(AddTorrentResponse::AlreadyManaged(id, handle));
            }

            let span = debug_span!(target: super::LOG_TARGET, parent: self.rs(), "torrent", id);
            let peer_opts = self.merge_peer_opts(opts.peer_opts);
            let metadata = Arc::new(metadata);
            let mut trackers = trackers;
            for value in &opts.tracker_additions {
                let url=url::Url::parse(value).context("invalid saved tracker URL")?;
                tracker_comms::validate_tracker(&url)?;
                if !trackers.contains(&url) {trackers.push(url);}
            }
            trackers.retain(|url| !opts.tracker_removals.iter().any(|v| v == url.as_str()));
            let tracker_stats = opts.tracker_stats.take().unwrap_or_else(||Arc::new(tracker_comms::TrackerStats::default()));
            tracker_stats.register(trackers.iter().cloned());
            for (index,url) in trackers.iter().enumerate() {
                if !matches!(url.scheme(), "http"|"https"|"udp") {
                    tracker_stats.not_contacted(url,"unsupported scheme");
                } else if metadata.info.info().private && index > 0 {
                    tracker_stats.not_contacted(url,"private torrent: only the first tracker is announced");
                }
            }
            let minfo = Arc::new(ManagedTorrentShared {
                cx: self.cx.clone(),
                id,
                span,
                info_hash,
                trackers: tracker_comms::TrackerSet::with_stats(trackers, tracker_stats.clone()),
                tracker_stats,
                spawner: self.spawner.clone(),
                peer_id: self.peer_id,
                storage_factory,
                options: ManagedTorrentOptions {
                    trusted_have_pieces: parking_lot::Mutex::new(opts.trusted_have_pieces.take()),
                    force_tracker_interval: opts.force_tracker_interval,
                    peer_connect_timeout: peer_opts.connect_timeout,
                    peer_encryption: peer_opts.encryption.unwrap_or_default(),
                    peer_read_write_timeout: peer_opts.read_write_timeout,
                    allow_overwrite: opts.overwrite,
                    output_folder,
                    derived_output_folder,
                    ratelimits: Limits::new(opts.ratelimits),
                    initial_peers: opts.initial_peers.clone().unwrap_or_default(),
                    peer_limit: opts.peer_limit.or(self.peer_limit),
                    session_upload_enabled: self.upload_enabled.clone(),
                    upload_override: std::sync::atomic::AtomicU8::new(match opts.upload_enabled {None=>0,Some(false)=>1,Some(true)=>2}),
                    #[cfg(feature = "disable-upload")]
                    _disable_upload: self._disable_upload,
                },
                connector: self.connector.clone(),
                session: Arc::downgrade(self),
                magnet_name: name,
                client_name_and_version: self.client_name_and_version.clone(),
            });

            let initializing = Arc::new(TorrentStateInitializing::new(
                minfo.clone(),
                metadata.clone(),
                only_files.clone(),
                self.spawner
                    .block_in_place(|| minfo.storage_factory.create_and_init(&minfo, &metadata))?,
                false,
            ));
            let handle = Arc::new(ManagedTorrent {
                locked: RwLock::new(ManagedTorrentLocked {
                    paused: opts.paused,
                    state: ManagedTorrentState::Initializing(initializing),
                    only_files,
                }),
                run_region: parking_lot::Mutex::new(None),
                state_change_notify: Notify::new(),
                shared: minfo,
                metadata: ArcSwapOption::new(Some(metadata.clone())),
            });

            g.add_torrent(handle.clone(), id);
            (handle, metadata)
        };

        managed_torrent
            .start(cx, peer_rx, opts.paused).await
            .context("error starting torrent")?;

        if let Some(name) = metadata.info.name() {
            info!(target: super::LOG_TARGET, ?name, "added torrent");
        }

        Ok(AddTorrentResponse::Added(id, managed_torrent))
    }

    async fn resolve_magnet(
        self: &Arc<Self>,
        cx: &Cx,
        info_hash: Id20,
        peer_rx: PeerStream,
        trackers: &[url::Url],
        peer_opts: Option<PeerConnectionOptions>,
    ) -> anyhow::Result<ResolveMagnetResult> {
        match read_metainfo_from_peer_receiver(
            cx,
            self.peer_id,
            info_hash,
            Default::default(),
            peer_rx,
            Some(self.merge_peer_opts(peer_opts)),
            self.connector.clone(),
            self.client_name_and_version.clone(),
        )
        .await
        {
            ReadMetainfoResult::Found {
                info,
                info_bytes,
                _rx: _,
                seen,
            } => {
                trace!(target: super::LOG_TARGET, ?info, "received result from DHT");
                let info = info.validate()?;
                Ok(ResolveMagnetResult {
                    metadata: TorrentMetadata::new(
                        info,
                        torrent_file_from_info_bytes(info_bytes.as_ref(), trackers)?,
                        info_bytes.0,
                    )?,
                    seen_peers: {
                        let seen = seen.into_iter().collect_vec();
                        for peer in &seen {
                            trace!(target: super::LOG_TARGET, ?peer, "seen")
                        }
                        seen
                    },
                })
            }
            ReadMetainfoResult::ChannelClosed { .. } => {
                bail!("input address stream exhausted, no way to discover torrent metainfo")
            }
        }
    }

    pub async fn create_and_serve_torrent(
        self: &Arc<Self>,
        cx: &Cx,
        path: &Path,
        opts: CreateTorrentOptions<'_>,
    ) -> Result<(CreateTorrentResult, ManagedTorrentHandle), ApiError> {
        if !path.exists() {
            return Err(ApiError::from((
                StatusCode::BAD_REQUEST,
                "path doesn't exist",
            )));
        }

        let torrent = create_torrent(cx, path, opts, &self.spawner)
            .await
            .with_status(StatusCode::BAD_REQUEST)?;

        let bytes = torrent.as_bytes()?;

        let handle = self
            .add_torrent(
                cx,
                AddTorrent::TorrentFileBytes(bytes.clone()),
                Some(AddTorrentOptions {
                    paused: false,
                    overwrite: true,
                    output_folder: Some(
                        torrent
                            .output_folder
                            .to_str()
                            .context("invalid utf-8")?
                            .to_owned(),
                    ),
                    ..Default::default()
                }),
            )
            .await?
            .into_handle()
            .context("error adding to session")?;

        Ok((torrent, handle))
    }
}
