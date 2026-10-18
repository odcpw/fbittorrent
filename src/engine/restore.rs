//! Cached-first restore work, cancellation and handle policy.
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
