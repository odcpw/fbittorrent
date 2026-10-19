//! Short-lived removal authorization belongs to the daemon, across slot restarts.
//! Disk entries are a cache, never a persisted right to undo after daemon exit.
use super::ledger::validate_info_hash;
use super::*;
use ops::{AddedTorrent, EngineOp, EngineResult};
use std::{io::Write, os::unix::fs::DirBuilderExt, sync::Mutex as StdMutex, time::Instant};

const WINDOW: Duration = Duration::from_secs(10);
#[derive(Default)]
pub struct History {
    pub(super) transaction: StdMutex<()>,
    entries: StdMutex<HashMap<String, Instant>>,
}
impl History {
    pub fn start(directory: &Path) -> Result<Self> {
        // Called once by the daemon owner, never on an engine-slot restart.
        clean_directory(directory)?;
        Ok(Self::default())
    }
    pub fn expire(&self, directory: &Path) {
        let Ok(_transaction) = self.transaction.try_lock() else {
            return;
        };
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|hash, deadline| {
            if Instant::now() < *deadline {
                return true;
            }
            remove_entry(directory, hash);
            false
        });
    }
    pub(super) fn authorize(&self, records: &[Record]) -> u64 {
        let deadline = Instant::now() + WINDOW;
        let mut entries = self.entries.lock().unwrap();
        for record in records {
            entries.insert(record.hash.clone(), deadline);
        }
        now() as u64 + WINDOW.as_secs()
    }
    pub(super) fn restored(&self, directory: &Path, hash: &str) {
        self.entries.lock().unwrap().remove(hash);
        remove_entry(directory, hash);
    }
    pub(super) fn record(&self, directory: &Path, hash: &str) -> Result<Record> {
        validate_info_hash(hash)?;
        if self
            .entries
            .lock()
            .unwrap()
            .get(hash)
            .is_none_or(|deadline| Instant::now() >= *deadline)
        {
            return Err(InvalidTorrentState {
                hash: hash.into(),
                reason: "undo_expired",
            }
            .into());
        }
        let record: Record = serde_json::from_slice(&std::fs::read(
            directory.join("undo").join(format!("{hash}.json")),
        )?)?;
        anyhow::ensure!(record.hash == hash, "undo record hash mismatch");
        Ok(record)
    }
}

fn remove_entry(directory: &Path, hash: &str) {
    for suffix in ["json", "torrent", "json.tmp"] {
        let _ = std::fs::remove_file(directory.join("undo").join(format!("{hash}.{suffix}")));
    }
}
fn clean_directory(directory: &Path) -> Result<()> {
    let undo = directory.join("undo");
    let entries = match std::fs::read_dir(&undo) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some((hash, suffix)) = name.split_once('.') {
            if validate_info_hash(hash).is_ok() && matches!(suffix, "json" | "torrent" | "json.tmp")
            {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    std::fs::File::open(undo)?.sync_all()?;
    Ok(())
}
/// Prepare without removing the original metainfo. A crash before the ledger
/// commit must leave every old record restorable; hard links avoid large copies.
pub(super) fn prepare(directory: &Path, records: &[Record]) -> Result<()> {
    let undo = directory.join("undo");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&undo)?;
    std::fs::set_permissions(&undo, std::fs::Permissions::from_mode(0o700))?;
    let result = (|| {
        for record in records {
            validate_info_hash(&record.hash)?;
            remove_entry(directory, &record.hash);
            let path = undo.join(format!("{}.json", record.hash));
            let temporary = path.with_extension("json.tmp");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .custom_flags(libc::O_NOFOLLOW)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(record)?)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(temporary, path)?;
            let original = directory.join(format!("{}.torrent", record.hash));
            match std::fs::hard_link(&original, undo.join(format!("{}.torrent", record.hash))) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        std::fs::File::open(&undo)?.sync_all()?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        for record in records {
            remove_entry(directory, &record.hash);
        }
    }
    result
}
pub(super) fn remove_originals(directory: &Path, records: &[Record]) {
    for record in records {
        let _ = std::fs::remove_file(directory.join(format!("{}.torrent", record.hash)));
    }
}
pub(super) fn restore_metadata(directory: &Path, hash: &str) -> Result<()> {
    let source = directory.join("undo").join(format!("{hash}.torrent"));
    match std::fs::hard_link(&source, directory.join(format!("{hash}.torrent"))) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !source.exists() => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            use std::os::unix::fs::MetadataExt;
            let a = std::fs::metadata(&source)?;
            let b = std::fs::metadata(directory.join(format!("{hash}.torrent")))?;
            anyhow::ensure!(
                a.ino() == b.ino() && a.dev() == b.dev(),
                "conflicting restored metainfo"
            );
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
pub(super) fn parked(
    history: &History,
    directory: &Path,
    ledger: &mut Ledger,
    op: &EngineOp,
) -> Option<Result<EngineResult>> {
    if !matches!(
        op,
        EngineOp::RemoveKeep { .. } | EngineOp::RestoreUndo { .. }
    ) {
        return None;
    }
    Some((|| match op {
        EngineOp::RemoveKeep { hashes } => {
            let hashes = hashes.resolve(ledger)?;
            let records: Vec<_> = ledger
                .records
                .iter()
                .filter(|r| hashes.contains(&r.hash))
                .cloned()
                .collect();
            prepare(directory, &records)?;
            let previous = ledger.records.clone();
            ledger.records.retain(|r| !hashes.contains(&r.hash));
            if let Err(error) = save_parked_ledger(directory, ledger) {
                ledger.records = previous;
                return Err(error);
            }
            remove_originals(directory, &records);
            Ok(EngineResult::RemovedUndo {
                undo_until: history.authorize(&records),
            })
        }
        EngineOp::RestoreUndo { hash } => {
            if let Some(existing) = ledger.records.iter().find(|r| r.hash == *hash) {
                return Ok(EngineResult::Added(vec![added(existing, true)]));
            }
            let mut record = history.record(directory, hash)?;
            restore_metadata(directory, hash)?;
            record.completion_on = 0;
            record.deferred = true;
            ledger.records.push(record.clone());
            if let Err(error) = save_parked_ledger(directory, ledger) {
                ledger.records.pop();
                return Err(error);
            }
            history.restored(directory, hash);
            Ok(EngineResult::Added(vec![added(&record, false)]))
        }
        _ => unreachable!(),
    })())
}
pub(super) fn added(record: &Record, duplicate: bool) -> AddedTorrent {
    AddedTorrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        duplicate,
        deferred: record.deferred,
    }
}
#[cfg(test)]
mod tests;
