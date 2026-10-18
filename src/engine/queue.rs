//! FIFO queue admission, live limits and network suspension policy.
use super::*;

impl Inner {
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
        let mut slots = 0;
        let mut decisions = Vec::new();
        let mut changed = false;
        for record in &mut ledger.records {
            cx.checkpoint()?;
            let Ok(handle) = self.handle(&record.hash) else {
                continue;
            };
            let stats = handle.stats();
            // Initializing.paused is transfer intent, not check activity.
            // Unpause closes the current run, so leave active verification
            // alone. Recompute post-check policy on the next reconciliation;
            // explicitly cancelled checks may still resume.
            let active_check = matches!(stats.state, TorrentStatsState::Initializing { .. })
                && !self.check_cancelled(&record.hash, &handle);
            if stats.finished && record.completion_on <= 0 {
                record.completion_on = now();
                changed = true;
            }
            let retry = retry_errors.contains(&record.hash);
            let running = if blocked || record.paused {
                false
            } else if stats.finished {
                !record.stop_on_complete()
            } else if matches!(stats.state, TorrentStatsState::Error) && !retry {
                false
            } else if slots < limit {
                slots += 1;
                true
            } else {
                false
            };
            decisions.push((handle, running, active_check));
        }
        if changed {
            self.save(cx, ledger).await?;
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
        self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await
    }
    pub(super) async fn set_download_limit(&self, cx: &Cx, limit: usize) -> Result<()> {
        if limit == 0 {
            bail!("max_active_downloads must be at least 1");
        }
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        let old = self.max_active_downloads.load(Ordering::Acquire);
        let mut cfg = Config::load()?;
        let saved = cfg.max_active_downloads;
        let previous = cfg.clone();
        cfg.max_active_downloads = limit;
        if let Err(error) = cfg.save() {
            // A directory fsync can fail after rename. Restore the previous
            // setting even when the staged writer reported a failed save.
            if let Err(rollback) = previous.save() {
                bail!("saving download limit failed: {error:#}; restoring previous settings failed: {rollback:#}");
            }
            return Err(error).context("saving download limit; previous settings restored");
        }
        self.max_active_downloads.store(limit, Ordering::Release);
        if let Err(error) = self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await {
            self.max_active_downloads.store(old, Ordering::Release);
            cfg.max_active_downloads = saved;
            let persisted = cfg.save();
            let resumed = self.reconcile_queue(cx, &mut ledger, &HashSet::new()).await;
            if let Err(rollback) = persisted.and(resumed) {
                bail!("changing download limit failed: {error:#}; restoring previous limit failed: {rollback:#}");
            }
            return Err(error).context("changing download limit; previous limit restored");
        }
        Ok(())
    }
}

impl EngineDispatcher {
    /// Persist and apply a live download queue limit through the app runtime.
    #[cfg(test)]
    pub fn set_download_limit(&self, limit: usize) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        self.runtime
            .block_on(self.inner.set_download_limit(self.runtime.cx(), limit))
    }

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
