pub use super::super::queue_policy::Direction as QueueDirection;
use super::*;

/// Explicit torrent hashes; every one must name a saved record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSelection(pub Vec<String>);

impl HashSelection {
    pub(in crate::engine) fn resolve(&self, ledger: &Ledger) -> Result<Vec<String>> {
        let hashes = self.0.clone();
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
    /// Explicit upload opt-in, including after completion.
    pub seed: bool,
    pub stopped: bool,
    pub category: String,
    pub tags: String,
    pub origin: Option<String>,
}

#[derive(Debug, Clone)]
pub enum EngineOp {
    TransferInfo,
    Info,
    Get {
        hash: String,
    },
    Files {
        hash: String,
    },
    Peers { hash: String },
    Rename {hash:String,name:String},
    EditTracker { hash: String, url: String, add: bool },
    OpenTarget {
        hash: String,
    },
    Move { hashes: Vec<String>, save_path: PathBuf },
    Trackers { hash: String },
    Reannounce { hashes: Vec<String> },
    RecheckBatch { hashes: Vec<String> },
    ForceStart { hashes: HashSelection, on: bool },
    QueueMove { hash: String, to: super::super::queue_policy::Direction },
    SetFiles {
        hash: String,
        include: Vec<usize>,
    },
    SetSeed {
        hashes: HashSelection,
        on: bool,
    },
    Stage { options: AddRequest, files: Option<Vec<usize>> },
    Commit { hash: String, files: Option<Vec<usize>>, seed: Option<bool>, save_path: Option<PathBuf> },
    Discard { hash: String },
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
    RemoveKeep { hashes: HashSelection },
    RestoreUndo { hash: String },
    SetDownloadLimit { hashes: HashSelection, bps: Option<u64> },
    SetUploadLimit {
        hashes: HashSelection,
        limit: i64,
    },
}

impl EngineOp {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Rename {name,..} if !crate::engine::rename::valid_name(name)=>bail!("invalid display name"),
            Self::SetDownloadLimit{bps:Some(bps),..} if !(16384..=u32::MAX as u64).contains(bps)=>bail!("download cap must be between 16 KiB/s and the engine maximum"),
            Self::SetUploadLimit { limit, .. } if *limit < 0 => {
                bail!("upload limit must be zero (unlimited) or a positive byte rate")
            }
            Self::Add(options) | Self::AddMagnet(options) | Self::AddSelected {options, ..} | Self::Stage {options, ..} => {
                if options.upload_limit < 0 {
                    bail!("upload limit cannot be negative");
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
            Self::ForceStart { hashes, .. }
            | Self::Start { hashes, .. }
            | Self::Stop { hashes }
            | Self::Delete { hashes, .. }
            | Self::SetUploadLimit { hashes, .. }
            | Self::SetDownloadLimit { hashes, .. } => Some(hashes),
            _ => None,
        }
    }
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
    pub original_name:String,
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
            original_name:torrent.original_name().to_owned(),
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
    Move(Arc<super::super::moving::Batch>),
    Trackers(Vec<librqbit::TrackerStat>),
    RemovedUndo { undo_until: u64 },
    QueuePosition(u64),
    Recheck(Arc<super::super::recheck::Batch>),
    Acknowledged,
    Torrents(Vec<super::super::view::TorrentView>),
    Detail(TorrentDetails),
    Added(Vec<AddedTorrent>),
    Files(Vec<TorrentFile>),
    Peers(Vec<super::super::peers::Peer>),
    OpenTarget(PathBuf),
    Transfer(TransferInfo),
    Start(crate::qbt::StartResult),
}
