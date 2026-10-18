use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashSelection {
    All,
    Hashes(Vec<String>),
}

impl HashSelection {
    pub(super) fn resolve(&self, ledger: &Ledger) -> Result<Vec<String>> {
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
    Files {
        hash: String,
    },
    Add(AddRequest),
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
            Self::Add(options) => {
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

#[derive(Debug)]
pub enum EngineResult {
    Acknowledged,
    Version(String),
    Torrents(Vec<super::super::view::TorrentView>),
    Files(Vec<TorrentFile>),
    Transfer(TransferInfo),
    Queue(QueueInfo),
    Start(crate::qbt::StartResult),
    Settings(EngineSettings),
    Network(crate::network_app::NetworkState),
}
impl EngineResult {
    pub fn to_legacy_json(&self) -> Result<String> {
        match self {
            Self::Acknowledged => Ok("Ok.".into()),
            Self::Version(version) => Ok(version.clone()),
            Self::Torrents(rows) => Ok(format!(
                "[{}]",
                rows.iter()
                    .map(|row| row.to_legacy_json())
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            )),
            Self::Files(files) => json(files),
            Self::Transfer(info) => json(info),
            Self::Queue(info) => json(info),
            Self::Start(result) => json(result),
            Self::Settings(settings) => json(settings),
            Self::Network(network) => json(network),
        }
    }
}
