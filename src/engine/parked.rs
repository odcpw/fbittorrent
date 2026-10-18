//! Typed saved-history and durable-intent operations without a native session.
use super::*;
use ops::{EngineOp, EngineResult, QueueInfo};
use view::TorrentView;

// Preserve cross-request serialization for the legacy compatibility function as
// well as owned parked ledgers. The daemon can retain one ParkedLedger handle.
static PARKED_LEDGER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parked_metadata(record: &Record, directory: &Path) -> Result<Option<Vec<u8>>> {
    let path = directory.join(format!("{}.torrent", record.hash));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let metadata = librqbit::torrent_from_bytes(&bytes)?;
    if metadata.info_hash.as_string() != record.hash {
        bail!("saved torrent metadata does not match {}", record.name);
    }
    metadata.info.data.validate()?;
    Ok(Some(bytes))
}
fn parked_view(record: &Record, directory: &Path) -> Result<TorrentView> {
    let metadata = if let Some(bytes) = parked_metadata(record, directory)? {
        let torrent = librqbit::torrent_from_bytes(&bytes)?;
        let info = torrent.info.data.validate()?;
        let files: Vec<_> = info.iter_file_details().collect();
        Some((
            info.lengths().total_length(),
            if files.len() == 1 {
                Some(Path::new(&record.output_folder).join(files[0].filename.to_pathbuf()))
            } else {
                None
            },
        ))
    } else {
        None
    };
    Ok(TorrentView::parked(record, metadata))
}

pub struct ParkedLedger {
    directory: PathBuf,
    lifecycle: Lifecycle,
    generation: Arc<LedgerGeneration>,
}
impl ParkedLedger {
    pub fn new(directory: PathBuf, lifecycle: Lifecycle) -> Self {
        Self {
            directory,
            lifecycle,
            generation: Arc::default(),
        }
    }
    pub fn ledger_generation(&self) -> Arc<LedgerGeneration> {
        self.generation.clone()
    }
    pub fn execute(&self, op: EngineOp) -> Result<EngineResult> {
        op.validate()?;
        let _guard = PARKED_LEDGER_LOCK
            .lock()
            .map_err(|_| anyhow!("saved torrent lock is unavailable"))?;
        let snapshot = self
            .lifecycle
            .expected_network()
            .unwrap_or_else(|| crate::network_app::state(&Config::default()));
        if let Some(result) = op.control(&snapshot, &self.lifecycle) {
            return result;
        }
        match &op {
            EngineOp::Version => {
                return Ok(EngineResult::Version(format!(
                    "native librqbit {} (waiting for ProtonVPN)",
                    librqbit::version()
                )));
            }
            EngineOp::TransferInfo => {
                return Ok(EngineResult::Transfer(TransferInfo {
                    connection_status: "vpn_waiting".into(),
                    ..Default::default()
                }));
            }
            EngineOp::SetQueue { limit } => {
                if *limit == 0 {
                    bail!("max_active_downloads must be at least 1");
                }
                let mut cfg = Config::load()?;
                cfg.max_active_downloads = *limit;
                cfg.save()?;
                return Ok(EngineResult::Acknowledged);
            }
            EngineOp::Queue
            | EngineOp::Info
            | EngineOp::Files { .. }
            | EngineOp::Start { .. }
            | EngineOp::Stop { .. }
            | EngineOp::Delete { .. } => {}
            _ => bail!(
                "connect ProtonVPN before changing downloads; saved downloads and settings remain available"
            ),
        }
        let mut ledger = read_ledger(&self.directory.join("torrents.json"))?;
        ledger.generation = self.generation.clone();
        match &op {
            EngineOp::Queue => {
                return Ok(EngineResult::Queue(QueueInfo {
                    max_active_downloads: Config::load()?.max_active_downloads,
                    active_downloads: 0,
                    queued_downloads: ledger
                        .records
                        .iter()
                        .filter(|record| {
                            !record.paused
                                && !(record.stop_on_complete() && record.completion_on > 0)
                        })
                        .count(),
                    network_blocked: true,
                }));
            }
            EngineOp::Info => {
                ledger.records.sort_by(|a, b| b.added_on.cmp(&a.added_on));
                return Ok(EngineResult::Torrents(
                    ledger
                        .records
                        .iter()
                        .map(|record| parked_view(record, &self.directory))
                        .collect::<Result<_>>()?,
                ));
            }
            EngineOp::Files { hash } => {
                let record = ledger
                    .records
                    .iter()
                    .find(|record| &record.hash == hash)
                    .context("torrent record not found")?;
                let bytes = parked_metadata(record, &self.directory)?.context(
                    "saved torrent metadata is unavailable; connect ProtonVPN to resolve it",
                )?;
                let torrent = librqbit::torrent_from_bytes(&bytes)?;
                let info = torrent.info.data.validate()?;
                return Ok(EngineResult::Files(
                    info.iter_file_details()
                        .enumerate()
                        .map(|(index, file)| TorrentFile {
                            index: index as i64,
                            name: file.filename.to_pathbuf().to_string_lossy().into(),
                            size: file.len as i64,
                            progress: 0.0,
                            progress_known: false,
                            priority: if record
                                .only_files
                                .as_ref()
                                .map(|ids| ids.contains(&index))
                                .unwrap_or(true)
                            {
                                1
                            } else {
                                0
                            },
                        })
                        .collect(),
                ));
            }
            _ => {}
        }
        let (hashes, start_result) = mutation_hashes(&op, &ledger)?;
        if let EngineOp::Delete { files, .. } = op {
            if files {
                bail!(
                    "connect ProtonVPN before removing downloaded files; removing the saved torrent while keeping files is available now"
                );
            }
            ledger
                .records
                .retain(|record| !hashes.contains(&record.hash));
            save_parked_ledger(&self.directory, &ledger)?;
            for hash in hashes {
                let _ = std::fs::remove_file(self.directory.join(format!("{hash}.torrent")));
            }
        } else {
            for record in &mut ledger.records {
                if hashes.contains(&record.hash) {
                    record.paused = matches!(op, EngineOp::Stop { .. });
                    record.resume_after_migration = false;
                }
            }
            save_parked_ledger(&self.directory, &ledger)?;
        }
        Ok(match start_result {
            Some(result) => EngineResult::Start(result),
            None => EngineResult::Acknowledged,
        })
    }
    pub fn dispatch(&self, request: &Request) -> Option<Result<String>> {
        match EngineOp::parse(&request.path, request) {
            Ok(Some(op)) => Some(self.execute(op).and_then(|result| {
                let legacy = result.to_legacy_json()?;
                match result {
                    // Parked lists used Vec<Value>, unlike the live lists of
                    // structs. Retain that wire ordering at this legacy facade.
                    EngineResult::Torrents(_) | EngineResult::Files(_) => {
                        json(&serde_json::from_str::<serde_json::Value>(&legacy)?)
                    }
                    _ => Ok(legacy),
                }
            })),
            Ok(None) if request.path.starts_with("torrents/") => Some(Err(anyhow!(
                "connect ProtonVPN before changing downloads; saved downloads and settings remain available"
            ))),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}
pub(super) fn dispatch_parked_in(
    request: &Request,
    directory: &Path,
    lifecycle: &Lifecycle,
) -> Option<Result<String>> {
    ParkedLedger::new(directory.into(), lifecycle.clone()).dispatch(request)
}
pub fn dispatch_parked(request: &Request, lifecycle: &Lifecycle) -> Option<Result<String>> {
    dispatch_parked_in(
        request,
        &crate::config::data_dir().join("engine"),
        lifecycle,
    )
}
