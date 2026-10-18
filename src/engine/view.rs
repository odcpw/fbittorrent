//! The existing qBittorrent-shaped torrent list and state mapping.
use super::*;

impl Inner {
    pub(super) fn unrestored_snapshot(&self, record: &Record) -> Torrent {
        let restores = self.restores.lock().unwrap();
        let restore = restores.get(&record.hash);
        let error_message = restore.and_then(|entry| entry.last_error.clone());
        let state = match restore.map(|entry| &entry.outcome) {
            Some(RestoreOutcome::Failed { .. }) => "error",
            Some(RestoreOutcome::Checking) => "checkingDL",
            Some(RestoreOutcome::Stopped) => if record.completion_on > 0 { "stoppedUP" } else { "stoppedDL" },
            _ => "metaDL",
        };
        Torrent {
            hash: record.hash.clone(), name: record.name.clone(), state: state.into(), error_message,
            progress_known: false, eta: 8640000, save_path: record.output_folder.clone(), content_path: record.output_folder.clone(),
            category: record.category.clone(), added_on: record.added_on, completion_on: record.completion_on,
            up_limit: record.upload_limit, ratio_limit: record.ratio_limit, tags: record.tags.clone(),
            seq_dl: true, f_l_piece_prio: true, ..Default::default()
        }
    }
}

impl EngineDispatcher {
    pub(super) async fn info(&self, cx: &Cx) -> Result<String> {
        let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let mut torrents = Vec::new();
        for record in &ledger.records {
            torrents.push(match self.inner.handle(&record.hash) {
                Ok(handle) => {
                    let mut torrent = snapshot(record, &handle, self.inner.check_cancelled(&record.hash, &handle));
                    if torrent.error_message.is_none() {
                        torrent.error_message = self.inner.restores.lock().unwrap()
                            .get(&record.hash).and_then(|restore| restore.last_error.clone());
                    }
                    torrent
                },
                Err(_) => self.inner.unrestored_snapshot(record),
            });
        }
        torrents.sort_by(|a, b| b.added_on.cmp(&a.added_on));
        return json(&torrents);
    }
}

fn snapshot(record: &Record, handle: &Arc<ManagedTorrent>, check_cancelled: bool) -> Torrent {
    let stats = handle.stats();
    let progress = if stats.total_bytes > 0 {
        (stats.progress_bytes as f64 / stats.total_bytes as f64).min(1.0)
    } else if stats.finished {
        1.0
    } else {
        0.0
    };
    let (dlspeed, upspeed, peers) = stats
        .live
        .as_ref()
        .map(|live| {
            (
                live.download_speed.as_bytes() as i64,
                live.upload_speed.as_bytes() as i64,
                live.snapshot.peer_stats.live as i64,
            )
        })
        .unwrap_or_default();
    let state = if handle.is_paused()
        && !record.paused
        && !stats.finished
        && !matches!(stats.state, TorrentStatsState::Error)
        && (!matches!(stats.state, TorrentStatsState::Initializing { .. }) || check_cancelled)
    {
        "queuedDL"
    } else {
        match stats.state {
            TorrentStatsState::Initializing { .. } if check_cancelled => "stoppedDL",
            TorrentStatsState::Initializing { .. } => "checkingDL",
            TorrentStatsState::Paused => {
                if stats.finished {
                    "stoppedUP"
                } else {
                    "stoppedDL"
                }
            }
            TorrentStatsState::Live => {
                if stats.finished {
                    "uploading"
                } else if dlspeed > 0 {
                    "downloading"
                } else {
                    "stalledDL"
                }
            }
            TorrentStatsState::Error => "error",
        }
    };
    let content_path = handle
        .metadata
        .load_full()
        .map(|metadata| {
            if metadata.file_infos.len() == 1 {
                handle
                    .output_folder()
                    .join(&metadata.file_infos[0].relative_filename)
            } else {
                handle.output_folder().to_path_buf()
            }
        })
        .unwrap_or_else(|| PathBuf::from(&record.output_folder));
    Torrent {
        hash: record.hash.clone(),
        name: record.name.clone(),
        state: state.into(),
        error_message: stats.error.clone(),
        progress,
        progress_known: true,
        size: stats.total_bytes as i64,
        completed: stats.progress_bytes as i64,
        dlspeed,
        upspeed,
        eta: if dlspeed > 0 {
            (stats.total_bytes.saturating_sub(stats.progress_bytes) / dlspeed as u64) as i64
        } else {
            8640000
        },
        // rqbit does not expose the swarm's seed/leecher split; peer count is available.
        num_seeds: 0,
        num_leechs: peers,
        ratio: if stats.progress_bytes > 0 {
            stats.uploaded_bytes as f64 / stats.progress_bytes as f64
        } else {
            0.0
        },
        seq_dl: true,
        f_l_piece_prio: true,
        save_path: record.output_folder.clone(),
        content_path: content_path.to_string_lossy().into_owned(),
        category: record.category.clone(),
        added_on: record.added_on,
        completion_on: record.completion_on,
        up_limit: record.upload_limit,
        ratio_limit: record.ratio_limit,
        tags: record.tags.clone(),
    }
}
