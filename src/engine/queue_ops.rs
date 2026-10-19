//! Queue order mutation through the engine's existing durable store.
use super::*;
use ops::EngineResult;
impl EngineDispatcher {
    pub(super) async fn move_queue(
        &self,
        cx: &Cx,
        hash: &str,
        to: queue_policy::Direction,
    ) -> Result<EngineResult> {
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        if !ledger.records.iter().any(|r| r.hash == hash) {
            return Err(super::recheck::MissingTorrent(hash.into()).into());
        }
        if self.inner.handle(hash).is_err() {
            return Err(InvalidTorrentState {
                hash: hash.into(),
                reason: "no_live_handle",
            }
            .into());
        }
        let previous = ledger.records.clone();
        let position = queue_policy::move_record(&mut ledger, hash, to)?;
        if let Err(error) = self.inner.save(cx, &ledger).await {
            ledger.records = previous;
            return Err(error);
        }
        let mut pins = self.inner.queue_pins.lock().unwrap();
        for record in &ledger.records {
            if record.paused || record.force {
                continue;
            }
            if let Ok(handle) = self.inner.handle(&record.hash) {
                let stats = handle.stats();
                if !handle.is_paused()
                    && !stats.finished
                    && !matches!(stats.state, TorrentStatsState::Error)
                {
                    pins.insert(record.hash.clone());
                }
            }
        }
        drop(pins);
        self.inner
            .reconcile_queue(cx, &mut ledger, &HashSet::new())
            .await?;
        Ok(EngineResult::QueuePosition(position))
    }
}
