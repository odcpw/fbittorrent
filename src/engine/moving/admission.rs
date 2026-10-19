//! Admit dormant storage only for an explicit, already claimed move job.
use super::*;

impl Inner {
    pub(super) async fn move_handle(
        &self,
        cx: &Cx,
        record: &Record,
    ) -> Result<Arc<ManagedTorrent>> {
        if let Ok(handle) = self.handle(&record.hash) {
            return Ok(handle);
        }
        {
            let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
            let index = ledger
                .records
                .iter()
                .position(|r| r.hash == record.hash)
                .context("torrent record not found")?;
            let current = ledger.records[index].clone();
            self.activate_record(cx, &mut ledger, index, current)
                .await
                .map_err(|error| {
                    if error
                        .downcast_ref::<InvalidTorrentState>()
                        .is_some_and(|e| e.reason == "missing_files")
                    {
                        error.context(files::SourceProblem)
                    } else {
                        error
                    }
                })?;
        }
        let handle = self.handle(&record.hash)?;
        // Shutdown owns this job. Never wait for hashing under the ledger lock,
        // or let a cancelled move retain an unjoinable initialization waiter.
        rqbit_runtime::select! {
            result = handle.wait_until_initialized(cx) => result?,
            _ = self.moves.cancel.cancelled() => bail!("engine is closing during move initialization"),
        }
        if handle.stats().finished && record.completion_on > 0 {
            let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
            let saved = ledger
                .records
                .iter_mut()
                .find(|r| r.hash == record.hash)
                .context("torrent record not found")?;
            // Admission first invalidated completion for crash safety. A real
            // successful check restores the original completion history.
            saved.completion_on = record.completion_on;
            self.save(cx, &ledger).await?;
        }
        Ok(handle)
    }
}
