//! ENOSPC classification and an atomic retry that never overrides paused intent.
use super::*;

pub(crate) fn is_full(message: &str) -> bool {
    ["no space left on device", "(os error 28)", "enospc"]
        .iter()
        .any(|needle| {
            message
                .as_bytes()
                .windows(needle.len())
                .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
        })
}
impl EngineDispatcher {
    pub(crate) fn retry_disk_full(&self, hash: &str) -> Result<bool> {
        self.runtime.block_on(async {
            let cx = self.runtime.cx();
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            let Some(record) = ledger
                .records
                .iter()
                .find(|r| r.hash == hash && !r.paused)
                .cloned()
            else {
                return Ok(false);
            };
            let Ok(handle) = self.inner.handle(hash) else {
                return Ok(false);
            };
            let stats = handle.stats();
            if !matches!(stats.state, TorrentStatsState::Error)
                || !stats.error.as_deref().is_some_and(is_full)
            {
                return Ok(false);
            }
            self.inner.rebuild(cx, &record, &record).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::from([hash.to_owned()]))
                .await?;
            Ok(true)
        })
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn disk_error_classification_keeps_other_failures() {
        for message in ["write: No space left on device (os error 28)", "ENOSPC"] {
            assert!(super::is_full(message));
        }
        for message in [
            "Permission denied (os error 13)",
            "connection refused",
            "disk full.mkv: file missing",
        ] {
            assert!(!super::is_full(message));
        }
    }
}
