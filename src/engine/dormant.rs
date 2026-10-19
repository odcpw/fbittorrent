//! Ledger-only completed torrents and the explicit actions that admit them.
use super::*;
use ops::{EngineOp, EngineResult};

/// The API adapter can preserve invalid_state's structured reason without
/// guessing from diagnostic text. No user paths are included in this error.
#[derive(Debug)]
pub struct InvalidTorrentState {
    pub hash: String,
    pub reason: &'static str,
}
impl std::fmt::Display for InvalidTorrentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "torrent {}: {}", self.hash, self.reason)
    }
}
impl std::error::Error for InvalidTorrentState {}

pub(super) struct SavedTorrent {
    pub(super) files: Vec<TorrentFile>,
    pub(super) selected_bytes: u64,
    pub(super) selected_files_present: bool,
}

impl Inner {
    pub(super) fn set_restore_outcome(&self, hash: &str, outcome: RestoreOutcome) {
        let last_error = match &outcome {
            RestoreOutcome::Failed { message, .. } => Some(message.clone()),
            _ => None,
        };
        self.restores.lock().unwrap().insert(
            hash.into(),
            RestoreTask {
                generation: RESTORE_GENERATION.fetch_add(1, Ordering::Relaxed),
                outcome,
                last_error,
                task: None,
                cancel: CancellationToken::new(),
            },
        );
    }

    pub(super) fn saved_files(&self, record: &Record) -> Result<Vec<TorrentFile>> {
        Ok(self.saved_torrent(record)?.files)
    }

    pub(super) fn saved_torrent(&self, record: &Record) -> Result<SavedTorrent> {
        let bytes = parked::parked_metadata(record, &self.directory)?
            .context("saved torrent metadata is unavailable")?;
        let torrent = librqbit::torrent_from_bytes(&bytes)?;
        let info = torrent.info.data.validate()?;
        let lengths = info.lengths();
        let mut selected_end = 0;
        let mut selected_bytes = 0;
        let mut selected_files_present = true;
        let files = info
            .iter_file_details_ext()
            .enumerate()
            .map(|(index, file)| {
                let selected = record
                    .only_files
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&index));
                // Match librqbit's compute_selected_pieces / hns.total(): the
                // union of non-padding selected files' pieces, not file sizes.
                // Metainfo order gives increasing ranges; count shared boundary
                // pieces once and clip the final piece to its actual length.
                let start = file.pieces.start.max(selected_end);
                if selected && !file.details.attrs().padding && start < file.pieces.end {
                    let piece_length = u64::from(lengths.default_piece_length());
                    let end_bytes =
                        (u64::from(file.pieces.end) * piece_length).min(lengths.total_length());
                    selected_bytes += end_bytes - u64::from(start) * piece_length;
                    selected_end = file.pieces.end;
                }
                if record.dormant_on_restore() && selected && !file.details.attrs().padding {
                    selected_files_present &= std::fs::metadata(Path::new(&record.output_folder).join(file.details.filename.to_pathbuf()))
                        .is_ok_and(|m| m.is_file() && m.len() == file.details.len);
                }
                TorrentFile {
                    index: index as i64,
                    name: file.details.filename.to_pathbuf().to_string_lossy().into(),
                    size: file.details.len as i64,
                    progress: if record.completion_on > 0 { 1.0 } else { 0.0 },
                    // This is the saved completion claim, not a measurement of data.
                    progress_known: false,
                    priority: if selected { 1 } else { 0 },
                }
            })
            .collect();
        Ok(SavedTorrent {
            files,
            selected_bytes,
            selected_files_present,
        })
    }

    pub(super) fn dormant(&self, hash: &str) -> bool {
        self.restores
            .lock()
            .unwrap()
            .get(hash)
            .is_some_and(|restore| {
                matches!(
                    restore.outcome,
                    RestoreOutcome::Dormant
                        | RestoreOutcome::Failed {
                            reason: "missing_files",
                            ..
                        }
                )
            })
    }

    async fn missing_files(
        &self,
        cx: &Cx,
        ledger: &mut Ledger,
        index: usize,
    ) -> Result<anyhow::Error> {
        let hash = ledger.records[index].hash.clone();
        ledger.records[index].completion_on = 0;
        self.set_restore_outcome(
            &hash,
            RestoreOutcome::Failed {
                reason: "missing_files",
                message: format!(
                    "{} restore failed: missing files",
                    ledger.records[index].name
                ),
            },
        );
        self.save(cx, ledger).await?;
        Ok(InvalidTorrentState {
            hash,
            reason: "missing_files",
        }
        .into())
    }

    /// Explicit actions verify selected paths before admitting storage. Listing
    /// dormant records also checks presence, without admitting or changing them.
    async fn require_saved_data(&self, cx: &Cx, ledger: &mut Ledger, index: usize) -> Result<()> {
        let record = &ledger.records[index];
        for file in self
            .saved_files(record)?
            .into_iter()
            .filter(|file| file.priority != 0)
        {
            match rqbit_runtime::fs::metadata(Path::new(&record.output_folder).join(file.name))
                .await
            {
                Ok(metadata) if metadata.is_file() && metadata.len() == file.size.max(0) as u64 => {}
                Ok(_) => return Err(self.missing_files(cx, ledger, index).await?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(self.missing_files(cx, ledger, index).await?)
                }
                Err(error) => return Err(error).context("inspecting downloaded file"),
            }
        }
        Ok(())
    }

    pub(super) async fn activate_record(
        &self,
        cx: &Cx,
        ledger: &mut Ledger,
        index: usize,
        mut changed: Record,
    ) -> Result<()> {
        let hash = changed.hash.clone();
        // Seed activation already has running intent. Starting its check as
        // paused would make reconciliation unpause an initializing handle,
        // cancelling that run and potentially performing a second check.
        let dormant_seed = self.dormant(&hash) && !changed.stop_on_complete() && !changed.paused;
        if self.dormant(&hash) {
            self.require_saved_data(cx, ledger, index).await?;
        }
        let metadata = self.resolve_record(cx, &changed).await?;
        // Invalidate the old completion BEFORE opening data. A crash or a failed
        // check must never turn this torrent back into a dormant completed one.
        changed.completion_on = 0;
        let old = std::mem::replace(&mut ledger.records[index], changed);
        if let Err(error) = self.save(cx, ledger).await {
            ledger.records[index] = old;
            return Err(error);
        }
        self.cancel_restore(&hash, true);
        if let Ok(handle) = self.handle(&hash) {
            self.session
                .delete(cx, TorrentIdOrHash::Id(handle.id()), false)
                .await?;
        }
        self.set_restore_outcome(&hash, RestoreOutcome::Checking);
        match self
            .admit_record(cx, &ledger.records[index], !dormant_seed, metadata)
            .await
        {
            Ok(_) => {
                self.set_restore_outcome(&hash, RestoreOutcome::Restored);
                Ok(())
            }
            Err(error) => {
                self.set_restore_outcome(
                    &hash,
                    RestoreOutcome::Failed {
                        reason: "restore_failed",
                        message: format!("could not restore torrent: {error:#}"),
                    },
                );
                Err(error)
            }
        }
    }
}

impl EngineDispatcher {
    pub(super) async fn execute_dormant(
        &self,
        cx: &Cx,
        op: &EngineOp,
    ) -> Option<Result<EngineResult>> {
        if !matches!(
            op,
            EngineOp::OpenTarget { .. }
                | EngineOp::Recheck { .. }
                | EngineOp::SetFiles { .. }
                | EngineOp::SetSeed { .. }
        ) {
            return None;
        }
        Some(
            async {
                let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
                if let EngineOp::SetSeed { hashes, on } = op {
                    for hash in hashes.resolve(&ledger)? {
                        let index = ledger.records.iter().position(|r| r.hash == hash).unwrap();
                        if ledger.records[index].staged {return Err(InvalidTorrentState {hash:hash.clone(),reason:"staged"}.into());}
                        let mut changed = ledger.records[index].clone();
                        changed.upload = Some(*on);
                        changed.ratio_limit = if *on { -2.0 } else { 0.0 };
                        let mut tags: Vec<_> = changed
                            .tags
                            .split(',')
                            .filter(|tag| !tag.is_empty() && *tag != SEED_TAG)
                            .map(str::to_owned)
                            .collect();
                        if *on {
                            tags.push(SEED_TAG.into());
                        }
                        tags.sort();
                        tags.dedup();
                        changed.tags = tags.join(",");
                        if *on
                            && (ledger.records[index].completion_on > 0
                                || self.inner.dormant(&hash))
                        {
                            changed.paused = false;
                        }
                        let handle = self.inner.handle(&hash).ok();
                        if *on && handle.is_none() && self.inner.dormant(&hash) {
                            self.inner
                                .activate_record(cx, &mut ledger, index, changed)
                                .await?;
                        } else {
                            let old = std::mem::replace(&mut ledger.records[index], changed);
                            if let Err(error) = self.inner.save(cx, &ledger).await {
                                ledger.records[index] = old;
                                let _ = self.inner.save(cx, &ledger).await;
                                return Err(error)
                                    .context("saving seed intent; previous intent restored");
                            }
                        }
                        if let Ok(handle) = self.inner.handle(&hash) {
                            // Save first, then use the same live limiter setter
                            // for existing and newly admitted dormant handles.
                            // Queue reconciliation owns pause/resume; neither a
                            // cap change nor a seed toggle needs another check.
                            handle.set_upload_enabled(Some(*on));
                            let limit = ledger.records[index].upload_limit;
                            handle.set_upload_bps((limit != 0).then_some(limit as u64))?;
                        } else if *on {
                            // An unresolved magnet must retain cached-first
                            // startup's nonblocking metadata resolution.
                            self.inner
                                .spawn_restore(cx, ledger.records[index].clone())?;
                        }
                    }
                } else {
                    let hash = match op {
                        EngineOp::OpenTarget { hash }
                        | EngineOp::Recheck { hash }
                        | EngineOp::SetFiles { hash, .. } => hash,
                        _ => unreachable!(),
                    };
                    let index = ledger
                        .records
                        .iter()
                        .position(|r| &r.hash == hash)
                        .context("torrent record not found")?;
                    if ledger.records[index].staged {return Err(InvalidTorrentState {hash:hash.clone(),reason:"staged"}.into());}
                    let mut changed = ledger.records[index].clone();
                    if !matches!(op, EngineOp::OpenTarget { .. })
                        && self.inner.handle(hash).is_err()
                        && !self.inner.dormant(hash)
                    {
                        return Err(InvalidTorrentState {
                            hash: hash.clone(),
                            reason: "no_live_handle",
                        }
                        .into());
                    }
                    match op {
                        EngineOp::OpenTarget { .. } => {
                            let files = self.inner.saved_files(&changed)?;
                            let mut target = PathBuf::from(&changed.output_folder);
                            if files.len() == 1 {
                                target.push(&files[0].name);
                            }
                            match rqbit_runtime::fs::metadata(&target).await {
                                Ok(metadata)
                                    if (files.len() == 1 && metadata.is_file())
                                        || (files.len() != 1 && metadata.is_dir()) => {}
                                Ok(_) => {
                                    return Err(self
                                        .inner
                                        .missing_files(cx, &mut ledger, index)
                                        .await?)
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    return Err(self
                                        .inner
                                        .missing_files(cx, &mut ledger, index)
                                        .await?)
                                }
                                Err(error) => return Err(error).context("inspecting open target"),
                            }
                            return Ok(EngineResult::OpenTarget(target));
                        }
                        EngineOp::SetFiles { include, .. } => {
                            let files = self.inner.saved_files(&changed)?;
                            if include.iter().any(|index| *index >= files.len()) {
                                bail!("file index is out of range");
                            }
                            if include.is_empty() && self.inner.handle(hash).is_err() {
                                return Err(InvalidTorrentState {
                                    hash: hash.clone(),
                                    reason: "no_live_handle",
                                }
                                .into());
                            }
                            changed.only_files = Some(include.clone());
                        }
                        _ => {}
                    }
                    // Admission itself performs the recheck. Never schedule a
                    // second validation after restoring a dormant torrent.
                    self.inner
                        .activate_record(cx, &mut ledger, index, changed)
                        .await?;
                }
                self.inner
                    .reconcile_queue(cx, &mut ledger, &HashSet::new())
                    .await?;
                Ok(EngineResult::Acknowledged)
            }
            .await,
        )
    }
}
