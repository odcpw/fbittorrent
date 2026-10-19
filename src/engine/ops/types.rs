use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashSelection {
    All,
    Hashes(Vec<String>),
}

impl HashSelection {
    pub(in crate::engine) fn resolve(&self, ledger: &Ledger) -> Result<Vec<String>> {
        let hashes = match self {
            Self::All => ledger
                .records
                .iter()
                .map(|record| record.hash.clone())
                .collect(),
            Self::Hashes(hashes) => hashes.clone(),
        };
        for hash in &hashes {
            if !ledger.records.iter().any(|record| &record.hash == hash) {
                bail!("torrent not found: {hash}");
            }
        }
        Ok(hashes)
    }
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub sources: Vec<String>,
    pub torrent: Option<(String, Vec<u8>)>,
    pub save_path: Option<String>,
    pub upload_limit: i64,
    pub ratio_limit: f64,
    pub stopped: bool,
    pub category: String,
    pub tags: String,
}

#[derive(Debug, Clone)]
pub enum EngineOp {
    Version,
    Shutdown,
    Restart,
    Network,
    Settings,
    SetPreferences,
    Queue,
    SetQueue {
        limit: usize,
    },
    TransferInfo,
    Info,
    Get {
        hash: String,
    },
    Files {
        hash: String,
    },
    // Typed engine entry points for the daemon adapter. These do not add routes
    // to the legacy transport or advertise v1 operations before that API lands.
    OpenTarget {
        hash: String,
    },
    Recheck {
        hash: String,
    },
    SetFiles {
        hash: String,
        include: Vec<usize>,
    },
    SetSeed {
        hashes: HashSelection,
        on: bool,
    },
    Add(AddRequest),
    AddSelected {
        options: AddRequest,
        files: Vec<usize>,
    },
    /// API adds persist unresolved magnets without waiting for peer metadata.
    AddMagnet(AddRequest),
    Start {
        hashes: HashSelection,
        skip_complete: bool,
    },
    Stop {
        hashes: HashSelection,
    },
    Delete {
        hashes: HashSelection,
        files: bool,
    },
    FilePriority {
        hash: String,
        ids: Vec<usize>,
        priority: u8,
    },
    SetUploadLimit {
        hashes: HashSelection,
        limit: i64,
    },
    SetShareLimits {
        hashes: HashSelection,
        ratio: f64,
    },
    AddTags {
        hashes: HashSelection,
        tags: Vec<String>,
    },
    RemoveTags {
        hashes: HashSelection,
        tags: Vec<String>,
    },
    ToggleSequentialDownload,
    ToggleFirstLastPiecePrio,
}

impl EngineOp {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::FilePriority { priority, .. } if !matches!(priority, 0 | 1) => bail!(
                "the native engine supports skip (0) and normal (1) file priorities; high and maximum are unavailable"
            ),
            Self::SetUploadLimit { limit, .. } if *limit < 0 => {
                bail!("upload limit must be zero (unlimited) or a positive byte rate")
            }
            Self::SetShareLimits { ratio, .. } if !matches!(ratio, -2.0 | 0.0) => bail!(
                "native share limits support unlimited (-2) or stop on completion (ratio 0, Stop)"
            ),
            Self::Add(options) | Self::AddMagnet(options) | Self::AddSelected {options, ..} => {
                if options.upload_limit < 0 {
                    bail!("upload limit cannot be negative");
                }
                if !matches!(options.ratio_limit, -2.0 | 0.0) {
                    bail!(
                        "native share limits support unlimited (-2) or stop on completion (ratio 0)"
                    );
                }
                if options.sources.is_empty() && options.torrent.is_none() {
                    bail!("no torrent was supplied");
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn selection(&self) -> Option<&HashSelection> {
        match self {
            Self::Start { hashes, .. }
            | Self::Stop { hashes }
            | Self::Delete { hashes, .. }
            | Self::SetUploadLimit { hashes, .. }
            | Self::SetShareLimits { hashes, .. }
            | Self::AddTags { hashes, .. }
            | Self::RemoveTags { hashes, .. } => Some(hashes),
            _ => None,
        }
    }
    pub(crate) fn control(
        &self,
        snapshot: &crate::network_app::NetworkState,
        lifecycle: &Lifecycle,
    ) -> Option<Result<EngineResult>> {
        use crate::daemon::lifecycle::RestartReason;
        Some(match self {
            Self::Network => Ok(EngineResult::Network(snapshot.clone())),
            Self::Settings => Config::load().map(|cfg| {
                EngineResult::Settings(EngineSettings {
                    max_active_downloads: cfg.max_active_downloads,
                    peer_encryption: cfg.peer_encryption,
                    network_interface: cfg.network_interface,
                })
            }),
            Self::Restart => {
                lifecycle.request_restart(RestartReason::Explicit);
                Ok(EngineResult::Acknowledged)
            }
            Self::Shutdown => {
                lifecycle.request_stop(StopReason::Quit);
                Ok(EngineResult::Acknowledged)
            }
            _ => return None,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct QueueInfo {
    pub max_active_downloads: usize,
    pub active_downloads: usize,
    pub queued_downloads: usize,
    pub network_blocked: bool,
}
#[derive(Debug, Serialize)]
pub struct EngineSettings {
    pub max_active_downloads: usize,
    pub peer_encryption: String,
    pub network_interface: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddedTorrent {
    pub hash: String,
    pub name: String,
    pub duplicate: bool,
    pub deferred: bool,
}

#[derive(Debug)]
pub struct TorrentDetails {
    pub torrent: super::super::view::TorrentView,
    pub private: bool,
    pub file_count: Option<u64>,
    pub piece_count: Option<u64>,
    pub piece_size: Option<u64>,
    pub tracker_count: Option<u64>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
}

impl TorrentDetails {
    pub(in crate::engine) fn from_bytes(
        torrent: super::super::view::TorrentView,
        bytes: Option<&[u8]>,
    ) -> Result<Self> {
        let mut detail = Self {
            torrent,
            private: false,
            file_count: None,
            piece_count: None,
            piece_size: None,
            tracker_count: None,
            comment: None,
            created_by: None,
        };
        if let Some(bytes) = bytes {
            let metadata = librqbit::torrent_from_bytes(bytes)?;
            detail.private = metadata.info.data.private;
            detail.piece_size = Some(u64::from(metadata.info.data.piece_length));
            detail.tracker_count = Some(metadata.iter_announce().count() as u64);
            let info = metadata.info.data.validate()?;
            detail.file_count = Some(info.iter_file_details().count() as u64);
            detail.piece_count = Some(info.lengths().total_pieces() as u64);
            detail.comment = metadata
                .comment
                .as_ref()
                .map(|s| String::from_utf8_lossy(s.as_ref()).into_owned());
            detail.created_by = metadata
                .created_by
                .as_ref()
                .map(|s| String::from_utf8_lossy(s.as_ref()).into_owned());
        }
        Ok(detail)
    }
}

#[derive(Debug)]
pub enum EngineResult {
    Acknowledged,
    Version(String),
    Torrents(Vec<super::super::view::TorrentView>),
    Detail(TorrentDetails),
    Added(Vec<AddedTorrent>),
    Files(Vec<TorrentFile>),
    OpenTarget(PathBuf),
    Transfer(TransferInfo),
    Queue(QueueInfo),
    Start(crate::qbt::StartResult),
    Settings(EngineSettings),
    Network(crate::network_app::NetworkState),
}
impl EngineResult {
    pub fn to_legacy_json(&self) -> Result<String> {
        match self {
            Self::Acknowledged | Self::Added(_) => Ok("Ok.".into()),
            Self::Detail(_) => bail!("torrent detail has no legacy route"),
            Self::Version(version) => Ok(version.clone()),
            Self::Torrents(rows) => Ok(format!(
                "[{}]",
                rows.iter()
                    .map(|row| row.to_legacy_json())
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            )),
            Self::Files(files) => json(files),
            Self::OpenTarget(path) => json(path),
            Self::Transfer(info) => json(info),
            // These responses historically serialized through Value. Preserve
            // its object-member ordering at the legacy boundary.
            Self::Queue(info) => json(&serde_json::to_value(info)?),
            Self::Start(result) => json(result),
            Self::Settings(settings) => json(&serde_json::to_value(settings)?),
            Self::Network(network) => json(network),
        }
    }
}
