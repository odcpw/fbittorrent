//! Typed saved-history and durable-intent operations without a native session.
use super::*;
use ops::{AddedTorrent, EngineOp, EngineResult, QueueInfo};
use view::TorrentView;

// Preserve cross-request serialization for the legacy compatibility function as
// well as owned parked ledgers. The daemon can retain one ParkedLedger handle.
static PARKED_LEDGER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn parked_metadata(record: &Record, directory: &Path) -> Result<Option<Vec<u8>>> {
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
    undo: Arc<undo::History>,
}
impl ParkedLedger {
    pub fn new(directory: PathBuf, lifecycle: Lifecycle) -> Self {
        Self {
            directory,
            lifecycle,
            generation: Arc::default(),
            undo: Arc::default(),
        }
    }
    pub fn with_undo(mut self, history: Arc<undo::History>) -> Self { self.undo=history; self }
    pub fn ledger_generation(&self) -> Arc<LedgerGeneration> {
        self.generation.clone()
    }
    pub fn execute(&self, op: EngineOp) -> Result<EngineResult> {
        op.validate()?;
        let _undo_guard = matches!(&op, EngineOp::RemoveKeep { .. } | EngineOp::RestoreUndo { .. })
            .then(|| self.undo.transaction.lock().unwrap());
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
            | EngineOp::Delete { .. } | EngineOp::ForceStart { .. } | EngineOp::QueueMove { .. } => {}
            // Typed API additions save intent without constructing a session.
            EngineOp::AddMagnet(_) | EngineOp::AddSelected { .. } | EngineOp::RemoveKeep { .. } | EngineOp::RestoreUndo { .. } => {}
            _ => bail!(
                "connect ProtonVPN before changing downloads; saved downloads and settings remain available"
            ),
        }
        let mut ledger = read_ledger(&self.directory.join("torrents.json"))?;
        ledger.generation = self.generation.clone();
        if let Some(result) = undo::parked(&self.undo, &self.directory, &mut ledger, &op) { return result; }
        if let EngineOp::AddMagnet(options) | EngineOp::AddSelected { options, .. } = &op {
            let cfg = Config::load()?;
            let folder = cfg.save_path.map(PathBuf::from).unwrap_or_else(|| {
                dirs::download_dir().unwrap_or_else(|| crate::config::data_dir().join("downloads"))
            });
            let record = match &op {
                EngineOp::AddSelected { files, .. } => Record::deferred_torrent(options, &folder, files.clone())?,
                _ => Record::deferred_magnet(options, &folder)?,
            };
            if let Some(existing) = ledger.records.iter().find(|r| r.hash == record.hash) {
                return Ok(EngineResult::Added(vec![AddedTorrent {
                    hash: existing.hash.clone(), name: existing.name.clone(), duplicate: true,
                    deferred: existing.deferred,
                }]));
            }
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.directory)?;
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))?;
            if let Some((_, bytes)) = &options.torrent {
                use std::io::Write;
                let temporary = self.directory.join(format!("{}.torrent.tmp", record.hash));
                let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true)
                    .custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&temporary)?;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                file.write_all(bytes)?; file.sync_all()?; drop(file);
                std::fs::rename(temporary, self.directory.join(format!("{}.torrent", record.hash)))?;
            }
            // Match live add's durable paused checkpoint before saving run intent.
            let mut checkpoint = record.clone(); checkpoint.paused = true;
            ledger.records.push(checkpoint);
            save_parked_ledger(&self.directory, &ledger)?;
            if !record.paused {
                ledger.records.last_mut().unwrap().paused = false;
                save_parked_ledger(&self.directory, &ledger)?;
            }
            return Ok(EngineResult::Added(vec![AddedTorrent {
                hash: record.hash, name: record.name, duplicate: false, deferred: true,
            }]));
        }
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
                let mut queue_position=0;
                let mut rows=ledger.records.iter().map(|record| {
                    let mut view=parked_view(record,&self.directory)?;
                    view.origin=Some(record.origin.clone());
                    view.queue_pos=if queue_policy::positioned(record) {queue_position+=1;Some(queue_position)} else {None};
                    Ok(view)
                }).collect::<Result<Vec<_>>>()?;
                rows.sort_by(|a,b|b.added_at.cmp(&a.added_at));
                return Ok(EngineResult::Torrents(rows));
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
        if let EngineOp::QueueMove {hash,to}=&op {
            let position=queue_policy::move_record(&mut ledger,hash,*to)?;
            save_parked_ledger(&self.directory,&ledger)?;
            return Ok(EngineResult::QueuePosition(position));
        }
        let (hashes, start_result) = mutation_hashes(&op, &ledger)?;
        if let EngineOp::ForceStart {on,..}=&op {queue_policy::validate_force(&ledger,&hashes,*on)?;}
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
                    match &op {
                        EngineOp::ForceStart {on,..}=>{record.force=*on;if *on {record.paused=false;}},
                        EngineOp::Stop {..}=>{record.paused=true;record.force=false;},
                        _=>record.paused=false,
                    }
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
