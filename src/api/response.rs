//! Typed response envelopes and operation results. Decode a successful result
//! using its request's operation (`Reply::decode`); there is no op on a reply.
use super::{objects::*, scalar::*};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bool<const V: bool>;
impl<const V: bool> Serialize for Bool<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(V)
    }
}
impl<'de, const V: bool> Deserialize<'de> for Bool<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if bool::deserialize(d)? == V {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "ok discriminator disagrees with envelope",
            ))
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Success<T> {
    pub id: RequestId,
    pub ok: Bool<true>,
    pub result: T,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseId {
    Valid(RequestId),
    Invalid(()),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    // The one nullable output field: a malformed/reused request ID has no ID.
    pub id: ResponseId,
    pub ok: Bool<false>,
    pub error: Error,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response<T> {
    Success(Success<T>),
    Failure(Failure),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Map<String, Value>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidMessage,
    HelloRequired,
    ProtoUnsupported,
    UnknownOp,
    InvalidArgs,
    NotFound,
    Ambiguous,
    Safety,
    InvalidState,
    Unsupported,
    EngineUnavailable,
    AlreadyRunning,
    Upstream,
    Timeout,
    Busy,
    Internal,
}
impl ErrorCode {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::InvalidMessage
            | Self::HelloRequired
            | Self::ProtoUnsupported
            | Self::UnknownOp
            | Self::InvalidArgs
            | Self::Unsupported => 2,
            Self::Safety => 3,
            Self::EngineUnavailable | Self::Busy => 4,
            Self::Upstream => 5,
            Self::NotFound => 6,
            Self::Ambiguous => 7,
            Self::Timeout => 8,
            Self::InvalidState | Self::AlreadyRunning | Self::Internal => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmptyResult {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingResult {
    pub state: PendingState,
    #[serde(rename = "ref")]
    pub reference: MutationRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoneResult {
    pub state: DoneState,
}

/// Pending is correlated by construction, including when flattened into add/remove.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MutationResult {
    Done,
    Pending {
        #[serde(rename = "ref")]
        reference: MutationRef,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkResult {
    pub state: NetworkState,
    pub interfaces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListResult {
    pub torrents: Vec<Torrent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetResult {
    pub torrent: TorrentDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddResult {
    #[serde(flatten)]
    pub status: MutationResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Hash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferred: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub hash: Hash,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartResult {
    pub state: DoneState,
    pub started: Vec<Hash>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopResult {
    pub state: DoneState,
    pub stopped: Vec<Hash>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveResult {
    #[serde(flatten)]
    pub status: MutationResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_until: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilesResult {
    pub files: Vec<File>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenTargetResult {
    pub path: String,
    pub kind: TargetKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeResult {
    pub sub: u64,
    pub topics: Vec<Topic>,
}

/// Each variant has the exact result type for an operation, even when two
/// operations share the same wire shape. The request ID supplies the context.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    TorrentsRecheck(RecheckReply),
    AppSettingsGet(Settings),
    AppSettingsSet(SettingsSetResult),
    ImdbLookup(ImdbLookupResult),
    Hello(HelloResult),
    AppInfo(AppInfo),
    AppClient(EmptyResult),
    AppShutdown(PendingResult),
    AppRestart(PendingResult),
    AppNetwork(NetworkResult),
    TransferInfo(TransferInfo),
    TorrentsList(ListResult),
    TorrentsGet(GetResult),
    TorrentsAdd(AddResult),
    TorrentsStart(StartResult),
    TorrentsStop(StopResult),
    TorrentsRemove(RemoveResult),
    TorrentsFiles(FilesResult),
    TorrentsSetFiles(DoneResult),
    TorrentsSetSeed(DoneResult),
    TorrentsSetUploadLimit(MutationResult),
    TorrentsOpenTarget(OpenTargetResult),
    Subscribe(SubscribeResult),
    Unsubscribe(EmptyResult),
}
impl Reply {
    pub fn decode(op: &str, result: Value) -> Result<Self, serde_json::Error> {
        let reply = match op {
            "torrents.recheck" => serde_json::from_value(result).map(Self::TorrentsRecheck),
            "app.settings.get" => serde_json::from_value(result).map(Self::AppSettingsGet),
            "app.settings.set" => serde_json::from_value(result).map(Self::AppSettingsSet),
            "imdb.lookup" => serde_json::from_value(result).map(Self::ImdbLookup),
            "hello" => serde_json::from_value(result).map(Self::Hello),
            "app.info" => serde_json::from_value(result).map(Self::AppInfo),
            "app.client" => serde_json::from_value(result).map(Self::AppClient),
            "app.shutdown" => serde_json::from_value(result).map(Self::AppShutdown),
            "app.restart" => serde_json::from_value(result).map(Self::AppRestart),
            "app.network" => serde_json::from_value(result).map(Self::AppNetwork),
            "transfer.info" => serde_json::from_value(result).map(Self::TransferInfo),
            "torrents.list" => serde_json::from_value(result).map(Self::TorrentsList),
            "torrents.get" => serde_json::from_value(result).map(Self::TorrentsGet),
            "torrents.add" => serde_json::from_value(result).map(Self::TorrentsAdd),
            "torrents.start" => serde_json::from_value(result).map(Self::TorrentsStart),
            "torrents.stop" => serde_json::from_value(result).map(Self::TorrentsStop),
            "torrents.remove" => serde_json::from_value(result).map(Self::TorrentsRemove),
            "torrents.files" => serde_json::from_value(result).map(Self::TorrentsFiles),
            "torrents.set_files" => serde_json::from_value(result).map(Self::TorrentsSetFiles),
            "torrents.set_seed" => serde_json::from_value(result).map(Self::TorrentsSetSeed),
            "torrents.set_upload_limit" => {
                serde_json::from_value(result).map(Self::TorrentsSetUploadLimit)
            }
            "torrents.open_target" => serde_json::from_value(result).map(Self::TorrentsOpenTarget),
            "subscribe" => serde_json::from_value(result).map(Self::Subscribe),
            "unsubscribe" => serde_json::from_value(result).map(Self::Unsubscribe),
            _ => Err(<serde_json::Error as serde::de::Error>::custom(
                "unknown P1 operation",
            )),
        }?;
        super::validation::Validate::validate(&reply)
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        Ok(reply)
    }
    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImdbLookupResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<ImdbCard>,
}

/// Durable settings acknowledgement; settings never produce mutation.result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSetResult {
    pub applied: Vec<String>,
    pub restart: bool,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecheckReply {
    Pending(PendingResult),
    Complete(RecheckResult),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecheckResult {
    pub rechecked: Vec<Hash>,
    pub failed: Vec<RecheckFailure>,
    pub have_bytes: std::collections::BTreeMap<Hash, u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecheckFailure { pub hash: Hash, pub message: String }
