//! Subscription frames and unsolicited events. Whole items are typed; only a
//! partial delta uses arbitrary JSON values (including null for field deletion).
use super::{objects::*, response::Error, scalar::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Item {
    Torrent(Torrent),
    File(File),
    Log(LogEntry),
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
    /// Shape checks that depend on the topic cannot be expressed by serde's
    /// field types alone. Servers and fixture consumers use the same check.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::SnapshotBegin {
                topic,
                reason,
                oldest_seq,
                ..
            } if oldest_seq.is_some()
                && (*reason != SnapshotReason::Resync || *topic != Topic::Events) =>
            {
                Err("oldest_seq is only valid for an events resync")
            }
            Self::SnapshotChunk { topic, items, .. } => {
                if items.len() > super::MAX_CHUNK_ITEMS {
                    return Err("snapshot chunk exceeds 256 items");
                }
                validate_items(topic, items)
            }
            Self::Delta {
                topic,
                added,
                removed,
                ..
            } => {
                if let Some(added) = added {
                    validate_items(topic, added)?;
                }
                if removed.as_ref().is_some_and(|keys| {
                    keys.iter().any(|key| match topic {
                        Topic::Torrents => serde_json::from_value::<Hash>(key.clone()).is_err(),
                        Topic::Files(_) | Topic::Log | Topic::Events => key.as_u64().is_none(),
                        _ => true,
                    })
                }) {
                    return Err("removed key does not match topic");
                }
                Ok(())
            }
            Self::State { topic, data, .. } => match (topic, data) {
                (Topic::Summary, StateData::Summary(_))
                | (Topic::Network, StateData::Network(_))
                | (Topic::Settings, StateData::Settings(_)) => Ok(()),
                _ => Err("state data does not match topic"),
            },
            Self::MutationResult { outcome, error, .. } => match (outcome, error) {
                (MutationOutcome::Done, Some(_)) => Err("done mutation cannot carry an error"),
                (MutationOutcome::Failed, None) => Err("failed mutation requires an error"),
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }
}
fn validate_items(topic: &Topic, items: &[Item]) -> Result<(), &'static str> {
    for item in items {
        match (topic, item) {
            (Topic::Torrents, Item::Torrent(_))
            | (Topic::Files(_), Item::File(_))
            | (Topic::Log, Item::Log(_)) => {}
            (Topic::Events, Item::Event(event))
                if matches!(
                    event.as_ref(),
                    Event::EngineChanged { .. }
                        | Event::TorrentAdded { .. }
                        | Event::TorrentCompleted { .. }
                        | Event::TorrentPlayable { .. }
                        | Event::TorrentError { .. }
                        | Event::MutationResult { .. }
                        | Event::SearchDone { .. }
                        | Event::ShowsChecked { .. }
                        | Event::ClipboardMagnet { .. }
                ) =>
            {
                event.validate()?;
            }
            _ => return Err("snapshot item does not match topic"),
        }
    }
    Ok(())
}
