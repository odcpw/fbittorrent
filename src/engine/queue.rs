//! FIFO queue admission, live limits and network suspension policy.
use super::*;
pub(super) use super::queue_policy::admit;
static INJECTED_ERROR: AtomicBool = AtomicBool::new(false);

impl Inner {
    /// Return whether the background maintenance task should keep running.
    pub(super) async fn maintenance_tick(&self, cx: &Cx) -> bool {
        let result = self.maintain(cx).await;
        // Quiesce can begin while maintenance waits for the ledger. Retire
        // that tick quietly instead of reporting its expected closing error.
        if self.stopping.load(Ordering::Acquire) || cx.checkpoint().is_err() {
            return false;
        }
        if let Err(error) = result {
            eprintln!("ERROR engine queue maintenance failed message={:?}", format!("{error:#}"));
        }
        true
    }

    /// Apply durable running intent in FIFO order. Pause displaced torrents
    /// before admitting replacements, including checking downloads in the cap.
    pub(super) async fn reconcile_queue(
        &self,
        cx: &Cx,
        ledger: &mut Ledger,
        retry_errors: &HashSet<String>,
    ) -> Result<()> {
        cx.checkpoint()?;
        if self.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let limit = self.max_active_downloads.load(Ordering::Acquire);
        let blocked = self.network_blocked.load(Ordering::Acquire);
        let mut snapshots = Vec::with_capacity(ledger.records.len());
        let mut handles = Vec::with_capacity(ledger.records.len());
        let pins = self.queue_pins.lock().unwrap().clone();
        let mut changed = false;
        let mut completed = Vec::new();
        for record in &mut ledger.records {
            cx.checkpoint()?;
            let handle = self.handle(&record.hash).ok();
            let snapshot = handle.as_ref().map(|handle| {
                let stats = handle.stats();
                let rechecking = self.rechecks.active(&record.hash) || self.moves.active(&record.hash);
                let failed = self.recheck_failed(&record.hash);
                if stats.finished && !rechecking && !failed && !matches!(stats.state,TorrentStatsState::Error) {
                    if record.completion_on <= 0 {
                        record.completion_on = now();
                        let target = handle.metadata.load_full().filter(|m| m.file_infos.len() == 1)
                            .map(|m| handle.output_folder().join(&m.file_infos[0].relative_filename))
                            .unwrap_or_else(|| handle.output_folder().to_path_buf());
                        completed.push(serde_json::json!({"ev":"torrent.completed", "hash":record.hash,
                            "name":handle.name().unwrap_or_else(|| record.name.clone()), "open_target":target.to_string_lossy()}));
                        changed = true;
                    }
                    if record.force { record.force=false;changed=true; }
                }
                let active_check = matches!(stats.state, TorrentStatsState::Initializing { .. })
                    && !self.check_cancelled(&record.hash, handle);
                (queue_policy::Stats { paused:handle.is_paused(),finished:stats.finished,
                    error:failed || matches!(stats.state,TorrentStatsState::Error),
                    retry:retry_errors.contains(&record.hash),rechecking,pinned:pins.contains(&record.hash) },active_check)
            });
            snapshots.push(snapshot.map(|(stats,_)|stats));
            handles.push(handle.map(|handle|(handle,snapshot.unwrap().1)));
        }
        let admission = admit(&ledger.records,&snapshots,limit);
        let mut decisions=Vec::new();
        let mut retained=HashSet::new();
        for (i,handle) in handles.into_iter().enumerate() {
            let Some((handle,active_check))=handle else {continue;};
            if snapshots[i].unwrap().rechecking {continue;}
            let running=admission[i] && !blocked;
            if running && !ledger.records[i].force && !snapshots[i].unwrap().finished && pins.contains(&ledger.records[i].hash) {
                retained.insert(ledger.records[i].hash.clone());
            }
            decisions.push((handle,running,active_check));
        }
        *self.queue_pins.lock().unwrap()=retained;
        if changed {
            self.save(cx, ledger).await?;
            self.completed_count.fetch_add(completed.len() as u64,Ordering::AcqRel);
            let mut events = self.completions.lock().unwrap();
            for event in completed {
                if events.len() == 1024 { eprintln!("WARN engine completion queue full capacity=1024"); events.pop_front(); }
                events.push_back(event);
            }
            drop(events);
            self.bus_changed.notify_waiters();
        }
        // Stop first: reducing the limit or restarting an earlier queue entry
        // must never create a transient extra payload download.
        for (handle, running, _) in &decisions {
            cx.checkpoint()?;
            if !running
                && !handle.is_paused()
                && !matches!(handle.stats().state, TorrentStatsState::Error)
            {
                self.pause_handle(cx, handle).await?;
            }
        }
        for (handle, running, active_check) in decisions {
            cx.checkpoint()?;
            if running
                && !active_check
                && (handle.is_paused() || matches!(handle.stats().state, TorrentStatsState::Error))
            {
                self.unpause_handle(cx, &handle).await?;
            }
        }
        Ok(())
    }
    pub(super) async fn maintain(&self, cx: &Cx) -> Result<()> {
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        if !INJECTED_ERROR.load(Ordering::Acquire) {
            if let Ok(hash) = std::env::var("FBT_TEST_INJECT_ERROR") {
                if ledger.records.iter().any(|record| record.hash == hash)
                    && !INJECTED_ERROR.swap(true, Ordering::AcqRel) {
                    bail!("injected engine error hash={hash}");
                }
            }
        }
        self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await
    }
}

impl EngineDispatcher {
    /// Suspend or admit network work without changing durable torrent intent.
    #[cfg(test)]
    pub fn set_network_blocked(&self, blocked: bool) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        self.runtime.block_on(async {
            let mut ledger =
                OwnedMutexGuard::lock(self.inner.ledger.clone(), self.runtime.cx()).await?;
            self.inner.network_blocked.store(blocked, Ordering::Release);
            self.inner
                .reconcile_queue(self.runtime.cx(), &mut ledger, &HashSet::new())
                .await
        })
    }
}

#[cfg(test)]
#[path="tests/queue_policy.rs"]
mod tests;
