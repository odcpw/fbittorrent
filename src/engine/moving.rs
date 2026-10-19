//! Owned, no-timeout payload jobs; verification survives re-admission.
use super::*;
use std::sync::Mutex as StdMutex;
mod files;
mod admission;
#[derive(Debug)]
pub(crate) struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("engine is busy with a payload move")
    }
}
impl std::error::Error for Busy {}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FailureKind {
    Source,
    Destination,
    Internal,
    Stopping,
}
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    pub hash: String,
    pub message: String,
    pub kind: FailureKind,
}
#[derive(Debug, Default)]
pub(crate) struct Batch(StdMutex<Option<std::result::Result<(), Failure>>>);
impl Batch {
    pub(crate) fn result(&self) -> Option<std::result::Result<(), Failure>> {
        self.0.lock().unwrap().clone()
    }
}
#[derive(Default)]
pub(super) struct Moves {
    pub(super) cancel: CancellationToken,
    active: StdMutex<HashSet<String>>,
    jobs: StdMutex<Vec<task::JoinHandle<()>>>,
}
struct MoveJoins<'a> {
    owner: &'a Moves,
    jobs: Vec<task::JoinHandle<()>>,
}
impl Drop for MoveJoins<'_> {
    fn drop(&mut self) {
        self.owner
            .jobs
            .lock()
            .unwrap()
            .extend(self.jobs.drain(..).filter(|job| !job.is_finished()));
    }
}
impl Moves {
    pub(super) fn any(&self) -> bool {
        !self.active.lock().unwrap().is_empty()
    }

    pub(super) fn active(&self, hash: &str) -> bool {
        self.active.lock().unwrap().contains(hash)
    }
    #[cfg(test)]
    pub(super) fn retained_jobs(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }
    pub(super) async fn drain(&self) -> Result<()> {
        let mut joins = MoveJoins {
            owner: self,
            jobs: std::mem::take(&mut *self.jobs.lock().unwrap()),
        };
        for job in &mut joins.jobs {
            job.await.context("joining payload move")?;
        }
        Ok(())
    }
}
struct Claim {
    inner: Arc<Inner>,
    hashes: Vec<String>,
}
impl Drop for Claim {
    fn drop(&mut self) {
        let mut active = self.inner.moves.active.lock().unwrap();
        for hash in &self.hashes {
            active.remove(hash);
        }
    }
}
impl Inner {
    pub(super) async fn begin_move(
        self: &Arc<Self>,
        cx: &Cx,
        mut hashes: Vec<String>,
        save_path: PathBuf,
    ) -> Result<Arc<Batch>> {
        anyhow::ensure!(
            (1..=256).contains(&hashes.len()),
            "hashes must contain 1..256 items"
        );
        anyhow::ensure!(
            save_path.is_absolute()
                && !save_path
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir)),
            "save_path must be absolute without parent traversal"
        );
        let mut seen = HashSet::new();
        hashes.retain(|hash| seen.insert(hash.clone()));
        let ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        for hash in &hashes {
            if self.moves.active(hash) {
                return Err(Busy.into());
            }
            ledger
                .records
                .iter()
                .find(|r| &r.hash == hash)
                .ok_or_else(|| recheck::MissingTorrent(hash.clone()))?;
            let handle = self
                .handle(hash)
                .ok()
                .filter(|handle| handle.have_pieces_snapshot().is_some());
            if (handle.is_none() && !self.dormant(hash)) || self.rechecks.active(hash) {
                return Err(InvalidTorrentState {
                    hash: hash.clone(),
                    reason: "not_initialized",
                }
                .into());
            }
        }
        self.moves
            .active
            .lock()
            .unwrap()
            .extend(hashes.iter().cloned());
        drop(ledger);
        let claim = Claim {
            inner: self.clone(),
            hashes: hashes.clone(),
        };
        let batch = Arc::new(Batch::default());
        let report = batch.clone();
        let inner = self.clone();
        let job = task::spawn_with(cx, move |cx| async move {
            let serial: Result<OwnedMutexGuard<()>> = rqbit_runtime::select! {
                result=OwnedMutexGuard::lock(inner.operations.clone(),&cx)=>result.map_err(anyhow::Error::from),
                _=inner.moves.cancel.cancelled()=>Err(anyhow!("engine is closing during move")),
            };
            let _serial = match serial {
                Ok(serial) => serial,
                Err(error) => {
                    drop(claim);
                    *report.0.lock().unwrap() = Some(Err(Failure {
                        hash: String::new(),
                        message: error.to_string(),
                        kind: FailureKind::Stopping,
                    }));
                    return;
                }
            };
            let mut result = Ok(());
            for hash in hashes {
                if let Err(error) = inner.move_one(&cx, &hash, &save_path).await {
                    let kind = if inner.stopping.load(Ordering::Acquire) {
                        FailureKind::Stopping
                    } else if error.downcast_ref::<files::SourceProblem>().is_some() {
                        FailureKind::Source
                    } else if error.downcast_ref::<files::DestinationProblem>().is_some() {
                        FailureKind::Destination
                    } else {
                        FailureKind::Internal
                    };
                    result = Err(Failure {
                        hash,
                        kind,
                        message: format!("{error:#}").chars().take(200).collect(),
                    });
                    break;
                }
            }
            drop(claim);
            // Restore the original durable intent, including completion policy.
            if let Ok(mut ledger) = OwnedMutexGuard::lock(inner.ledger.clone(), &cx).await {
                if let Err(error) = inner
                    .reconcile_queue(&cx, &mut ledger, &HashSet::new())
                    .await
                {
                    if result.is_ok() {
                        result = Err(Failure {
                            hash: String::new(),
                            kind: if inner.stopping.load(Ordering::Acquire) {
                                FailureKind::Stopping
                            } else {
                                FailureKind::Internal
                            },
                            message: error.to_string(),
                        });
                    }
                }
            }
            *report.0.lock().unwrap() = Some(result);
        });
        let mut jobs = self.moves.jobs.lock().unwrap();
        jobs.retain(|job| !job.is_finished());
        jobs.push(job);
        Ok(batch)
    }
    async fn move_one(self: &Arc<Self>, cx: &Cx, hash: &str, target: &Path) -> Result<()> {
        let record = {
            let ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
            ledger
                .records
                .iter()
                .find(|r| r.hash == hash)
                .cloned()
                .context("torrent record not found")?
        };
        if Path::new(&record.output_folder) == target {
            return Ok(());
        }
        let result = self.perform_move(cx, &record, target).await;
        if result.is_err() {
            let relocated = {
                let ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
                ledger
                    .records
                    .iter()
                    .find(|r| r.hash == hash)
                    .is_some_and(|r| Path::new(&r.output_folder) == target)
            };
            let source_failure = result
                .as_ref()
                .err()
                .is_some_and(|error| error.downcast_ref::<files::SourceProblem>().is_some());
            if relocated || source_failure {
                // A failure after durable relocation must not restart a handle
                // whose descriptors still refer to the old copied/unlinked data.
                if let Ok(handle) = self.handle(hash) {
                    let _ = self
                        .session
                        .delete(cx, TorrentIdOrHash::Hash(handle.info_hash()), false)
                        .await;
                }
                self.resume_prepared.remove(hash);
                if source_failure {
                    let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
                    let index = ledger
                        .records
                        .iter()
                        .position(|record| record.hash == hash)
                        .context("torrent record not found")?;
                    ledger.records[index].completion_on = 0;
                    self.save(cx, &ledger).await?;
                }
                self.set_restore_outcome(
                    hash,
                    RestoreOutcome::Failed {
                        reason: "move_failed",
                        message: format!(
                            "{} move failed: {}",
                            record.name,
                            result.as_ref().unwrap_err()
                        ),
                    },
                );
            }
        }
        result
    }
    async fn perform_move(self: &Arc<Self>, cx: &Cx, record: &Record, target: &Path) -> Result<()> {
        let hash = &record.hash;
        // Initialization is part of the owned background move, after claiming
        // the record and the mutation lock. The request still returns promptly.
        let handle = self.move_handle(cx, record).await?;
        if !handle.is_paused() {
            self.session.pause(cx, &handle).await?;
        }
        let bits = handle
            .have_pieces_snapshot()
            .context("torrent has no verified have snapshot")?
            .as_bytes()
            .to_vec();
        let metadata = handle
            .metadata
            .load_full()
            .context("torrent has no metadata")?;
        let required = metadata
            .file_infos
            .iter()
            .enumerate()
            .filter(|(index, file)| {
                !file.attrs.padding
                    && ((file.len > 0
                        && record
                            .only_files
                            .as_ref()
                            .is_none_or(|ids| ids.contains(index)))
                        || file
                            .piece_range
                            .clone()
                            .any(|piece| bits[piece as usize / 8] & (1 << (7 - piece % 8)) != 0))
            })
            .map(|(_, file)| file.relative_filename.clone())
            .collect::<HashSet<_>>();
        let piece_length = u64::from(metadata.lengths().default_piece_length());
        let lengths = metadata
            .file_infos
            .iter()
            .filter(|file| !file.attrs.padding)
            .map(|file| {
                let minimum = file
                    .piece_range
                    .clone()
                    .filter(|piece| bits[*piece as usize / 8] & (1 << (7 - *piece % 8)) != 0)
                    .map(|piece| {
                        ((u64::from(piece) + 1) * piece_length)
                            .min(metadata.lengths().total_length())
                            .saturating_sub(file.offset_in_torrent)
                            .min(file.len)
                    })
                    .max()
                    .unwrap_or(0);
                (file.relative_filename.clone(), (minimum, file.len))
            })
            .collect::<HashMap<_, _>>();
        let paths = metadata
            .file_infos
            .iter()
            .filter(|file| !file.attrs.padding)
            .map(|file| file.relative_filename.clone())
            .collect::<Vec<_>>();
        self.invalidate_resume(hash)?;
        let source = PathBuf::from(&record.output_folder);
        let destination = target.to_owned();
        let cancel = self.clone();
        let prepared = task::spawn_blocking(cx, move || {
            files::Prepared::cancellable(
                &source,
                &destination,
                &paths,
                &required,
                &lengths,
                &cancel.stopping,
            )
        })
        .await??;
        let mut ledger = OwnedMutexGuard::lock(self.ledger.clone(), cx).await?;
        let index = ledger
            .records
            .iter()
            .position(|r| r.hash.as_str() == hash.as_str())
            .context("torrent record not found")?;
        let original = ledger.records[index].output_folder.clone();
        ledger.records[index].output_folder = target.to_string_lossy().into_owned();
        let updated = ledger.records[index].clone();
        let directory = self.directory.clone();
        let committing = updated.clone();
        task::spawn_blocking(cx, move || -> Result<()> {
            let updated = committing;
            // The ledger guard and payload transaction share one synchronous
            // commit, so cancellation cannot split their ownership boundary.
            if let Err(error) = ledger::save_parked_ledger(&directory, &ledger) {
                let saved = read_ledger(&directory.join("torrents.json"))
                    .ok()
                    .and_then(|saved| {
                        saved
                            .records
                            .into_iter()
                            .find(|record| record.hash == updated.hash)
                            .map(|record| record.output_folder)
                    });
                if saved.as_deref() == Some(original.as_str()) {
                    // The write failed before replacement. The on-disk record
                    // is still authoritative: roll the payload back as well.
                    ledger.records[index].output_folder = original;
                    return Err(error);
                }
                if saved.as_deref() == Some(updated.output_folder.as_str()) {
                    // Replacement happened but its fsync failed. Keep the
                    // published data at the location the saved record names.
                    prepared.retain();
                    return Err(error);
                }
                ledger.records[index].output_folder = original;
                if let Err(rollback) = ledger::save_parked_ledger(&directory, &ledger) {
                    ledger.records[index].output_folder = updated.output_folder.clone();
                    prepared.retain();
                    bail!("saving move failed: {error:#}; restoring ledger failed: {rollback:#}");
                }
                return Err(error);
            }
            prepared.commit()
        })
        .await??;
        self.cancel_restore(hash, true);
        self.resume_prepared
            .install(&updated, &metadata.torrent_bytes, bits)?;
        self.set_restore_outcome(hash, RestoreOutcome::Checking);
        self.session
            .delete(cx, TorrentIdOrHash::Hash(handle.info_hash()), false)
            .await?;
        let mut options = updated.options(true)?;
        options.trusted_have_pieces = self.resume_prepared.take(hash);
        options.tracker_stats = Some(handle.shared().tracker_stats.clone());
        let admitted = self
            .session
            .add_torrent(
                cx,
                AddTorrent::from_bytes(metadata.torrent_bytes.to_vec()),
                Some(options),
            )
            .await?
            .into_handle()
            .context("move admission returned no handle")?;
        admitted.wait_until_initialized(cx).await?;
        self.resume_prepared.finish(hash);
        self.set_restore_outcome(hash, RestoreOutcome::Restored);
        let store = self.resume.clone();
        task::spawn_blocking(cx, move || store.snapshot(&updated, &admitted, false)).await??;
        Ok(())
    }
}
