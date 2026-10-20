//! Staging owns metainfo and intent only. It never admits payload storage.
use super::*;
use ops::{AddedTorrent, EngineOp, EngineResult};

pub(super) fn execute(
    directory: &Path,
    folder: &Path,
    ledger: &mut Ledger,
    op: &EngineOp,
) -> Result<EngineResult> {
    match op {
        EngineOp::Stage { options, files } => {
            let mut record = if options.torrent.is_some() {
                Record::deferred_torrent(options, folder, files.clone().unwrap_or_default())?
            } else {
                Record::deferred_magnet(options, folder)?
            };
            if let Some(existing) = ledger.records.iter().find(|r| r.hash == record.hash) {
                return Ok(EngineResult::Added(vec![added(existing, true)]));
            }
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(directory)?;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            record.staged = true;
            record.paused = true;
            record.deferred = false;
            record.force = false;
            if let Some((_, bytes)) = &options.torrent {
                cache(directory, &record.hash, bytes)?;
            }
            ledger.records.push(record.clone());
            if let Err(error) = save_parked_ledger(directory, ledger) {
                // Keep the safe paused intent if rename succeeded before fsync.
                return Err(error);
            }
            Ok(EngineResult::Added(vec![added(&record, false)]))
        }
        EngineOp::Commit {
            hash,
            files,
            seed,
            save_path,
        } => {
            let index = staged_index(ledger, hash)?;
            let mut record = ledger.records[index].clone();
            if let Some(files) = files {
                let bytes = parked::parked_metadata(&record, directory)?.ok_or_else(|| {
                    InvalidTorrentState {
                        hash: hash.clone(),
                        reason: "metadata_pending",
                    }
                })?;
                let metadata = librqbit::torrent_from_bytes(&bytes)?;
                let count = metadata.info.data.validate()?.iter_file_details().count();
                anyhow::ensure!(
                    files.iter().all(|i| *i < count),
                    "file index is out of range"
                );
                record.only_files = Some(files.clone());
            }
            if let Some(seed) = seed {
                record.upload = (*seed).then_some(true);
            }
            if let Some(folder) = save_path {
                record.output_folder = folder.to_string_lossy().into();
            }
            record.options(true)?;
            record.staged = false;
            record.paused = false;
            record.deferred = true;
            let previous = std::mem::replace(&mut ledger.records[index], record);
            if let Err(error) = save_parked_ledger(directory, ledger) {
                ledger.records[index] = previous;
                let _ = save_parked_ledger(directory, ledger);
                return Err(error);
            }
            Ok(EngineResult::Acknowledged)
        }
        EngineOp::Discard { hash } => {
            let index = staged_index(ledger, hash)?;
            let record = ledger.records.remove(index);
            if let Err(error) = save_parked_ledger(directory, ledger) {
                ledger.records.insert(index, record);
                let _ = save_parked_ledger(directory, ledger);
                return Err(error);
            }
            remove_metadata(directory, hash)?;
            Ok(EngineResult::Acknowledged)
        }
        _ => bail!("not a staged operation"),
    }
}

fn staged_index(ledger: &Ledger, hash: &str) -> Result<usize> {
    let index = ledger
        .records
        .iter()
        .position(|r| r.hash == hash)
        .context("torrent record not found")?;
    if !ledger.records[index].staged {
        return Err(InvalidTorrentState {
            hash: hash.into(),
            reason: "not_staged",
        }
        .into());
    }
    Ok(index)
}
fn added(record: &Record, duplicate: bool) -> AddedTorrent {
    AddedTorrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        duplicate,
        deferred: record.deferred,
    }
}
fn remove_metadata(directory: &Path, hash: &str) -> Result<()> {
    match std::fs::remove_file(directory.join(format!("{hash}.torrent"))) {
        Ok(()) => std::fs::File::open(directory)?
            .sync_all()
            .context("syncing discarded metainfo"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("removing discarded metainfo"),
    }
}
fn cache(directory: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let temporary = directory.join(format!("{hash}.torrent.tmp"));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, directory.join(format!("{hash}.torrent")))?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

/// Once per daemon lifetime, under its instance lock and before listeners.
/// Engine-slot restarts retain a still-open add sheet's staged records.
pub(crate) fn discard_at_start(directory: &Path) -> Result<()> {
    let mut ledger = read_ledger(&directory.join("torrents.json"))?;
    let discarded: Vec<_> = ledger
        .records
        .iter()
        .filter(|r| r.staged)
        .map(|r| r.hash.clone())
        .collect();
    if discarded.is_empty() {
        return Ok(());
    }
    ledger.records.retain(|r| !r.staged);
    save_parked_ledger(directory, &ledger)?;
    for hash in discarded {
        remove_metadata(directory, &hash)?;
        eprintln!("info engine staged record discarded at start hash={hash}");
    }
    Ok(())
}

impl EngineDispatcher {
    pub(super) async fn staged_operation(&self, cx: &Cx, op: EngineOp) -> Result<EngineResult> {
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let directory = self.inner.directory.clone();
        let folder = self.inner.default_folder.clone();
        let saved_op = op.clone();
        let (ledger, result) = task::spawn_blocking(cx, move || {
            let result = execute(&directory, &folder, &mut ledger, &saved_op);
            (ledger, result)
        })
        .await?;
        let result = result?;
        // execute() saved through the parked-store path while retaining the
        // exclusive ledger. Publish that durable result for nonblocking reads,
        // just as Inner::save does for the normal engine mutations.
        *self.inner.committed_ledger.write().unwrap() = Arc::new(ledger.clone());
        match &op {
            EngineOp::Discard { hash } => self.inner.cancel_restore(hash, true),
            EngineOp::Commit { hash, .. } => {
                self.inner.cancel_restore(hash, true);
                let record = ledger
                    .records
                    .iter()
                    .find(|r| &r.hash == hash)
                    .unwrap()
                    .clone();
                self.inner.spawn_restore(cx, record)?;
            }
            EngineOp::Stage { .. } => {
                if let EngineResult::Added(rows) = &result {
                    for row in rows.iter().filter(|r| !r.duplicate) {
                        let record = ledger
                            .records
                            .iter()
                            .find(|r| r.hash == row.hash)
                            .unwrap()
                            .clone();
                        self.inner.spawn_restore(cx, record)?;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "tests/staging.rs"]
mod tests;
