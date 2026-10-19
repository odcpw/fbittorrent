//! Running-slot admission/removal around the daemon's short-lived undo cache.
use super::*;
use ops::{EngineResult, HashSelection};

impl EngineDispatcher {
    pub(crate) fn attach_undo(&self, history: Arc<undo::History>) {
        *self.inner.undo.lock().unwrap() = history;
    }
    pub(super) async fn remove_keep(
        &self,
        cx: &Cx,
        selection: &HashSelection,
    ) -> Result<EngineResult> {
        // Ordinary adds cannot race the old handle's cancellation/cleanup. The
        // caller's history transaction also serializes restore with this reply.
        let _additions = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
        let history = self.inner.undo.lock().unwrap().clone();
        let (records, checks) = {
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            let hashes = selection.resolve(&ledger)?;
            let records: Vec<_> = ledger
                .records
                .iter()
                .filter(|r| hashes.contains(&r.hash))
                .cloned()
                .collect();
            let directory = self.inner.directory.clone();
            let prepared = records.clone();
            task::spawn_blocking(cx, move || undo::prepare(&directory, &prepared)).await??;
            let replacement = Ledger {
                generation: ledger.generation.clone(),
                migrated_legacy: ledger.migrated_legacy,
                records: ledger
                    .records
                    .iter()
                    .filter(|r| !hashes.contains(&r.hash))
                    .cloned()
                    .collect(),
            };
            self.inner.save(cx, &replacement).await?;
            ledger.records = replacement.records;
            let checks = self.inner.rechecks.cancel_removed(&hashes);
            // Authorize after the durable removal even if handle teardown fails:
            // the removed record must remain recoverable after such a failure.
            history.authorize(&records);
            for hash in hashes {
                self.inner.cancel_restore(&hash, true);
                if let Ok(handle) = self.inner.handle(&hash) {
                    if let Err(error) = self
                        .inner
                        .session
                        .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
                        .await
                    {
                        self.lifecycle
                            .request_restart(crate::daemon::lifecycle::RestartReason::Explicit);
                        return Err(error);
                    }
                }
            }
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await?;
            (records, checks)
        };
        self.inner.rechecks.settle_removed(cx, checks).await?;
        for record in &records {
            self.inner.cancel_restore(&record.hash, true);
        }
        let directory = self.inner.directory.clone();
        let removed = records.clone();
        task::spawn_blocking(cx, move || undo::remove_originals(&directory, &removed)).await?;
        // Give the owner the full window after cleanup, one deadline per batch.
        Ok(EngineResult::RemovedUndo {
            undo_until: history.authorize(&records),
        })
    }
    pub(super) async fn restore_undo(&self, cx: &Cx, hash: &str) -> Result<EngineResult> {
        let _additions = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
        let history = self.inner.undo.lock().unwrap().clone();
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        if let Some(existing) = ledger.records.iter().find(|r| r.hash == hash) {
            return Ok(EngineResult::Added(vec![undo::added(existing, true)]));
        }
        let mut record = history.record(&self.inner.directory, hash)?;
        let directory = self.inner.directory.clone();
        let restore_hash = hash.to_owned();
        task::spawn_blocking(cx, move || {
            undo::restore_metadata(&directory, &restore_hash)
        })
        .await??;
        self.inner.invalidate_resume(hash)?;
        record.completion_on = 0;
        record.deferred = false;
        ledger.records.push(record.clone()); // Undo always appends to queue order.
        if let Err(error) = self.inner.save(cx, &ledger).await {
            ledger.records.pop();
            return Err(error);
        }
        history.restored(&self.inner.directory, hash);
        self.inner.spawn_restore(cx, record.clone())?;
        Ok(EngineResult::Added(vec![undo::added(&record, false)]))
    }
}
