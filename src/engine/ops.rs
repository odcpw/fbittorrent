//! Typed engine operations; legacy route parsing lives at the transport boundary.
use super::*;
mod parser;
mod types;
pub use parser::{LEGACY_ROUTES, Route};
pub use types::*;

impl EngineDispatcher {
    pub fn restore_outcomes(&self) -> HashMap<String, RestoreOutcome> {
        self.inner
            .restores
            .lock()
            .unwrap()
            .iter()
            .map(|(hash, restore)| (hash.clone(), restore.outcome.clone()))
            .collect()
    }
    /// Legacy compatibility entry point. All execution below takes typed data.
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        match EngineOp::parse(&request.path, request) {
            Ok(Some(op)) => Some(self.execute(op).and_then(|result| result.to_legacy_json())),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
    pub fn execute(&self, op: EngineOp) -> Result<EngineResult> {
        op.validate()?;
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            rqbit_runtime::select! {
                result = self.execute_async(cx, op) => result,
                _ = self.inner.session.cancellation_token().cancelled() => Err(anyhow!("native engine is closing")),
            }
        })
    }
    async fn execute_async(&self, cx: &Cx, op: EngineOp) -> Result<EngineResult> {
        if matches!(
            op,
            EngineOp::Network | EngineOp::Settings | EngineOp::Restart | EngineOp::Shutdown
        ) {
            let snapshot = self
                .lifecycle
                .expected_network()
                .unwrap_or_else(|| crate::network_app::state(&Config::default()));
            return op
                .control(&snapshot, &self.lifecycle)
                .expect("control operation");
        }
        match &op {
            EngineOp::Version => {
                return Ok(EngineResult::Version(format!(
                    "native librqbit {}",
                    librqbit::version()
                )));
            }
            EngineOp::Queue => {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                let (mut active, mut queued) = (0, 0);
                for record in &ledger.records {
                    if let Ok(handle) = self.inner.handle(&record.hash) {
                        let stats = handle.stats();
                        if !stats.finished && !matches!(stats.state, TorrentStatsState::Error) {
                            if !handle.is_paused() {
                                active += 1;
                            } else if !record.paused {
                                queued += 1;
                            }
                        }
                    }
                }
                return Ok(EngineResult::Queue(QueueInfo {
                    max_active_downloads: self.inner.max_active_downloads.load(Ordering::Acquire),
                    active_downloads: active,
                    queued_downloads: queued,
                    network_blocked: self.inner.network_blocked.load(Ordering::Acquire),
                }));
            }
            EngineOp::SetQueue { limit } => {
                self.inner.set_download_limit(cx, *limit).await?;
                return Ok(EngineResult::Acknowledged);
            }
            EngineOp::SetPreferences => bail!(
                "qBittorrent preferences do not apply to the native engine; edit fbittorrent configuration"
            ),
            EngineOp::TransferInfo => {
                let stats = self.inner.session.stats_snapshot();
                return Ok(EngineResult::Transfer(TransferInfo {
                    dl_info_speed: stats.download_speed.as_bytes() as i64,
                    up_info_speed: stats.upload_speed.as_bytes() as i64,
                    dl_info_data: stats.counters.fetched_bytes as i64,
                    up_info_data: stats.counters.uploaded_bytes as i64,
                    connection_status: "connected".into(),
                }));
            }
            EngineOp::Info => return Ok(EngineResult::Torrents(self.info(cx).await?)),
            EngineOp::Files { hash } => {
                let handle = self.inner.handle(hash)?;
                let metadata = handle
                    .metadata
                    .load_full()
                    .context("torrent metadata is still loading")?;
                let stats = handle.stats();
                let selected = handle.only_files();
                return Ok(EngineResult::Files(
                    metadata
                        .file_infos
                        .iter()
                        .enumerate()
                        .map(|(index, file)| {
                            let progress = stats.file_progress.get(index).copied().unwrap_or(0);
                            TorrentFile {
                                index: index as i64,
                                name: file.relative_filename.to_string_lossy().into_owned(),
                                size: file.len as i64,
                                progress: if file.len == 0 {
                                    1.0
                                } else {
                                    progress as f64 / file.len as f64
                                },
                                progress_known: true,
                                priority: if selected
                                    .as_ref()
                                    .map(|ids| ids.contains(&index))
                                    .unwrap_or(true)
                                {
                                    1
                                } else {
                                    0
                                },
                            }
                        })
                        .collect(),
                ));
            }
            EngineOp::Add(options) => {
                self.add_typed(cx, options).await?;
                return Ok(EngineResult::Acknowledged);
            }
            EngineOp::ToggleSequentialDownload => bail!(
                "the native engine always downloads each file sequentially; this setting cannot be disabled"
            ),
            EngineOp::ToggleFirstLastPiecePrio => bail!(
                "the native engine always prioritizes the first and last piece of each file; this setting cannot be disabled"
            ),
            _ => {}
        }
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        if let EngineOp::FilePriority {
            hash,
            ids,
            priority,
        } = &op
        {
            let handle = self.inner.handle(hash)?;
            let metadata = handle
                .metadata
                .load_full()
                .context("torrent metadata is still loading")?;
            let mut selected: HashSet<usize> = handle
                .only_files()
                .unwrap_or_else(|| (0..metadata.file_infos.len()).collect())
                .into_iter()
                .collect();
            for &index in ids {
                if index >= metadata.file_infos.len() {
                    bail!("file index {index} is out of range");
                }
                if *priority == 0 {
                    selected.remove(&index);
                } else {
                    selected.insert(index);
                }
            }
            if !handle.is_paused() && !matches!(handle.stats().state, TorrentStatsState::Error) {
                self.inner.pause_handle(cx, &handle).await?;
            }
            self.inner
                .session
                .update_only_files(cx, &handle, &selected)
                .await?;
            let record = ledger
                .records
                .iter_mut()
                .find(|record| &record.hash == hash)
                .context("torrent record not found")?;
            let mut selected: Vec<_> = selected.into_iter().collect();
            selected.sort_unstable();
            record.only_files = Some(selected);
            record.completion_on = 0;
            self.inner.save(cx, &ledger).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok(EngineResult::Acknowledged);
        }
        let (hashes, start_result) = mutation_hashes(&op, &ledger)?;
        if matches!(
            op,
            EngineOp::SetUploadLimit { .. } | EngineOp::Delete { files: true, .. }
        ) {
            for hash in &hashes {
                self.inner.handle(hash)?;
            }
        }
        if let EngineOp::Delete { files, .. } = &op {
            let replacement = Ledger {
                generation: ledger.generation.clone(),
                migrated_legacy: ledger.migrated_legacy,
                records: ledger
                    .records
                    .iter()
                    .filter(|record| !hashes.contains(&record.hash))
                    .cloned()
                    .collect(),
            };
            self.inner.save(cx, &replacement).await?;
            ledger.records = replacement.records;
            for hash in hashes {
                self.inner.cancel_restore(&hash, true);
                if let Ok(handle) = self.inner.handle(&hash) {
                    self.inner
                        .session
                        .delete(cx, TorrentIdOrHash::Id(handle.id()), *files)
                        .await?;
                }
                let _ = rqbit_runtime::fs::remove_file(self.inner.torrent_path(&hash)?).await;
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok(EngineResult::Acknowledged);
        }
        let retry_errors = if matches!(op, EngineOp::Start { .. }) {
            hashes.iter().cloned().collect()
        } else {
            HashSet::new()
        };
        for hash in hashes {
            let index = ledger
                .records
                .iter()
                .position(|record| record.hash == hash)
                .context("torrent record not found")?;
            let old = ledger.records[index].clone();
            let mut record = old.clone();
            let handle = self.inner.handle(&hash).ok();
            match &op {
                EngineOp::Start { .. } => {
                    record.paused = false;
                    record.resume_after_migration = false;
                    if handle.as_ref().is_some_and(|handle| {
                        matches!(handle.stats().state, TorrentStatsState::Error)
                    }) {
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                EngineOp::Stop { .. } => {
                    record.paused = true;
                    record.resume_after_migration = false;
                }
                EngineOp::SetUploadLimit { limit, .. } => {
                    record.upload_limit = *limit;
                    record.options(record.paused)?;
                    if record.upload_limit != old.upload_limit {
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                EngineOp::SetShareLimits { ratio, .. } => {
                    record.ratio_limit = *ratio;
                    if let Some(handle) = &handle {
                        if *ratio == 0.0 && handle.stats().finished && !handle.is_paused() {
                            self.inner.pause_handle(cx, handle).await?;
                        }
                    }
                }
                EngineOp::AddTags { tags, .. } | EngineOp::RemoveTags { tags, .. } => {
                    let mut saved: HashSet<String> = record
                        .tags
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_owned)
                        .collect();
                    for tag in tags {
                        if matches!(op, EngineOp::AddTags { .. }) {
                            saved.insert(tag.clone());
                        } else {
                            saved.remove(tag);
                        }
                    }
                    let mut tags: Vec<_> = saved.into_iter().collect();
                    tags.sort();
                    record.tags = tags.join(",");
                }
                _ => bail!("operation is not a torrent mutation"),
            }
            ledger.records[index] = record;
            if let Err(error) = self.inner.save(cx, &ledger).await {
                ledger.records[index] = old;
                let _ = self.inner.save(cx, &ledger).await;
                return Err(error).context("saving torrent intent; previous intent restored");
            }
            if matches!(op, EngineOp::Stop { .. }) {
                self.inner.cancel_restore(&hash, false);
                if let Some(handle) = &handle {
                    if matches!(handle.stats().state, TorrentStatsState::Initializing { .. }) {
                        self.inner.pause_handle(cx, handle).await?;
                    }
                }
            }
            if matches!(op, EngineOp::Start { .. }) && handle.is_none() {
                self.inner
                    .spawn_restore(cx, ledger.records[index].clone())?;
            }
        }
        self.inner
            .reconcile_queue(cx, &mut ledger, &retry_errors)
            .await?;
        Ok(match start_result {
            Some(result) => EngineResult::Start(result),
            None => EngineResult::Acknowledged,
        })
    }
    #[cfg(test)]
    pub(super) async fn add(&self, cx: &Cx, request: &Request) -> Result<String> {
        let Some(EngineOp::Add(options)) = EngineOp::parse(Route::Add.name(), request)? else {
            unreachable!()
        };
        self.add_typed(cx, &options).await?;
        EngineResult::Acknowledged.to_legacy_json()
    }
    async fn add_typed(&self, cx: &Cx, options: &AddRequest) -> Result<()> {
        // Serialize additions without holding the ledger while resolving magnets.
        // A duplicate add must observe the first add's durable record.
        let _addition = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
        if self.inner.network_blocked.load(Ordering::Acquire) {
            bail!("connect ProtonVPN before adding a download");
        }
        let paused = options.stopped;
        let mut record = Record {
            hash: String::new(),
            name: options
                .torrent
                .as_ref()
                .map(|(name, _)| name.clone())
                .unwrap_or_default(),
            source: String::new(),
            output_folder: options
                .save_path
                .clone()
                .unwrap_or_else(|| self.inner.default_folder.to_string_lossy().into()),
            only_files: None,
            upload_limit: options.upload_limit,
            ratio_limit: options.ratio_limit,
            category: options.category.clone(),
            tags: options.tags.clone(),
            added_on: now(),
            completion_on: 0,
            paused,
            resume_after_migration: false,
        };
        let sources = if options.torrent.is_some() {
            vec![String::new()]
        } else {
            options.sources.clone()
        };
        for source in sources {
            record.source = source.clone();
            let add = if let Some((_, bytes)) = &options.torrent {
                AddTorrent::from_bytes(bytes.clone())
            } else if Path::new(&source).is_file() {
                AddTorrent::from_local_filename(&source)?
            } else {
                AddTorrent::from_url(source)
            };
            // Persist metadata and policy before allowing payload exchange.
            let mut native_options = record.options(true)?;
            // Let rqbit create a named subdirectory for new multi-file torrents.
            if options.save_path.is_none() {
                native_options.output_folder = None;
            }
            let response = time::timeout(
                cx,
                Duration::from_secs(90),
                self.inner
                    .session
                    .add_torrent(cx, add, Some(native_options)),
            )
            .await
            .context("torrent metadata lookup timed out")??;
            let existed = matches!(&response, AddTorrentResponse::AlreadyManaged(_, _));
            let handle = response.into_handle().context("torrent was only listed")?;
            if existed {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                if !ledger
                    .records
                    .iter()
                    .any(|saved| saved.hash == handle.info_hash().as_string())
                {
                    bail!("torrent is being restored; retry the add shortly");
                }
                // Also completes a prior paused add whose record fsync failed.
                self.inner.save(cx, &ledger).await?;
                continue;
            }
            record.hash = handle.info_hash().as_string();
            record.name = handle.name().unwrap_or_else(|| record.hash.clone());
            record.output_folder = handle.output_folder().to_string_lossy().into_owned();
            if let Err(error) = self.inner.cache_metadata(cx, &handle).await {
                let _ = self
                    .inner
                    .session
                    .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
                    .await;
                return Err(error)
                    .context("saving torrent metadata; torrent stopped without deleting files");
            }
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            // The first durable record is paused. A failed first save must never
            // leave an acknowledged-running record for the next app opening.
            record.paused = true;
            ledger.records.push(record.clone());
            if let Err(error) = self.inner.save(cx, &ledger).await {
                // Retain the paused record in memory: rename may have completed
                // before an fsync failed, so deleting it could contradict disk.
                return Err(error).context("saving torrent records; torrent remains paused");
            }
            if !paused {
                let saved = ledger
                    .records
                    .iter_mut()
                    .find(|saved| saved.hash == record.hash)
                    .unwrap();
                saved.paused = false;
                if let Err(error) = self.inner.save(cx, &ledger).await {
                    ledger
                        .records
                        .iter_mut()
                        .find(|saved| saved.hash == record.hash)
                        .unwrap()
                        .paused = true;
                    let _ = self.inner.save(cx, &ledger).await;
                    return Err(error).context("saving requested torrent; torrent remains paused");
                }
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
        }
        Ok(())
    }
}
pub(super) fn required<'a>(request: &'a Request, key: &str) -> Result<&'a str> {
    request
        .arg(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing {key}"))
}

pub(super) fn mutation_hashes(
    op: &EngineOp,
    ledger: &Ledger,
) -> Result<(Vec<String>, Option<crate::qbt::StartResult>)> {
    let selection = op
        .selection()
        .context("operation has no torrent selection")?;
    let hashes = selection.resolve(ledger)?;
    let EngineOp::Start { skip_complete, .. } = op else {
        return Ok((hashes, None));
    };
    let bulk = matches!(selection, HashSelection::All) || *skip_complete;
    let mut result = crate::qbt::StartResult::default();
    for hash in hashes {
        let record = ledger
            .records
            .iter()
            .find(|record| record.hash == hash)
            .expect("validated hash");
        if record.completion_on > 0 && record.stop_on_complete() {
            if !bulk {
                return Err(crate::agent::AppError::new(
                    crate::agent::Code::Failure,
                    "already complete; seeding is off for this torrent",
                )
                .hint(format!("fbittorrent seed {hash}"))
                .into());
            }
            result.skipped.push(crate::qbt::StartSkipped {
                hash,
                reason: "complete_no_seed".into(),
            });
        } else {
            result.started.push(hash);
        }
    }
    Ok((result.started.clone(), Some(result)))
}
pub(super) fn json(value: &impl Serialize) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "tests/ops.rs"]
mod tests;
