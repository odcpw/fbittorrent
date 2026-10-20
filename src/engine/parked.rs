//! Typed saved-history and durable-intent operations without a native session.
use super::*;
use ops::{AddedTorrent, EngineOp, EngineResult};
use view::TorrentView;

// Serialize saved-record changes across owned parked ledgers.
static PARKED_LEDGER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A transfer action that needs the network the owner is waiting for. The
/// API maps it to engine_unavailable(state=parked) whatever the interface.
#[derive(Debug)]
pub struct ParkedRefusal(pub String);
impl std::fmt::Display for ParkedRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for ParkedRefusal {}

/// The configured network, by the name the user chose for it.
pub(super) fn waiting_for(network: &crate::network_app::NetworkState) -> String {
    match (network.selection.as_deref(), network.interface.as_deref()) {
        (Some("proton"), _) => "ProtonVPN".into(),
        (_, Some(name)) => name.into(),
        (Some(name), None) => name.into(),
        (None, None) => "the configured network".into(),
    }
}

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
fn parked_view(record: &Record, directory: &Path, saved: &saved::SavedCache) -> Result<TorrentView> {
    let metadata = saved.info(record, directory)?.map(|info| {
        (
            info.total_length,
            (info.files.len() == 1).then(|| Path::new(&record.output_folder).join(&info.files[0].path)),
        )
    });
    Ok(TorrentView::parked(record, metadata))
}

pub struct ParkedLedger {
    directory: PathBuf,
    generation: Arc<LedgerGeneration>,
    undo: Arc<undo::History>,
    /// Saved records by torrents.json identity, and parsed metainfo: the
    /// sampler's parked Info reads no file whose identity is unchanged.
    records: std::sync::Mutex<Option<(saved::Stamp, Arc<Vec<Record>>)>>,
    saved: saved::SavedCache,
    network: Option<crate::network_app::NetworkState>,
}
impl ParkedLedger {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            generation: Arc::default(),
            undo: Arc::default(),
            records: Default::default(),
            saved: Default::default(),
            network: None,
        }
    }
    pub fn with_network(mut self, network: crate::network_app::NetworkState) -> Self {
        self.network = Some(network);
        self
    }
    pub fn with_undo(mut self, history: Arc<undo::History>) -> Self { self.undo=history; self }
    pub fn ledger_generation(&self) -> Arc<LedgerGeneration> {
        self.generation.clone()
    }
    /// Read-only records, reparsed only after torrents.json is replaced.
    fn saved_records(&self) -> Result<Arc<Vec<Record>>> {
        let path = self.directory.join("torrents.json");
        let Some(current) = saved::file_stamp(&path) else {
            return Ok(Arc::new(read_ledger(&path)?.records));
        };
        let mut cached = self.records.lock().unwrap();
        if let Some((seen, records)) = cached.as_ref() {
            if *seen == current { return Ok(records.clone()); }
        }
        let records = Arc::new(read_ledger(&path)?.records);
        // A replacement that crossed this read is observed on the next call.
        if saved::file_stamp(&path) == Some(current) { *cached = Some((current, records.clone())); }
        Ok(records)
    }
    /// Caps, upload permission and file selection change only the saved
    /// record while parked; the engine applies them when it admits the
    /// record, exactly as it applies every other saved option. None: not a
    /// record edit.
    fn edit_record(&self, ledger: &mut Ledger, op: &EngineOp, network: &str) -> Option<Result<()>> {
        if !matches!(op, EngineOp::SetUploadLimit { .. } | EngineOp::SetDownloadLimit { .. }
            | EngineOp::SetSeed { .. } | EngineOp::SetFiles { .. }) {
            return None;
        }
        Some((|| {
            if let EngineOp::SetFiles { hash, include } = op {
                let index = ledger.records.iter().position(|r| &r.hash == hash).context("torrent record not found")?;
                if ledger.records[index].staged {
                    return Err(InvalidTorrentState { hash: hash.clone(), reason: "staged" }.into());
                }
                let info = self.saved.info(&ledger.records[index], &self.directory)?.ok_or_else(|| ParkedRefusal(format!(
                    "saved torrent metadata is unavailable; connect {network} to resolve it"
                )))?;
                if include.iter().any(|index| *index >= info.files.len()) {
                    bail!("file index is out of range");
                }
                // An empty selection pauses a live handle; there is none here.
                if include.is_empty() {
                    return Err(InvalidTorrentState { hash: hash.clone(), reason: "no_live_handle" }.into());
                }
                let record = &mut ledger.records[index];
                record.only_files = Some(include.clone());
                // As a live selection change: the next admission checks the
                // new selection instead of trusting the old completion.
                record.completion_on = 0;
                return Ok(());
            }
            let hashes = match op {
                EngineOp::SetSeed { hashes, .. } => hashes.resolve(ledger)?,
                _ => mutation_hashes(op, ledger)?.0,
            };
            for hash in hashes {
                let record = ledger.records.iter_mut().find(|r| r.hash == hash).context("torrent record not found")?;
                match op {
                    EngineOp::SetUploadLimit { limit, .. } => {
                        record.upload_limit = *limit;
                        record.options(record.paused)?;
                    }
                    EngineOp::SetDownloadLimit { bps, .. } => {
                        record.download_limit = *bps;
                        record.options(record.paused)?;
                    }
                    EngineOp::SetSeed { on, .. } => {
                        if record.staged {
                            return Err(InvalidTorrentState { hash, reason: "staged" }.into());
                        }
                        // The same saved fields a live SetSeed writes.
                        record.upload = Some(*on);
                        if *on && record.completion_on > 0 {
                            record.paused = false;
                        }
                    }
                    _ => unreachable!("record edit"),
                }
            }
            Ok(())
        })())
    }
    pub fn execute(&self, op: EngineOp) -> Result<EngineResult> {
        op.validate()?;
        let _undo_guard = matches!(&op, EngineOp::RemoveKeep { .. } | EngineOp::RestoreUndo { .. })
            .then(|| self.undo.transaction.lock().unwrap());
        let _guard = PARKED_LEDGER_LOCK
            .lock()
            .map_err(|_| anyhow!("saved torrent lock is unavailable"))?;
        let network = self.network.as_ref().map(waiting_for)
            .unwrap_or_else(|| "the configured network".into());
        match &op {
            EngineOp::TransferInfo => {
                return Ok(EngineResult::Transfer(TransferInfo {
                    connection_status: "vpn_waiting".into(),
                    ..Default::default()
                }));
            }
            EngineOp::Rename {..}
            | EngineOp::Info
            | EngineOp::Files { .. }
            | EngineOp::Trackers { .. }
            | EngineOp::Peers { .. }
            | EngineOp::Start { .. }
            | EngineOp::Stop { .. }
            | EngineOp::Delete { .. } | EngineOp::ForceStart { .. } | EngineOp::QueueMove { .. } => {}
            // Typed API additions save intent without constructing a session.
            EngineOp::Stage {..} | EngineOp::Commit {..} | EngineOp::Discard {..} | EngineOp::AddMagnet(_) | EngineOp::AddSelected { .. } | EngineOp::RemoveKeep { .. } | EngineOp::RestoreUndo { .. } => {}
            // Record edits are durable now and applied by the next engine.
            EngineOp::SetUploadLimit { .. } | EngineOp::SetDownloadLimit { .. }
            | EngineOp::SetSeed { .. } | EngineOp::SetFiles { .. } => {}
            _ => return Err(ParkedRefusal(format!(
                "connect {network} before changing downloads; saved downloads and settings remain available"
            )).into()),
        }
        if matches!(&op, EngineOp::Info) {
            let records = self.saved_records()?;
            let mut queue_position=0;
            let mut rows=records.iter().map(|record| {
                let mut view=parked_view(record,&self.directory,&self.saved)?;
                view.origin=Some(record.origin.clone());
                view.queue_pos=if queue_policy::positioned(record) {queue_position+=1;Some(queue_position)} else {None};
                Ok(view)
            }).collect::<Result<Vec<_>>>()?;
            rows.sort_by(|a,b|b.added_at.cmp(&a.added_at));
            return Ok(EngineResult::Torrents(rows));
        }
        let mut ledger = read_ledger(&self.directory.join("torrents.json"))?;
        ledger.generation = self.generation.clone();
        if let EngineOp::Rename {hash,name}=&op {rename::apply(&mut ledger,hash,name)?;save_parked_ledger(&self.directory,&ledger)?;return Ok(EngineResult::Acknowledged);}
        if matches!(&op, EngineOp::Stage {..} | EngineOp::Commit {..} | EngineOp::Discard {..}) {
            let folder=Config::load()?.save_path.map(PathBuf::from).unwrap_or_else(||dirs::download_dir().unwrap_or_else(||crate::config::data_dir().join("downloads")));
            return staging::execute(&self.directory,&folder,&mut ledger,&op);
        }
        if let Some(result) = undo::parked(&self.undo, &self.directory, &mut ledger, &op) { return result; }
        if let Some(edit) = self.edit_record(&mut ledger, &op, &network) {
            edit?;
            save_parked_ledger(&self.directory, &ledger)?;
            return Ok(EngineResult::Acknowledged);
        }
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
            EngineOp::Trackers {hash} => {
                let record=ledger.records.iter().find(|r|&r.hash==hash).context("torrent record not found")?;
                return Ok(EngineResult::Trackers(trackers::saved(record,&self.directory)?));
            }
            EngineOp::Peers{hash}=>{
                anyhow::ensure!(ledger.records.iter().any(|r|&r.hash==hash),"torrent record not found");
                return Ok(EngineResult::Peers(Vec::new()));
            }
            EngineOp::Files { hash } => {
                let record = ledger
                    .records
                    .iter()
                    .find(|record| &record.hash == hash)
                    .context("torrent record not found")?;
                let info = self.saved.info(record, &self.directory)?.ok_or_else(|| ParkedRefusal(format!(
                    "saved torrent metadata is unavailable; connect {network} to resolve it"
                )))?;
                return Ok(EngineResult::Files(
                    info.files.iter()
                        .enumerate()
                        .map(|(index, file)| TorrentFile {
                            index: index as i64,
                            name: file.name.clone(),
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
                return Err(ParkedRefusal(format!(
                    "connect {network} before removing downloaded files; removing the saved torrent while keeping files is available now"
                )).into());
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
}
