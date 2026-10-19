//! Subscription frames and unsolicited events. Whole items are typed; only a
//! partial delta uses arbitrary JSON values (including null for field deletion).
use super::{objects::*, response::Error, scalar::*};
use super::rss::{Feed,FeedItem};
use super::shows::{Show,Release};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Item {
    Show(Show),Release(Release),
    Feed(Feed), FeedItem(FeedItem),
    Log(LogEntry),
    Torrent(Torrent),
    File(File),
    Search(super::SearchItem),
    Peer(super::Peer),
    Event(Box<Event>),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StateData {
    Summary(Summary),
    Network(NetworkState),
    Settings(Settings),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderWarning {
    pub provider: String,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowWarning {
    pub imdb: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev")]
pub enum Event {
    #[serde(rename = "snapshot.begin")]
    SnapshotBegin {
        sub: u64,
        topic: Topic,
        rev: u64,
        count: u64,
        reason: SnapshotReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        oldest_seq: Option<u64>,
    },
    #[serde(rename = "snapshot.chunk")]
    SnapshotChunk {
        sub: u64,
        topic: Topic,
        rev: u64,
        items: Vec<Item>,
    },
    #[serde(rename = "snapshot.end")]
    SnapshotEnd { sub: u64, topic: Topic, rev: u64 },
    #[serde(rename = "delta")]
    Delta {
        sub: u64,
        topic: Topic,
        rev: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        changed: Option<BTreeMap<String, BTreeMap<String, Value>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        added: Option<Vec<Item>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        removed: Option<Vec<Value>>,
    },
    #[serde(rename = "state")]
    State {
        sub: u64,
        topic: Topic,
        rev: u64,
        data: StateData,
    },
    #[serde(rename = "engine.changed")]
    EngineChanged {
        seq: u64,
        ts: u64,
        epoch: u64,
        state: DaemonState,
        reason: String,
    },
    #[serde(rename = "torrent.added")]
    TorrentAdded {
        seq: u64,
        ts: u64,
        hash: Hash,
        name: String,
        origin: String,
    },
    #[serde(rename = "torrent.completed")]
    TorrentCompleted {
        seq: u64,
        ts: u64,
        hash: Hash,
        name: String,
        open_target: String,
    },
    #[serde(rename = "torrent.playable")]
    TorrentPlayable {
        seq: u64,
        ts: u64,
        hash: Hash,
        name: String,
        file: u64,
        open_target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        buffered_s: Option<u64>,
    },
    #[serde(rename = "torrent.error")]
    TorrentError {
        seq: u64,
        ts: u64,
        hash: Hash,
        name: String,
        message: String,
    },
    #[serde(rename = "mutation.result")]
    MutationResult {
        seq: u64,
        ts: u64,
        #[serde(rename = "ref")]
        reference: MutationRef,
        op: String,
        outcome: MutationOutcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Error>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Map<String, Value>>,
    },
    #[serde(rename = "search.done")]
    SearchDone {
        seq: u64,
        ts: u64,
        search_id: u64,
        total: u64,
        warnings: Vec<ProviderWarning>,
    },
    #[serde(rename = "shows.checked")]
    ShowsChecked {
        seq: u64,
        ts: u64,
        new: u64,
        warnings: Vec<ShowWarning>,
        #[serde(skip_serializing_if = "Option::is_none")]
        imdb: Option<String>,
    },
    #[serde(rename = "clipboard.magnet")]
    ClipboardMagnet {
        seq: u64,
        ts: u64,
        hash: Hash,
        already_added: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat { ts: u64 },
    #[serde(rename = "bye")]
    Bye { reason: ByeReason },
    #[serde(rename = "error")]
    Error { error: Error },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SnapshotBegin { .. } => "snapshot.begin",
            Self::SnapshotChunk { .. } => "snapshot.chunk",
            Self::SnapshotEnd { .. } => "snapshot.end",
            Self::Delta { .. } => "delta",
            Self::State { .. } => "state",
            Self::EngineChanged { .. } => "engine.changed",
            Self::TorrentAdded { .. } => "torrent.added",
            Self::TorrentCompleted { .. } => "torrent.completed",
            Self::TorrentPlayable { .. } => "torrent.playable",
            Self::TorrentError { .. } => "torrent.error",
            Self::MutationResult { .. } => "mutation.result",
            Self::SearchDone { .. } => "search.done",
            Self::ShowsChecked { .. } => "shows.checked",
            Self::ClipboardMagnet { .. } => "clipboard.magnet",
            Self::Heartbeat { .. } => "heartbeat",
            Self::Bye { .. } => "bye",
            Self::Error { .. } => "error",
        }
    }
    /// Value and topic checks are shared with replies and fixture consumers.
    pub fn validate(&self) -> Result<(), super::validation::Violation> {
        super::validation::Validate::validate(self)
    }
    pub fn subscription_revision(&self) -> Option<(u64, u64)> {
        match self {
            Self::SnapshotBegin { sub, rev, .. }
            | Self::SnapshotChunk { sub, rev, .. }
            | Self::SnapshotEnd { sub, rev, .. }
            | Self::Delta { sub, rev, .. }
            | Self::State { sub, rev, .. } => Some((*sub, *rev)),
            _ => None,
        }
    }
}
