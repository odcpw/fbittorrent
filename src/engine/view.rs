//! Normalized torrent views and the single legacy qBittorrent conversion.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentState {
    Metadata,
    Checking,
    Downloading,
    Stalled,
    Queued,
    Paused,
    Done,
    Seeding,
    WaitingVpn,
    Error,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Run,
    Pause,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPolicy {
    DefaultCap,
    Unlimited,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Activity {
    Metadata,
    Checking,
    Paused,
    Live,
    Error,
}
#[derive(Debug, Clone, Copy)]
pub(super) struct StateFacts {
    pub activity: Activity,
    pub paused: bool,
    pub finished: bool,
    pub stop_on_complete: bool,
    pub record_paused: bool,
    pub check_cancelled: bool,
    pub receiving: bool,
    pub parked: bool,
}

fn derive_state(f: StateFacts) -> (TorrentState, Option<&'static str>) {
    use TorrentState::*;
    if f.parked {
        return (WaitingVpn, Some("vpn_down"));
    }
    match (f.activity, f.check_cancelled) {
        (Activity::Error, _) => (Error, None),
        // Saved pause is transfer intent; an active verification still checks.
        (Activity::Checking, false) => (Checking, None),
        (Activity::Metadata, _) => (Metadata, None),
        (Activity::Checking, true) | (Activity::Paused, _) | (Activity::Live, _)
            if f.record_paused =>
        {
            (Paused, Some("user"))
        }
        (Activity::Checking, true) | (Activity::Paused, _) | (Activity::Live, _)
            if f.finished && f.stop_on_complete =>
        {
            (Done, Some("complete_no_seed"))
        }
        (Activity::Checking, true) | (Activity::Paused, _) | (Activity::Live, _)
            if f.paused && !f.finished =>
        {
            (Queued, Some("queue_full"))
        }
        (Activity::Live, _) if f.finished => (Seeding, None),
        (Activity::Live, _) if f.receiving => (Downloading, None),
        (Activity::Live, _) => (Stalled, None),
        // Name both inactive cases so a new Activity requires a mapping.
        (Activity::Checking, true) | (Activity::Paused, _) => (Paused, None),
    }
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Handle,
    Dormant,
    Unrestored {
        checking: bool,
        stopped: bool,
        failed: bool,
    },
    Parked,
}
#[derive(Debug, Clone)]
struct Legacy {
    source: Source,
    facts: StateFacts,
    name: String,
    category: String,
    tags: String,
    completion_on: i64,
    ratio_limit: f64,
    progress_known: bool,
    content_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TorrentView {
    pub hash: String,
    pub name: String,
    pub state: TorrentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub intent: Intent,
    pub staged: bool,
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dl_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ul_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<u64>,
    pub seed: bool,
    pub upload_policy: UploadPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_limit_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    pub save_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<String>,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_pos: Option<u64>,
    #[serde(skip)]
    legacy: Legacy,
}

impl TorrentView {
    fn rechecking(&mut self) {
        self.state = TorrentState::Checking;
        self.checked = false;
        self.reason = None;
        self.completed_at = None;
    }

    fn retain_restore_reason(&mut self) {
        if self
            .error
            .as_deref()
            .is_some_and(|message| message.ends_with(" restore failed: missing files"))
        {
            // Stop and retry retain the last restore error until success. Its
            // reason must remain consistent with that visible error as well.
            self.reason = Some("missing_files".into());
        }
    }

    fn record(record: &Record, facts: StateFacts, source: Source) -> Self {
        let (state, reason) = derive_state(facts);
        Self {
            hash: record.hash.clone(),
            name: record.name.chars().take(1024).collect(),
            state,
            reason: reason.map(str::to_owned),
            error: None,
            intent: if record.paused {
                Intent::Pause
            } else {
                Intent::Run
            },
            staged: record.staged,
            checked: false,
            progress: None,
            size: None,
            completed_bytes: None,
            uploaded_bytes: None,
            dl_bps: None,
            ul_bps: None,
            eta_s: None,
            peers: None,
            seed: !record.stop_on_complete(),
            upload_policy: if record.upload_limit == 0 && record.ratio_limit == -2.0 {
                UploadPolicy::Unlimited
            } else if record.upload_limit == NO_UPLOAD_LIMIT && record.ratio_limit == 0.0 {
                UploadPolicy::DefaultCap
            } else {
                UploadPolicy::Custom
            },
            upload_limit_bps: u64::try_from(record.upload_limit).ok(),
            ratio: None,
            save_path: record.output_folder.clone(),
            content_path: None,
            added_at: record.added_on,
            completed_at: (record.completion_on > 0).then_some(record.completion_on),
            origin: None, // projected once this row is attached to its owned ledger
            force: record.force,
            queue_pos: None,
            legacy: Legacy {
                source,
                facts,
                name: record.name.clone(),
                category: record.category.clone(),
                tags: record.tags.clone(),
                completion_on: record.completion_on,
                ratio_limit: record.ratio_limit,
                progress_known: false,
                content_path: record.output_folder.clone(),
            },
        }
    }

    fn trusted_pending(record:&Record, value:resume::Measurement)->Self {
        let facts=StateFacts {activity:Activity::Paused,paused:true,finished:value.have==value.size,
            stop_on_complete:record.stop_on_complete(),record_paused:record.paused,
            check_cancelled:false,receiving:false,parked:false};
        let mut view=Self::record(record,facts,Source::Handle);
        view.checked=true;view.size=Some(value.size);view.completed_bytes=Some(value.have);
        view.progress=Some(if value.size==0 {1.0} else {value.have as f64/value.size as f64});
        view.legacy.progress_known=true;
        view
    }

    fn handle(record: &Record, handle: &Arc<ManagedTorrent>, check_cancelled: bool) -> Self {
        let stats = handle.stats();
        let activity = match stats.state {
            TorrentStatsState::Initializing { .. } => Activity::Checking,
            TorrentStatsState::Paused => Activity::Paused,
            TorrentStatsState::Live => Activity::Live,
            TorrentStatsState::Error => Activity::Error,
        };
        let facts = StateFacts {
            activity,
            paused: handle.is_paused(),
            finished: stats.finished,
            stop_on_complete: record.stop_on_complete(),
            record_paused: record.paused,
            check_cancelled,
            receiving: stats
                .live
                .as_ref()
                .is_some_and(|live| live.download_speed.as_bytes() > 0),
            parked: false,
        };
        let mut view = Self::record(record, facts, Source::Handle);
        let metadata = handle.metadata.load_full();
        view.populate_stats(&stats, metadata.is_some());
        if let Some(metadata) = metadata {
            let content = if metadata.file_infos.len() == 1 {
                handle
                    .output_folder()
                    .join(&metadata.file_infos[0].relative_filename)
            } else {
                handle.output_folder().to_path_buf()
            };
            view.content_path = Some(content.to_string_lossy().into());
            view.legacy.content_path = view.content_path.clone().unwrap();
        }
        view
    }

    fn populate_stats(&mut self, stats: &librqbit::TorrentStats, metadata_known: bool) {
        self.error = stats.error.clone();
        self.checked = matches!(
            self.legacy.facts.activity,
            Activity::Paused | Activity::Live
        );
        self.legacy.progress_known = true;
        if metadata_known {
            self.progress = Some(if stats.total_bytes > 0 {
                (stats.progress_bytes as f64 / stats.total_bytes as f64).min(1.0)
            } else if stats.finished {
                1.0
            } else {
                0.0
            });
            self.size = Some(stats.total_bytes);
            self.completed_bytes = Some(stats.progress_bytes);
            self.uploaded_bytes = Some(stats.uploaded_bytes);
            self.ratio = (stats.progress_bytes > 0)
                .then(|| stats.uploaded_bytes as f64 / stats.progress_bytes as f64);
        }
        if self.legacy.facts.activity == Activity::Live {
            if let Some(live) = &stats.live {
                self.dl_bps = Some(live.download_speed.as_bytes());
                self.ul_bps = Some(live.upload_speed.as_bytes());
                self.peers = Some(live.snapshot.peer_stats.live as u64);
                if self.state == TorrentState::Downloading && live.download_speed.as_bytes() > 0 {
                    self.eta_s = Some(
                        stats.total_bytes.saturating_sub(stats.progress_bytes)
                            / live.download_speed.as_bytes(),
                    );
                }
            }
        }
    }

    pub(super) fn unrestored(record: &Record, restore: Option<&RestoreTask>) -> Self {
        if restore.is_some_and(|entry| matches!(entry.outcome, RestoreOutcome::Dormant)) {
            let facts = StateFacts {
                activity: Activity::Paused, paused: true, finished: true,
                stop_on_complete: true, record_paused: record.paused,
                check_cancelled: false, receiving: false, parked: false,
            };
            let mut view = Self::record(record, facts, Source::Dormant);
            // Completion is saved history, explicitly not a new data check.
            view.state = TorrentState::Done;
            view.reason = Some("complete_no_seed".into());
            view.progress = Some(1.0);
            view.legacy.progress_known = true;
            return view;
        }
        let checking =
            restore.is_some_and(|entry| matches!(entry.outcome, RestoreOutcome::Checking));
        let stopped = restore.is_some_and(|entry| matches!(entry.outcome, RestoreOutcome::Stopped));
        let failed =
            restore.is_some_and(|entry| matches!(entry.outcome, RestoreOutcome::Failed { .. }));
        let facts = StateFacts {
            activity: if failed {
                Activity::Error
            } else if checking {
                Activity::Checking
            } else if stopped {
                Activity::Paused
            } else {
                Activity::Metadata
            },
            paused: stopped,
            finished: stopped && record.completion_on > 0,
            stop_on_complete: record.stop_on_complete(),
            record_paused: record.paused,
            check_cancelled: false,
            receiving: false,
            parked: false,
        };
        let mut view = Self::record(
            record,
            facts,
            Source::Unrestored {
                checking,
                stopped,
                failed,
            },
        );
        view.error = restore.and_then(|entry| entry.last_error.clone());
        if let Some(RestoreOutcome::Failed { reason, message }) =
            restore.map(|entry| &entry.outcome)
        {
            view.reason = Some(
                if message.contains("metadata lookup timed out") {
                    "metadata_timeout"
                } else {
                    reason
                }
                .to_string(),
            );
        }
        view.retain_restore_reason();
        view
    }

    pub(super) fn parked(record: &Record, metadata: Option<(u64, Option<PathBuf>)>) -> Self {
        let facts = StateFacts {
            activity: Activity::Paused,
            paused: true,
            finished: record.completion_on > 0,
            stop_on_complete: record.stop_on_complete(),
            record_paused: record.paused,
            check_cancelled: false,
            receiving: false,
            parked: true,
        };
        let mut view = Self::record(record, facts, Source::Parked);
        if let Some((size, path)) = metadata {
            view.size = Some(size);
            view.content_path = Some(
                path.unwrap_or_else(|| PathBuf::from(&record.output_folder))
                    .to_string_lossy()
                    .into(),
            );
            view.legacy.content_path = view.content_path.clone().unwrap();
        }
        view
    }

    /// This is the only qBittorrent state-string mapping. Preserve legacy unknown
    /// sentinels here; normalized serialization above always omits unknown data.
    pub fn to_legacy_json(&self) -> Result<String> {
        let f = self.legacy.facts;
        let state = match self.legacy.source {
            Source::Dormant => "stoppedUP",
            Source::Parked if !f.record_paused => "waitingVPN",
            Source::Parked => {
                if f.finished {
                    "stoppedUP"
                } else {
                    "stoppedDL"
                }
            }
            Source::Unrestored { failed: true, .. } => "error",
            Source::Unrestored { checking: true, .. } => "checkingDL",
            Source::Unrestored { stopped: true, .. } => {
                if f.finished {
                    "stoppedUP"
                } else {
                    "stoppedDL"
                }
            }
            Source::Unrestored { .. } => "metaDL",
            Source::Handle
                if f.paused
                    && !f.record_paused
                    && !f.finished
                    && f.activity != Activity::Error
                    && (f.activity != Activity::Checking || f.check_cancelled) =>
            {
                "queuedDL"
            }
            Source::Handle => match f.activity {
                Activity::Checking if f.check_cancelled => "stoppedDL",
                Activity::Checking => "checkingDL",
                Activity::Paused => {
                    if f.finished {
                        "stoppedUP"
                    } else {
                        "stoppedDL"
                    }
                }
                Activity::Live if f.finished => "uploading",
                Activity::Live if f.receiving => "downloading",
                Activity::Live => "stalledDL",
                Activity::Error => "error",
                Activity::Metadata => "metaDL",
            },
        };
        let dl = self.dl_bps.unwrap_or(0);
        json(&Torrent {
            hash: self.hash.clone(),
            name: self.legacy.name.clone(),
            state: state.into(),
            error_message: self.error.clone(),
            progress: self.progress.unwrap_or(0.0),
            progress_known: self.legacy.progress_known,
            size: self.size.unwrap_or(0) as i64,
            completed: self.completed_bytes.unwrap_or(0) as i64,
            dlspeed: dl as i64,
            upspeed: self.ul_bps.unwrap_or(0) as i64,
            eta: if dl > 0 {
                (self
                    .size
                    .unwrap_or(0)
                    .saturating_sub(self.completed_bytes.unwrap_or(0))
                    / dl) as i64
            } else {
                8640000
            },
            num_seeds: 0,
            num_leechs: self.peers.unwrap_or(0) as i64,
            ratio: self.ratio.unwrap_or(0.0),
            seq_dl: true,
            f_l_piece_prio: true,
            save_path: self.save_path.clone(),
            content_path: self.legacy.content_path.clone(),
            category: self.legacy.category.clone(),
            added_on: self.added_at,
            completion_on: self.legacy.completion_on,
            up_limit: self.upload_limit_bps.unwrap_or(0) as i64,
            ratio_limit: self.legacy.ratio_limit,
            tags: self.legacy.tags.clone(),
        })
    }
}

impl Inner {
    pub(super) fn unrestored_view(&self, record: &Record) -> TorrentView {
        let mut view = TorrentView::unrestored(record, self.restores.lock().unwrap().get(&record.hash));
        if matches!(view.legacy.source, Source::Dormant) {
            if let Ok(saved) = self.saved_torrent(record) {
                view.size = Some(saved.selected_bytes);
                view.completed_bytes = Some(saved.selected_bytes);
                let mut path = PathBuf::from(&record.output_folder);
                if saved.files.len() == 1 {
                    path.push(&saved.files[0].name);
                }
                view.content_path = Some(path.to_string_lossy().into());
                view.legacy.content_path = view.content_path.clone().unwrap();
            }
        }
        view
    }
    #[cfg(test)]
    pub(super) fn unrestored_snapshot(&self, record: &Record) -> Torrent {
        serde_json::from_str(&self.unrestored_view(record).to_legacy_json().unwrap()).unwrap()
    }
}
impl EngineDispatcher {
    pub(super) async fn info(&self, cx: &Cx) -> Result<Vec<TorrentView>> {
        let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let mut torrents = Vec::new();
        let mut queue_position=0;
        for record in &ledger.records {
            let mut view = match self.inner.handle(&record.hash) {
                Ok(handle) => TorrentView::handle(
                    record,
                    &handle,
                    self.inner.check_cancelled(&record.hash, &handle),
                ),
                Err(_) => self.inner.unrestored_view(record),
            };
            if let Some(value)=self.inner.resume_prepared.pending(&record.hash) {
                if view.state==TorrentState::Checking || view.state==TorrentState::Metadata {
                    view=TorrentView::trusted_pending(record,value);
                } else if !self.inner.moves.active(&record.hash) {self.inner.resume_prepared.finish(&record.hash);}
            }
            if view.error.is_none() {
                view.error = self
                    .inner
                    .restores
                    .lock()
                    .unwrap()
                    .get(&record.hash)
                    .and_then(|restore| restore.last_error.clone());
            }
            if self.inner.rechecks.active(&record.hash) {
                view.rechecking();
            } else if self.inner.recheck_failed(&record.hash) {
                view.state = TorrentState::Error;
                view.checked = false;
                view.reason = Some("restore_failed".into());
                view.completed_at = None;
                view.completed_bytes = None;
                view.progress = None;
            }
            view.origin=Some(record.origin.clone());
            view.queue_pos=if queue_policy::positioned(record) {queue_position+=1;Some(queue_position)} else {None};
            view.retain_restore_reason();
            torrents.push(view);
        }
        torrents.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        Ok(torrents)
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
