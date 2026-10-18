//! Existing string routes and torrent operations; typed operations follow in P1-19.
use super::*;

impl Inner {
    pub(super) fn handle(&self, hash: &str) -> Result<Arc<ManagedTorrent>> {
        self.session
            .with_torrents(|torrents| {
                for (_, handle) in torrents {
                    if handle.info_hash().as_string() == hash {
                        return Some(handle.clone());
                    }
                }
                None
            })
            .ok_or_else(|| anyhow!("torrent not found: {hash}"))
    }
    pub(super) async fn resolve_record(
        &self,
        cx: &Cx,
        record: &Record,
    ) -> Result<ListOnlyResponse> {
        let path = self.torrent_path(&record.hash)?;
        let mut options = record.options(true)?;
        options.list_only = true;
        let add = if path.exists() {
            AddTorrent::from_bytes(rqbit_runtime::fs::read(path).await?)
        } else if !record.source.is_empty() {
            AddTorrent::from_url(record.source.clone())
        } else {
            bail!("missing torrent metadata for {}", record.name)
        };
        let response = time::timeout(
            cx,
            Duration::from_secs(90),
            self.session.add_torrent(cx, add, Some(options)),
        )
        .await
        .context("torrent metadata lookup timed out")??;
        let AddTorrentResponse::ListOnly(metadata) = response else {
            bail!("torrent metadata preflight returned an unexpected managed torrent");
        };
        if metadata.info_hash.as_string() != record.hash {
            bail!("restored torrent hash does not match saved record");
        }
        Ok(metadata)
    }
    pub(super) async fn admit_record(
        &self,
        cx: &Cx,
        record: &Record,
        paused: bool,
        metadata: ListOnlyResponse,
    ) -> Result<Arc<ManagedTorrent>> {
        // A paused torrent still opens and verifies its files. Validate the hash
        // before admitting any storage, then apply the saved cap at admission.
        let mut options = record.options(paused)?;
        options.initial_peers = Some(metadata.seen_peers);
        let handle = self
            .session
            .add_torrent(
                cx,
                AddTorrent::from_bytes(metadata.torrent_bytes),
                Some(options),
            )
            .await?
            .into_handle()
            .context("torrent admission did not return a handle")?;
        Ok(handle)
    }
    pub(super) async fn restore_record(&self, cx: &Cx, record: &Record, paused: bool) -> Result<Arc<ManagedTorrent>> {
        let metadata = self.resolve_record(cx, record).await?;
        self.admit_record(cx, record, paused, metadata).await
    }

    pub(super) fn cancel_restore(&self, hash: &str, remove: bool) {
        let mut restores = self.restores.lock().unwrap();
        if let Some(restore) = restores.get_mut(hash) {
            // Invalidate the result before cancellation: a task completing at
            // the same time must not overwrite the user's stop/removal.
            restore.generation = RESTORE_GENERATION.fetch_add(1, Ordering::Relaxed);
            // Drop metadata lookup through a protocol stop, so its nested
            // region runs its close backstop before this task completes.
            restore.cancel.cancel();
            restore.outcome = RestoreOutcome::Stopped;
        }
        if remove {
            restores.remove(hash);
            drop(restores);
            self.cancelled_checks.lock().unwrap().remove(hash);
        }
    }

    pub(super) fn check_cancelled(&self, hash: &str, handle: &Arc<ManagedTorrent>) -> bool {
        self.cancelled_checks.lock().unwrap().get(hash).is_some_and(|saved|
            saved.upgrade().is_some_and(|saved| Arc::ptr_eq(&saved, handle)))
    }

    pub(super) async fn pause_handle(&self, cx: &Cx, handle: &Arc<ManagedTorrent>) -> Result<()> {
        // Session::pause waits for the check's region and blocking file work.
        // Publish cancellation only after that work has actually drained.
        self.session.pause(cx, handle).await?;
        if matches!(handle.stats().state, TorrentStatsState::Initializing { .. }) {
            self.cancelled_checks.lock().unwrap().insert(
                handle.info_hash().as_string(), Arc::downgrade(handle));
        }
        Ok(())
    }

    pub(super) async fn unpause_handle(&self, cx: &Cx, handle: &Arc<ManagedTorrent>) -> Result<()> {
        self.session.unpause(cx, handle).await?;
        self.cancelled_checks.lock().unwrap().remove(&handle.info_hash().as_string());
        Ok(())
    }

    pub(super) fn spawn_restore(self: &Arc<Self>, cx: &Cx, record: Record) -> Result<()> {
        cx.checkpoint()?;
        let mut restores = self.restores.lock().unwrap();
        if restores.get(&record.hash).is_some_and(|restore|
            matches!(restore.outcome, RestoreOutcome::Checking | RestoreOutcome::Metadata) &&
            restore.task.as_ref().is_some_and(|task| !task.is_finished())) {
            return Ok(());
        }
        let generation = RESTORE_GENERATION.fetch_add(1, Ordering::Relaxed);
        let last_error = restores.get(&record.hash).and_then(|restore| restore.last_error.clone());
        let outcome = if self.torrent_path(&record.hash)?.exists() {
            RestoreOutcome::Checking
        } else { RestoreOutcome::Metadata };
        let inner = self.clone();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let hash = record.hash.clone();
        let task = task::try_spawn_with(cx, move |cx| async move {
            let result = async {
                // Metadata lookup never owns the ledger. A single unresolved
                // magnet cannot block reads, mutations, or another restore.
                let metadata = rqbit_runtime::select! {
                    result = inner.resolve_record(&cx, &record) => result?,
                    _ = task_cancel.cancelled() => bail!("torrent restore cancelled"),
                };
                let mut ledger = OwnedMutexGuard::lock(inner.ledger.clone(), &cx).await?;
                cx.checkpoint()?;
                if inner.stopping.load(Ordering::Acquire) { bail!("native engine is closing"); }
                let current = ledger.records.iter().find(|saved| saved.hash == record.hash)
                    .context("torrent was removed during restore")?;
                // Stop/delete serialize against admission, and invalidate this
                // generation before releasing the ledger to the resolver.
                let current_generation = inner.restores.lock().unwrap().get(&record.hash)
                    .map(|restore| restore.generation);
                if current_generation != Some(generation) { return Ok(()); }
                inner.admit_record(&cx, current, true, metadata).await?;
                inner.reconcile_queue(&cx, &mut ledger, &HashSet::new()).await
            }.await;
            let mut restores = inner.restores.lock().unwrap();
            if let Some(restore) = restores.get_mut(&record.hash) {
                if restore.generation == generation {
                    restore.outcome = match result {
                        Ok(()) => {
                            restore.last_error = None;
                            RestoreOutcome::Restored
                        }
                        Err(error) => {
                            let message = format!("could not restore {}: {error:#}", record.name);
                            restore.last_error = Some(message.clone());
                            RestoreOutcome::Failed { reason: "restore_failed", message }
                        }
                    };
                }
            }
        })?;
        restores.insert(hash, RestoreTask { generation, outcome, last_error, task: Some(task), cancel });
        Ok(())
    }

    pub(super) fn begin_restores(self: &Arc<Self>, cx: &Cx, mut records: Vec<Record>) -> Result<()> {
        records.sort_by_cached_key(|record| !self.directory.join(format!("{}.torrent", record.hash)).exists());
        for record in records { self.spawn_restore(cx, record)?; }
        Ok(())
    }

    pub(super) async fn rebuild(&self, cx: &Cx, old: &Record, changed: &Record) -> Result<()> {
        let handle = self.handle(&old.hash)?;
        self.cache_metadata(cx, &handle).await?;
        self.session
            .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
            .await?;
        if let Err(error) = self.restore_record(cx, changed, true).await {
            // Restore the previous record if changing the cap cannot be applied.
            let rollback = self.restore_record(cx, old, true).await;
            if let Err(rollback) = rollback {
                bail!("changing upload cap failed: {error:#}; restoring previous torrent failed: {rollback:#}");
            }
            return Err(error).context("changing upload cap; previous torrent restored");
        }
        Ok(())
    }
}

impl EngineDispatcher {
    pub fn restore_outcomes(&self) -> HashMap<String, RestoreOutcome> {
        self.inner.restores.lock().unwrap().iter()
            .map(|(hash, restore)| (hash.clone(), restore.outcome.clone())).collect()
    }

    /// Returns None for routes owned by other client features.
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        if !matches!(
            request.path.as_str(),
            "app/version"
                | "app/shutdown"
                | "app/setPreferences"
                | "app/queue"
                | "app/setQueue"
                | "transfer/info"
                | "torrents/info"
                | "torrents/add"
                | "torrents/start"
                | "torrents/stop"
                | "torrents/delete"
                | "torrents/files"
                | "torrents/filePrio"
                | "torrents/setUploadLimit"
                | "torrents/setShareLimits"
                | "torrents/addTags"
                | "torrents/removeTags"
                | "torrents/toggleSequentialDownload"
                | "torrents/toggleFirstLastPiecePrio"
        ) {
            return None;
        }
        if self.inner.stopping.load(Ordering::Acquire) {
            return Some(Err(anyhow!("native engine is closing")));
        }
        let cx = self.runtime.cx();
        Some(self.runtime.block_on(async {
            rqbit_runtime::select! {
                result = self.dispatch_async(cx, request) => result,
                _ = self.inner.session.cancellation_token().cancelled() => Err(anyhow!("native engine is closing")),
            }
        }))
    }

    async fn dispatch_async(&self, cx: &Cx, request: &Request) -> Result<String> {
        match request.path.as_str() {
            "app/version" => return Ok(format!("native librqbit {}", librqbit::version())),
            "app/shutdown" => { self.lifecycle.request_stop(StopReason::Quit); return Ok("Ok.".into()); },
            "app/queue" => {
                let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                let mut active = 0;
                let mut queued = 0;
                for record in &ledger.records {
                    if let Ok(handle) = self.inner.handle(&record.hash) {
                        let stats = handle.stats();
                        if !stats.finished && !matches!(stats.state, TorrentStatsState::Error) {
                            if !handle.is_paused() { active += 1; }
                            else if !record.paused { queued += 1; }
                        }
                    }
                }
                return json(&serde_json::json!({
                    "max_active_downloads": self.inner.max_active_downloads.load(Ordering::Acquire),
                    "active_downloads": active,
                    "queued_downloads": queued,
                    "network_blocked": self.inner.network_blocked.load(Ordering::Acquire),
                }));
            },
            "app/setQueue" => {
                let limit = required(request, "max_active_downloads")?.parse()?;
                self.inner.set_download_limit(cx, limit).await?;
                return Ok("Ok.".into());
            },
            "app/setPreferences" => bail!("qBittorrent preferences do not apply to the native engine; edit fbittorrent configuration"),
            "transfer/info" => {
                let stats = self.inner.session.stats_snapshot();
                return json(&TransferInfo { dl_info_speed: stats.download_speed.as_bytes() as i64,
                    up_info_speed: stats.upload_speed.as_bytes() as i64,
                    dl_info_data: stats.counters.fetched_bytes as i64,
                    up_info_data: stats.counters.uploaded_bytes as i64,
                    connection_status: "connected".into() });
            },
            "torrents/info" => return self.info(cx).await,
            "torrents/files" => {
                let handle = self.inner.handle(required(request, "hash")?)?;
                let metadata = handle.metadata.load_full().context("torrent metadata is still loading")?;
                let stats = handle.stats();
                let selected = handle.only_files();
                let files: Vec<TorrentFile> = metadata.file_infos.iter().enumerate().map(|(index, file)| {
                    let progress = stats.file_progress.get(index).copied().unwrap_or(0);
                    TorrentFile { index: index as i64, name: file.relative_filename.to_string_lossy().into_owned(),
                        size: file.len as i64, progress: if file.len == 0 { 1.0 } else { progress as f64 / file.len as f64 },
                        progress_known: true,
                        priority: if selected.as_ref().map(|ids| ids.contains(&index)).unwrap_or(true) { 1 } else { 0 } }
                }).collect();
                return json(&files);
            },
            "torrents/add" => return self.add(cx, request).await,
            "torrents/toggleSequentialDownload" => bail!("the native engine always downloads each file sequentially; this setting cannot be disabled"),
            "torrents/toggleFirstLastPiecePrio" => bail!("the native engine always prioritizes the first and last piece of each file; this setting cannot be disabled"),
            _ => {},
        }
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        if request.path == "torrents/filePrio" {
            let hash = required(request, "hash")?;
            let priority: i64 = required(request, "priority")?.parse()?;
            if !matches!(priority, 0 | 1) {
                bail!("the native engine supports skip (0) and normal (1) file priorities; high and maximum are unavailable");
            }
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
            for value in required(request, "id")?.split('|') {
                let index: usize = value.parse()?;
                if index >= metadata.file_infos.len() {
                    bail!("file index {index} is out of range");
                }
                if priority == 0 {
                    selected.remove(&index);
                } else {
                    selected.insert(index);
                }
            }
            // A finished seeder can become an unfinished download when another
            // file is selected. Stop its run before changing the selected set.
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
                .find(|record| record.hash == hash)
                .context("torrent record not found")?;
            let mut selected: Vec<_> = selected.into_iter().collect();
            selected.sort_unstable();
            record.only_files = Some(selected);
            record.completion_on = 0;
            self.inner.save(cx, &ledger).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok("Ok.".into());
        }
        let (hashes, start_result) = mutation_hashes(request, &ledger)?;
        // Handle-dependent operations still validate the entire selection
        // before applying any changes; start/stop/remove are ledger intents.
        if request.path == "torrents/setUploadLimit" ||
            (request.path == "torrents/delete" && request.arg("deleteFiles") == Some("true")) {
            for hash in &hashes { self.inner.handle(hash)?; }
        }
        if request.path == "torrents/delete" {
            let delete_files = request.arg("deleteFiles") == Some("true");
            // Persist the removal before closing handles or deleting payload:
            // a write failure must leave the existing download recoverable.
            let retained = ledger
                .records
                .iter()
                .filter(|record| !hashes.contains(&record.hash))
                .cloned()
                .collect();
            let replacement = Ledger {
                migrated_legacy: ledger.migrated_legacy,
                records: retained,
            };
            self.inner.save(cx, &replacement).await?;
            ledger.records = replacement.records;
            for hash in hashes {
                self.inner.cancel_restore(&hash, true);
                if let Ok(handle) = self.inner.handle(&hash) {
                    self.inner.session.delete(cx, TorrentIdOrHash::Id(handle.id()), delete_files).await?;
                }
                let _ = rqbit_runtime::fs::remove_file(self.inner.torrent_path(&hash)?).await;
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            return Ok("Ok.".into());
        }
        let retry_errors = if request.path == "torrents/start" {
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
            match request.path.as_str() {
                "torrents/start" => {
                    record.paused = false;
                    record.resume_after_migration = false;
                    if handle.as_ref().is_some_and(|handle| matches!(handle.stats().state, TorrentStatsState::Error)) {
                        // Recheck a manually retried error while paused. Its
                        // future queue admission then survives a full queue.
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                "torrents/stop" => {
                    record.paused = true;
                    record.resume_after_migration = false;
                }
                "torrents/setUploadLimit" => {
                    let limit: i64 = required(request, "limit")?.parse()?;
                    if limit < 0 {
                        bail!("upload limit must be zero (unlimited) or a positive byte rate");
                    }
                    record.upload_limit = limit;
                    record.options(record.paused)?;
                    if record.upload_limit != old.upload_limit {
                        self.inner.rebuild(cx, &old, &record).await?;
                    }
                }
                "torrents/setShareLimits" => {
                    let ratio: f64 = required(request, "ratioLimit")?.parse()?;
                    let action = request.arg("shareLimitAction").unwrap_or("Default");
                    if ratio != -2.0 && (ratio != 0.0 || !action.eq_ignore_ascii_case("stop")) {
                        bail!("native share limits support unlimited (-2) or stop on completion (ratio 0, Stop)");
                    }
                    record.ratio_limit = ratio;
                    if let Some(handle) = &handle {
                        if ratio == 0.0 && handle.stats().finished && !handle.is_paused() {
                            self.inner.pause_handle(cx, handle).await?;
                        }
                    }
                }
                "torrents/addTags" | "torrents/removeTags" => {
                    let mut tags: HashSet<String> = record
                        .tags
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .map(str::to_owned)
                        .collect();
                    for tag in required(request, "tags")?
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                    {
                        if request.path == "torrents/addTags" {
                            tags.insert(tag.into());
                        } else {
                            tags.remove(tag);
                        }
                    }
                    let mut tags: Vec<_> = tags.into_iter().collect();
                    tags.sort();
                    record.tags = tags.join(",");
                }
                _ => bail!("unsupported native torrent operation {}", request.path),
            }
            ledger.records[index] = record;
            if let Err(error) = self.inner.save(cx, &ledger).await {
                ledger.records[index] = old;
                let _ = self.inner.save(cx, &ledger).await;
                return Err(error).context("saving torrent intent; previous intent restored");
            }
            if request.path == "torrents/stop" {
                self.inner.cancel_restore(&hash, false);
                if let Some(handle) = &handle {
                    if matches!(handle.stats().state, TorrentStatsState::Initializing { .. }) {
                        // Even a saved-paused handle may be actively hashing.
                        self.inner.pause_handle(cx, handle).await?;
                    }
                }
            }
            if request.path == "torrents/start" && handle.is_none() {
                self.inner.spawn_restore(cx, ledger.records[index].clone())?;
            }
        }
        self.inner
            .reconcile_queue(cx, &mut ledger, &retry_errors)
            .await?;
        match start_result { Some(result) => json(&result), None => Ok("Ok.".into()) }
    }

    pub(super) async fn add(&self, cx: &Cx, request: &Request) -> Result<String> {
        // Serialize additions without holding the ledger while resolving magnets.
        // A duplicate add must observe the first add's durable record.
        let _addition = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
        if self.inner.network_blocked.load(Ordering::Acquire) {
            bail!("connect ProtonVPN before adding a download");
        }
        let upload_limit = request
            .arg("upLimit")
            .map(str::parse::<i64>)
            .transpose()?
            .unwrap_or_else(|| {
                if request
                    .arg("tags")
                    .unwrap_or("")
                    .split(',')
                    .any(|tag| tag.trim() == SEED_TAG)
                {
                    0
                } else {
                    NO_UPLOAD_LIMIT
                }
            });
        if upload_limit < 0 {
            bail!("upload limit cannot be negative");
        }
        let paused =
            request.arg("stopped") == Some("true") || request.arg("paused") == Some("true");
        let mut record = Record {
            hash: String::new(),
            name: String::new(),
            source: String::new(),
            output_folder: request
                .arg("savepath")
                .map(str::to_owned)
                .unwrap_or_else(|| self.inner.default_folder.to_string_lossy().into_owned()),
            only_files: None,
            upload_limit,
            ratio_limit: request
                .arg("ratioLimit")
                .map(str::parse::<f64>)
                .transpose()?
                .unwrap_or(if upload_limit == NO_UPLOAD_LIMIT {
                    0.0
                } else {
                    -2.0
                }),
            category: request.arg("category").unwrap_or("").into(),
            tags: request.arg("tags").unwrap_or("").into(),
            added_on: now(),
            completion_on: 0,
            paused,
            resume_after_migration: false,
        };
        if !matches!(record.ratio_limit, -2.0 | 0.0) {
            bail!("native share limits support unlimited (-2) or stop on completion (ratio 0)");
        }
        let sources: Vec<String> = if let Some(file) = &request.file {
            record.name = file.name.clone();
            vec![String::new()]
        } else {
            required(request, "urls")?
                .lines()
                .filter(|source| !source.trim().is_empty())
                .map(|source| source.trim().to_owned())
                .collect()
        };
        if sources.is_empty() {
            bail!("no torrent was supplied");
        }
        for source in sources {
            record.source = source.clone();
            let add = if let Some(file) = &request.file {
                AddTorrent::from_bytes(file.bytes.clone())
            } else if Path::new(&source).is_file() {
                AddTorrent::from_local_filename(&source)?
            } else {
                AddTorrent::from_url(source)
            };
            // Persist metadata and policy before allowing payload exchange.
            let mut options = record.options(true)?;
            // Let rqbit create a named subdirectory for new multi-file torrents.
            if request.arg("savepath").is_none() {
                options.output_folder = None;
            }
            let response = time::timeout(
                cx,
                Duration::from_secs(90),
                self.inner.session.add_torrent(cx, add, Some(options)),
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
        Ok("Ok.".into())
    }

}

pub(super) fn required<'a>(request: &'a Request, key: &str) -> Result<&'a str> {
    request
        .arg(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing {key}"))
}
fn requested_hashes(request: &Request, ledger: &Ledger) -> Result<Vec<String>> {
    let value = required(request, "hashes")?;
    if value == "all" {
        return Ok(ledger
            .records
            .iter()
            .map(|record| record.hash.clone())
            .collect());
    }
    let hashes: Vec<String> = value
        .split('|')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    for hash in &hashes {
        if !ledger.records.iter().any(|record| &record.hash == hash) {
            bail!("torrent not found: {hash}");
        }
    }
    Ok(hashes)
}

/// Validate the whole selection before changing intent. Bulk start omits done
/// records; an explicit selection refuses atomically with a useful next step.
pub(super) fn mutation_hashes(request: &Request, ledger: &Ledger) -> Result<(Vec<String>, Option<crate::qbt::StartResult>)> {
    let hashes = requested_hashes(request, ledger)?;
    if request.path != "torrents/start" { return Ok((hashes, None)); }
    let mut result = crate::qbt::StartResult::default();
    let bulk = request.arg("hashes") == Some("all") || request.arg("skip_complete") == Some("true");
    for hash in hashes {
        let record = ledger.records.iter().find(|r| r.hash == hash).expect("validated hash");
        // Legacy records can contain -1 for an unknown completion date.
        if record.completion_on > 0 && record.stop_on_complete() {
            if !bulk {
                return Err(crate::agent::AppError::new(crate::agent::Code::Failure,
                    "already complete; seeding is off for this torrent")
                    .hint(format!("fbittorrent seed {hash}")).into());
            }
            result.skipped.push(crate::qbt::StartSkipped { hash, reason: "complete_no_seed".into() });
        } else { result.started.push(hash); }
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
