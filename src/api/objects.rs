//! Full wire objects: unknown optional values are omitted, never sent as null.
use super::scalar::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub version: String,
    pub build: String,
    pub pid: u64,
    pub launcher: Launcher,
    pub log: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireLimits {
    pub line_in: u64,
    pub line_out: u64,
    pub inflight: u64,
    pub subs: u64,
    pub hashes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub idle_exit_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub proto: u64,
    pub server: Server,
    pub engine: String,
    pub instance: Instance,
    pub engine_epoch: u64,
    pub state: DaemonState,
    pub caps: Vec<String>,
    pub limits: WireLimits,
    pub policy: Policy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientCounts {
    pub cli: u64,
    pub agent: u64,
    pub tui: u64,
    pub bar: u64,
    pub test: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscriptions {
    pub total: u64,
    pub by_topic: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub proto: u64,
    pub caps: Vec<String>,
    pub limits: WireLimits,
    pub policy: Policy,
    pub server: Server,
    pub engine: String,
    pub instance: Instance,
    pub engine_epoch: u64,
    pub state: DaemonState,
    pub uptime_s: u64,
    pub upgrade_pending: bool,
    pub clients: ClientCounts,
    pub subscriptions: Subscriptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferInfo {
    pub session_downloaded: u64,
    pub session_uploaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dl_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ul_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playable {
    pub file: u64,
    pub ready: bool,
    pub prefix_bytes: u64,
    pub acked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stall_free: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffered_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finishes_in_s: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Torrent {
    pub hash: Hash,
    pub name: String,
    pub state: TorrentState,
    pub intent: Intent,
    pub staged: bool,
    pub checked: bool,
    pub seed: bool,
    pub upload_policy: UploadPolicy,
    pub save_path: String,
    pub added_at: u64,
    pub force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_limit_bps: Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub download_limit_bps: Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub effective_download_limit_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_pos: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playable: Option<Playable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub index: u64,
    pub path: String,
    pub size: u64,
    pub progress: f64,
    pub included: bool,
    pub sort_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    pub total: u64,
    pub downloading: u64,
    pub stalled: u64,
    pub metadata: u64,
    pub checking: u64,
    pub queued: u64,
    pub paused: u64,
    pub done: u64,
    pub seeding: u64,
    pub waiting_vpn: u64,
    pub error: u64,
    pub staged: u64,
    pub ready: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_download_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_upload_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkState {
    pub ready: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryNetwork {
    pub state: NetworkReadiness,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryDaemon {
    pub state: DaemonState,
    pub busy_reasons: Vec<String>,
    pub upgrade_pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_exit_in_s: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopTorrent {
    pub hash: Hash,
    pub name: String,
    pub state: TorrentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dl_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playable: Option<Playable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiskSpace { pub free_bytes: u64, pub needed_bytes: u64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    pub needed_bytes: u64,
    pub low: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub counts: Counts,
    pub alt: bool,
    pub limits: Limits,
    pub network: SummaryNetwork,
    pub daemon: SummaryDaemon,
    pub top: Vec<TopTorrent>,
    pub disk: Disk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dl_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ul_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub max_active_downloads: u64,
    pub peer_encryption: PeerEncryption,
    pub save_path: String,
    pub seed_by_default: bool,
    pub autostart: bool,
    pub idle_exit_secs: u64,
    pub notify: bool,
    pub timeout_secs: u64,
    pub handler: Handler,
    pub open_command: String,
    pub clipboard_watch: bool,
    /// Wire presence is retained for old transcripts; omission means false.
    #[serde(
        default,
        deserialize_with = "optional_bool",
        skip_serializing_if = "Option::is_none"
    )]
    pub reduce_motion: Option<bool>,
    pub peer_port: u64,
    pub show_quality: String,
    pub alt_limits_on: bool,
    pub restart_class: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_interface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_limit_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_limit_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_download_limit_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_upload_limit_bps: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub seq: u64,
    pub ts: u64,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Hash>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorrentDetail {
    #[serde(skip_serializing_if="Option::is_none")]
    pub original_name:Option<String>,
    #[serde(flatten)]
    pub torrent: Torrent,
    pub private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub piece_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub piece_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

/// A local-file poster is optional; unknown ratings stay absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImdbCard {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub votes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stars: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster_path: Option<String>,
}

impl Settings {
    /// Older daemons omit this preference. Their clients keep hover motion on.
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion.unwrap_or(false)
    }
}

/// An absent optional boolean is allowed; a present null is not a boolean.
fn optional_bool<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
}

/// Full URLs stay on the private socket; clients redact queries when rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum TrackerStatus { Ok, Error, Updating, NotContacted }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tracker {
    pub url:String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub tier:Option<u64>,
    pub status:TrackerStatus,
    #[serde(skip_serializing_if="Option::is_none")]
    pub last_announce_at:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub next_announce_at:Option<u64>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub peers:Option<u32>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub message:Option<String>,
}
