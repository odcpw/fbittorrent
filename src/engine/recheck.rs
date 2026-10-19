//! Explicit verification receipts, owned and drained by the replaceable engine.
use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex as StdMutex;

#[derive(Debug)]
pub struct MissingTorrent(pub String);
impl std::fmt::Display for MissingTorrent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "torrent record not found: {}", self.0)
    }
}
impl std::error::Error for MissingTorrent {}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub rechecked: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub have_bytes: BTreeMap<String, u64>,
}
#[derive(Debug, Default)]
pub struct Batch(StdMutex<Option<Report>>);
impl Batch {
    pub fn result(&self) -> Option<Report> {
        self.0.lock().unwrap().clone()
    }
}
#[derive(Default)]
struct Check {
    active: AtomicBool,
    handle: StdMutex<Option<Weak<ManagedTorrent>>>,
    result: StdMutex<Option<std::result::Result<u64, String>>>,
}
#[derive(Default)]
pub(super) struct Rechecks {
    checks: StdMutex<HashMap<String, Arc<Check>>>,
    serial: Arc<Mutex<()>>,
    pub(super) cancel: CancellationToken,
}
impl Rechecks {
    pub(super) fn active(&self, hash: &str) -> bool {
        self.checks
            .lock()
            .unwrap()
            .get(hash)
            .is_some_and(|c| c.active.load(Ordering::Acquire))
    }
}

impl Inner {
    pub(super) fn recheck_failed(&self, hash: &str) -> bool {
        self.restores
            .lock()
            .unwrap()
            .get(hash)
            .is_some_and(|restore| {
                matches!(
                    restore.outcome,
                    RestoreOutcome::Failed {
                        reason: "recheck_failed",
                        ..
                    }
                )
            })
    }

    pub(super) async fn begin_recheck(
        self: &Arc<Self>,
        cx: &Cx,
        hashes: Vec<String>,
    ) -> Result<Arc<Batch>> {
        anyhow::ensure!(
            (1..=256).contains(&hashes.len()),
            "hashes must contain 1..256 items"
        );
        let ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        let mut records = Vec::new();
        let mut seen = HashSet::new();
        for hash in hashes {
            if !seen.insert(hash.clone()) {
                continue;
            }
            let record = ledger
                .records
                .iter()
                .find(|r| r.hash == hash)
                .ok_or_else(|| MissingTorrent(hash.clone()))?;
            if record.staged {
                return Err(InvalidTorrentState {
                    hash,
                    reason: "staged",
                }
                .into());
            }
            // Metadata must already exist; recheck never starts a peer lookup.
            if self
                .handle(&hash)
                .ok()
                .is_none_or(|h| h.metadata.load_full().is_none())
                && parked::parked_metadata(record, &self.directory)?.is_none()
            {
                return Err(InvalidTorrentState {
                    hash,
                    reason: "metadata_unavailable",
                }
                .into());
            }
            records.push(record.hash.clone());
        }
        if self.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let batch = Arc::new(Batch::default());
        let mut work = Vec::new();
        {
            let mut checks = self.rechecks.checks.lock().unwrap();
            for hash in records {
                let fresh = !checks.contains_key(&hash);
                let check = checks.entry(hash.clone()).or_default().clone();
                work.push((hash, check, fresh));
            }
        }
        drop(ledger);
        let inner = self.clone();
        let output = batch.clone();
        let failed_to_spawn = work.clone();
        let task = task::try_spawn_with(cx, move |cx| async move {
            let mut report = Report::default();
            for (hash, check, fresh) in work {
                let result = if fresh {
                    let verified = rqbit_runtime::select! {
                        _ = inner.rechecks.cancel.cancelled() => Err(anyhow!("recheck cancelled: engine is closing")),
                        result = inner.verify_one(&cx, &hash, &check) => result,
                    }.map_err(|error| format!("{error:#}"));
                    if let Err(message) = &verified {
                        // Do not leave an old "finished" handle able to restore
                        // invalidated completion after verification failed.
                        let owned = check
                            .handle
                            .lock()
                            .unwrap()
                            .as_ref()
                            .and_then(Weak::upgrade);
                        if let Some(owned) = owned {
                            if let Ok(_ledger) =
                                OwnedMutexGuard::lock(inner.ledger.clone(), &cx).await
                            {
                                if inner
                                    .handle(&hash)
                                    .is_ok_and(|current| Arc::ptr_eq(&current, &owned))
                                {
                                    let _ = inner
                                        .session
                                        .delete(&cx, TorrentIdOrHash::Id(owned.id()), false)
                                        .await;
                                }
                            }
                        }
                        inner.set_restore_outcome(
                            &hash,
                            if message.contains("cancelled by stop") {
                                RestoreOutcome::Stopped
                            } else {
                                RestoreOutcome::Failed {
                                    reason: "recheck_failed",
                                    message: message.clone(),
                                }
                            },
                        );
                    }
                    check.active.store(false, Ordering::Release);
                    *check.result.lock().unwrap() = Some(verified.clone());
                    inner.rechecks.checks.lock().unwrap().remove(&hash);
                    // Reconciliation is after verification and outside its set.
                    // Preserve any stop/remove intent saved while I/O was running.
                    if !inner.stopping.load(Ordering::Acquire) {
                        if let Err(error) = inner.maintain(&cx).await {
                            eprintln!("WARN engine recheck queue reconciliation error={error:?}");
                        }
                    }
                    verified
                } else {
                    loop {
                        if let Some(result) = check.result.lock().unwrap().clone() {
                            break result;
                        }
                        if inner.rechecks.cancel.is_cancelled() {
                            break Err("recheck cancelled: engine is closing".into());
                        }
                        time::sleep(&cx, Duration::from_millis(20)).await;
                    }
                };
                match result {
                    Ok(bytes) => {
                        report.have_bytes.insert(hash.clone(), bytes);
                        report.rechecked.push(hash);
                    }
                    Err(message) => report.failed.push((hash, message)),
                }
            }
            *output.0.lock().unwrap() = Some(report);
        });
        match task {
            Ok(task) => {
                // The owner joins these beside restore work before stopping the
                // session/runtime. No detached worker can outlive its slot.
                let mut tasks = self.retired_restores.lock().unwrap();
                tasks.retain(|task| !task.is_finished());
                tasks.push(task);
            }
            Err(error) => {
                let mut checks = self.rechecks.checks.lock().unwrap();
                for (hash, check, fresh) in failed_to_spawn {
                    if fresh {
                        *check.result.lock().unwrap() = Some(Err("could not spawn recheck".into()));
                        checks.remove(&hash);
                    }
                }
                return Err(error.into());
            }
        }
        Ok(batch)
    }

    async fn verify_one(&self, cx: &Cx, hash: &str, check: &Check) -> Result<u64> {
        let _serial = OwnedMutexGuard::lock(self.rechecks.serial.clone(), cx).await?;
        let (handle, previous_completion) = {
            let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
            let record = ledger
                .records
                .iter_mut()
                .find(|r| r.hash == hash)
                .context("torrent was removed before recheck")?;
            // Invalidate saved completion before any data I/O, even if the
            // process dies during the check. This must never restore dormant.
            let previous_completion = record.completion_on;
            record.completion_on = 0;
            let record = record.clone();
            self.save(cx, &ledger).await?;
            self.cancel_restore(hash, true);
            self.set_restore_outcome(hash, RestoreOutcome::Checking);
            check.active.store(true, Ordering::Release);
            eprintln!("INFO engine recheck started hash={hash}");
            // Serialize only handle replacement with ledger mutations. It
            // returns an initializing handle; the scan and wait below never
            // hold the ledger. Seed/stop/remove cannot change policy in a gap
            // between our snapshot and admission or resurrect a removed row.
            if let Ok(handle) = self.handle(hash) {
                *check.handle.lock().unwrap() = Some(Arc::downgrade(&handle));
                self.rebuild(cx, &record, &record).await?;
            } else {
                self.restore_record(cx, &record, true).await?;
            }
            let handle = self.handle(hash)?;
            *check.handle.lock().unwrap() = Some(Arc::downgrade(&handle));
            (handle, previous_completion)
        };
        loop {
            if self.check_cancelled(hash, &handle) {
                bail!("recheck cancelled by stop");
            }
            match time::timeout(
                cx,
                Duration::from_millis(100),
                handle.wait_until_initialized(cx),
            )
            .await
            {
                Ok(result) => {
                    result?;
                    break;
                }
                Err(_) => cx.checkpoint()?,
            }
        }
        let stats = handle.stats();
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        let saved = ledger
            .records
            .iter_mut()
            .find(|r| r.hash == hash)
            .context("torrent was removed during recheck")?;
        apply_measurement(
            saved,
            stats.progress_bytes,
            stats.total_bytes,
            previous_completion,
        );
        self.save(cx, &ledger).await?;
        self.set_restore_outcome(hash, RestoreOutcome::Restored);
        eprintln!(
            "INFO engine recheck finished hash={hash} have_bytes={} size={}",
            stats.progress_bytes, stats.total_bytes
        );
        Ok(stats.progress_bytes)
    }
}

fn apply_measurement(record: &mut Record, have: u64, size: u64, previous_completion: i64) {
    record.completion_on = if have < size {
        0
    } else if previous_completion > 0 {
        previous_completion
    } else {
        now()
    };
}

#[cfg(test)]
mod tests;
