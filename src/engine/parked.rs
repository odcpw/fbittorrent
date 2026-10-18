//! Saved-history and durable-intent routes while the VPN-bound engine is parked.
use super::*;

// A parked application has no native session or network work. Its IPC workers
// still serialize saved intent so simultaneous widget/CLI actions cannot lose
// another action's ledger update.
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

fn parked_snapshot(record: &Record, directory: &Path) -> Result<serde_json::Value> {
    let mut torrent = Torrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        state: if record.paused {
            if record.completion_on > 0 {
                "stoppedUP"
            } else {
                "stoppedDL"
            }
        } else {
            "waitingVPN"
        }
        .into(),
        save_path: record.output_folder.clone(),
        content_path: record.output_folder.clone(),
        category: record.category.clone(),
        added_on: record.added_on,
        completion_on: record.completion_on,
        up_limit: record.upload_limit,
        ratio_limit: record.ratio_limit,
        tags: record.tags.clone(),
        seq_dl: true,
        f_l_piece_prio: true,
        eta: 8640000,
        ..Default::default()
    };
    if let Some(bytes) = parked_metadata(record, directory)? {
        let metadata = librqbit::torrent_from_bytes(&bytes)?;
        let info = metadata.info.data.validate()?;
        torrent.size = info.lengths().total_length() as i64;
        let files: Vec<_> = info.iter_file_details().collect();
        if files.len() == 1 {
            torrent.content_path = Path::new(&record.output_folder)
                .join(files[0].filename.to_pathbuf())
                .to_string_lossy()
                .into_owned();
        }
    }
    let mut value = serde_json::to_value(torrent)?;
    value["progress_known"] = serde_json::Value::Bool(false);
    Ok(value)
}

pub(super) fn dispatch_parked_in(request: &Request, directory: &Path, lifecycle: &Lifecycle) -> Option<Result<String>> {
    if !matches!(
        request.path.as_str(),
        "app/version"
            | "app/shutdown"
            | "app/queue"
            | "app/setQueue"
            | "transfer/info"
            | "torrents/info"
            | "torrents/files"
            | "torrents/start"
            | "torrents/stop"
            | "torrents/delete"
    ) {
        return if request.path.starts_with("torrents/") || request.path == "app/setPreferences" {
            Some(Err(anyhow!("connect ProtonVPN before changing downloads; saved downloads and settings remain available")))
        } else {
            None
        };
    }
    Some((|| {
        let _guard = PARKED_LEDGER_LOCK
            .lock()
            .map_err(|_| anyhow!("saved torrent lock is unavailable"))?;
        match request.path.as_str() {
            "app/version" => {
                return Ok(format!(
                    "native librqbit {} (waiting for ProtonVPN)",
                    librqbit::version()
                ))
            }
            "app/shutdown" => {
                lifecycle.request_stop(StopReason::Quit);
                return Ok("Ok.".into());
            }
            "transfer/info" => {
                return json(&TransferInfo {
                    connection_status: "vpn_waiting".into(),
                    ..Default::default()
                })
            }
            "app/setQueue" => {
                let limit: usize = required(request, "max_active_downloads")?.parse()?;
                if limit == 0 {
                    bail!("max_active_downloads must be at least 1");
                }
                let mut cfg = Config::load()?;
                cfg.max_active_downloads = limit;
                cfg.save()?;
                return Ok("Ok.".into());
            }
            _ => {}
        }
        let mut ledger = read_ledger(&directory.join("torrents.json"))?;
        match request.path.as_str() {
            "app/queue" => {
                return json(&serde_json::json!({
                    "max_active_downloads": Config::load()?.max_active_downloads,
                    "active_downloads": 0,
                    "queued_downloads": ledger.records.iter().filter(|record| !record.paused && !(record.stop_on_complete() && record.completion_on > 0)).count(),
                    "network_blocked": true,
                }))
            }
            "torrents/info" => {
                ledger.records.sort_by(|a, b| b.added_on.cmp(&a.added_on));
                let torrents: Vec<_> = ledger
                    .records
                    .iter()
                    .map(|record| parked_snapshot(record, directory))
                    .collect::<Result<_>>()?;
                return json(&torrents);
            }
            "torrents/files" => {
                let hash = required(request, "hash")?;
                let record = ledger
                    .records
                    .iter()
                    .find(|record| record.hash == hash)
                    .context("torrent record not found")?;
                let bytes = parked_metadata(record, directory)?.context(
                    "saved torrent metadata is unavailable; connect ProtonVPN to resolve it",
                )?;
                let metadata = librqbit::torrent_from_bytes(&bytes)?;
                let info = metadata.info.data.validate()?;
                let files: Vec<_> = info.iter_file_details().enumerate().map(|(index, file)| serde_json::json!({
                    "index": index,
                    "name": file.filename.to_pathbuf().to_string_lossy(),
                    "size": file.len,
                    "progress": 0.0,
                    "progress_known": false,
                    "priority": if record.only_files.as_ref().map(|ids| ids.contains(&index)).unwrap_or(true) { 1 } else { 0 },
                })).collect();
                return json(&files);
            }
            _ => {}
        }
        let (hashes, start_result) = mutation_hashes(request, &ledger)?;
        if request.path == "torrents/delete" {
            if request.arg("deleteFiles") == Some("true") {
                bail!("connect ProtonVPN before removing downloaded files; removing the saved torrent while keeping files is available now");
            }
            ledger
                .records
                .retain(|record| !hashes.contains(&record.hash));
            save_parked_ledger(directory, &ledger)?;
            for hash in hashes {
                let _ = std::fs::remove_file(directory.join(format!("{hash}.torrent")));
            }
        } else {
            for record in &mut ledger.records {
                if hashes.contains(&record.hash) {
                    record.paused = request.path == "torrents/stop";
                    record.resume_after_migration = false;
                }
            }
            save_parked_ledger(directory, &ledger)?;
        }
        match start_result { Some(result) => json(&result), None => Ok("Ok.".into()) }
    })())
}

/// Serve saved history and durable intent while no VPN-bound session exists.
pub fn dispatch_parked(request: &Request, lifecycle: &Lifecycle) -> Option<Result<String>> {
    dispatch_parked_in(request, &crate::config::data_dir().join("engine"), lifecycle)
}
