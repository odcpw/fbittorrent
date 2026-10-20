//! Normalized torrent views: the engine side of the one torrent model.
use super::*;
pub use crate::api::{Intent, TorrentState, UploadPolicy};

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

/// Unknown values are None; the API serializes them by omission.
#[derive(Debug, Clone)]
pub struct TorrentView {
    pub hash: String,
    pub name: String,
    pub state: TorrentState,
    pub reason: Option<String>,
    pub error: Option<String>,
    pub intent: Intent,
    pub staged: bool,
    pub checked: bool,
    pub progress: Option<f64>,
    pub size: Option<u64>,
    pub completed_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub dl_bps: Option<u64>,
    pub ul_bps: Option<u64>,
    pub eta_s: Option<u64>,
    pub peers: Option<u64>,
    pub seeds: Option<u64>,
    pub seed: bool,
    pub upload_policy: UploadPolicy,
    pub upload_enabled: bool,
    pub upload_limit_bps: Option<u64>,
    pub download_limit_bps: Option<u64>,
    pub effective_download_limit_bps: Option<u64>,
    pub ratio: Option<f64>,
    pub save_path: String,
    pub content_path: Option<String>,
    pub added_at: i64,
    pub completed_at: Option<i64>,
    pub force: bool,
    pub origin: Option<String>,
    pub queue_pos: Option<u64>,
    pub piece_length: Option<u64>,
    activity: Activity,
    dormant: bool,
    original_name: String,
}

impl TorrentView {
    pub(crate) fn original_name(&self)->&str {&self.original_name}

    fn rechecking(&mut self) {
        self.state = TorrentState::Checking;
        self.checked = false;
        self.reason = None;
        self.completed_at = None;
    }

    pub(super) fn files_missing(&mut self) {
        self.state = TorrentState::Error;
        self.reason = Some("missing_files".into());
        self.error = Some(format!("Files missing: {} restore failed: missing files", self.name));
        self.checked = false;
        self.progress = None;
        self.completed_bytes = None;
        self.completed_at = None;
        self.dl_bps = None;
        self.ul_bps = None;
        self.eta_s = None;
        self.activity = Activity::Error;
    }

    fn retain_restore_reason(&mut self) {
        if self.error.as_deref().is_some_and(super::disk::is_full) {
            self.state = TorrentState::Error;
            self.reason = Some("disk_full".into());
            if let Some(message) = &mut self.error { if !message.starts_with("Disk full") { *message = format!("Disk full: {message}"); } }
            return;
        }
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

    fn record(record: &Record, facts: StateFacts, dormant: bool) -> Self {
        let (state, reason) = derive_state(facts);
        Self {
            hash: record.hash.clone(),
            name: record.display_name.as_deref().unwrap_or(&record.name).chars().take(1024).collect(),
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
            seeds: None,
            seed: !record.stop_on_complete(),
            upload_enabled: record.upload == Some(true),
            upload_policy: if record.upload != Some(true) { UploadPolicy::Off }
                else if record.upload_limit > 0 { UploadPolicy::Custom } else { UploadPolicy::Unlimited },
            upload_limit_bps: u64::try_from(record.upload_limit).ok(),
            download_limit_bps: record.download_limit,
            effective_download_limit_bps: None,
            ratio: None,
            save_path: record.output_folder.clone(),
            content_path: None,
            added_at: record.added_on,
            completed_at: (record.completion_on > 0).then_some(record.completion_on),
            origin: None, // projected once this row is attached to its owned ledger
            force: record.force,
            queue_pos: None,
            piece_length: None,
            activity: facts.activity,
            dormant,
            original_name: record.name.clone(),
        }
    }

    fn trusted_pending(record:&Record, value:resume::Measurement)->Self {
        let facts=StateFacts {activity:Activity::Paused,paused:true,finished:value.have==value.size,
            stop_on_complete:record.stop_on_complete(),record_paused:record.paused,
            check_cancelled:false,receiving:false,parked:false};
        let mut view=Self::record(record,facts,false);
        view.checked=true;view.size=Some(value.size);view.completed_bytes=Some(value.have);
        view.progress=Some(if value.size==0 {1.0} else {value.have as f64/value.size as f64});
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
        let mut view = Self::record(record, facts, false);
        let metadata = handle.metadata.load_full();
        view.populate_stats(&stats, metadata.is_some());
        view.effective_download_limit_bps=handle.effective_download_bps();
        view.seeds = handle.live().map(|live| live.connected_seed_count() as u64);
        if view.state==TorrentState::Stalled && handle.shared().tracker_stats.all_failed() {view.error=Some(crate::user_messages::TRACKERS.into());}
        if let Some(metadata) = metadata {
            view.piece_length = Some(u64::from(metadata.info.lengths().default_piece_length()));
            let content = if metadata.file_infos.len() == 1 {
                handle
                    .output_folder()
                    .join(&metadata.file_infos[0].relative_filename)
            } else {
                handle.output_folder().to_path_buf()
            };
            view.content_path = Some(content.to_string_lossy().into());
        }
        view
    }

    fn populate_stats(&mut self, stats: &librqbit::TorrentStats, metadata_known: bool) {
        self.error = stats.error.clone();
        self.checked = matches!(self.activity, Activity::Paused | Activity::Live);
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
        if self.activity == Activity::Live {
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
            let mut view = Self::record(record, facts, true);
            // Completion is saved history, explicitly not a new data check.
            view.state = TorrentState::Done;
            view.reason = Some("complete_no_seed".into());
            view.progress = Some(1.0);
            return view;
        }
        if record.staged {
            let facts=StateFacts {activity:Activity::Paused,paused:true,finished:false,
                stop_on_complete:record.stop_on_complete(),record_paused:true,check_cancelled:false,receiving:false,parked:false};
            let mut view=Self::record(record,facts,false);
            if let Some(RestoreOutcome::Failed {reason,message})=restore.map(|r|&r.outcome) {
                view.reason=Some(if message.contains("metadata lookup timed out") {"metadata_timeout"} else {reason}.into());
            }
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
        let mut view = Self::record(record, facts, false);
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
        let mut view = if record.staged {Self::unrestored(record,None)} else {Self::record(record, facts, false)};
        if let Some((size, path)) = metadata {
            view.size = Some(size);
            view.content_path = Some(
                path.unwrap_or_else(|| PathBuf::from(&record.output_folder))
                    .to_string_lossy()
                    .into(),
            );
        }
        view
    }
}

impl Inner {
    /// `fresh` is an explicit read; the shared sampler reuses bounded presence.
    pub(super) fn unrestored_view(&self, record: &Record, fresh: bool) -> TorrentView {
        let mut view = TorrentView::unrestored(record, self.restores.lock().unwrap().get(&record.hash));
        if record.staged {
            if let Ok(saved)=self.saved_torrent_observed(record, fresh) {view.size=Some(saved.files.iter().map(|f|f.size.max(0) as u64).sum());}
        }
        if view.dormant {
            if let Ok(saved) = self.saved_torrent_observed(record, fresh) {
                view.size = Some(saved.selected_bytes);
                view.completed_bytes = Some(saved.selected_bytes);
                let mut path = PathBuf::from(&record.output_folder);
                if saved.files.len() == 1 {
                    path.push(&saved.files[0].name);
                }
                view.content_path = Some(path.to_string_lossy().into());
                if !saved.selected_files_present { view.files_missing(); }
            }
        }
        view
    }
}
impl EngineDispatcher {
    pub(super) async fn info(&self, cx: &Cx) -> Result<Vec<TorrentView>> {
        cx.checkpoint()?;
        let ledger = self.observation_ledger().await?;
        Ok(self.info_locked(&ledger))
    }
    /// Explicit reads stat payload presence now.
    pub(super) fn info_locked(&self, ledger: &Ledger) -> Vec<TorrentView> {
        self.info_observed(ledger, true)
    }
    pub(super) fn info_observed(&self, ledger: &Ledger, fresh: bool) -> Vec<TorrentView> {
        let mut torrents = Vec::with_capacity(ledger.records.len());
        let mut queue_position=0;
        for record in &ledger.records {
            let handle = self.inner.handle(&record.hash).ok();
            let mut view = match &handle {
                Some(handle) => TorrentView::handle(
                    record,
                    handle,
                    self.inner.check_cancelled(&record.hash, handle),
                ),
                None => self.inner.unrestored_view(record, fresh),
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
            view.upload_enabled = record.upload.unwrap_or(self.inner.session.upload_enabled() && view.state != TorrentState::Done);
            view.upload_policy = if !view.upload_enabled { UploadPolicy::Off }
                else if record.upload_limit > 0 { UploadPolicy::Custom } else { UploadPolicy::Unlimited };
            view.origin=Some(record.origin.clone());
            view.queue_pos=if queue_policy::positioned(record) {queue_position+=1;Some(queue_position)} else {None};
            // A handle whose downloaded payload was deleted is 'Files missing'
            // (dormant rows are checked by unrestored_view). Checks and moves
            // own their files; an engine error already explains itself.
            if let Some(handle) = &handle {
                if !matches!(view.state, TorrentState::Checking | TorrentState::Metadata | TorrentState::Error)
                    && !self.inner.rechecks.active(&record.hash)
                    && !self.inner.moves.active(&record.hash)
                    && self.inner.saved.live_missing(&record.hash, handle, fresh)
                {
                    view.files_missing();
                }
            }
            view.retain_restore_reason();
            torrents.push(view);
        }
        torrents.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        torrents
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
