//! Typed response envelopes and operation results. Decode a successful result
//! using its request's operation (`Reply::decode`); there is no op on a reply.
use super::{objects::*, scalar::*, search::*};
use super::rss::*;
use super::shows::*;
use super::peers::*;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<DiskSpace>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitsResult { pub limits: Limits }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltResult { pub alt: bool }

/// Each variant has the exact result type for an operation, even when two
/// operations share the same wire shape. The request ID supplies the context.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    TorrentsCommit(DoneResult),
    TorrentsDiscard(DoneResult),
    TorrentsMove(MutationResult),
    TorrentsTrackers(TrackersResult),
    TorrentsReannounce(DoneResult),
    TorrentsForceStart(DoneResult),
    TorrentsQueueMove(QueueMoveResult),
    SearchStart(SearchStarted),
    SearchStop(DoneResult),
    SearchProviders(ProvidersResult),
    SearchSetProvider(DoneResult),
    SearchInstall(InstallReply),
    SearchBrowseSources(BrowseSourcesResult),
    SearchBrowse(SearchItemsResult),
    SearchGrab(AddResult),

    TorrentsRecheck(RecheckReply),
    ShowsList(ShowsResult),ShowsFollow(ShowResult),ShowsSetQuality(ShowResult),ShowsUnfollow(EmptyResult),ShowsCheck(ShowCheckResult),ShowsReleases(ReleasesResult),ShowsSetRead(EmptyResult),ShowsMarkShowRead(CountResult),ShowsUndoRead(UndoReadResult),ShowsGrab(AddResult),
    RssFeeds(FeedsResult), RssAdd(FeedResult), RssRemove(EmptyResult), RssRefresh(MutationResult),
    RssItems(FeedItemsResult), RssSetRead(EmptyResult), RssMarkFeedRead(CountResult), RssGrab(AddResult),
    AppClipboardDismiss(EmptyResult),
    AppLog(LogResult),
    AppStats(StatsResult),
    AppFreeSpace(super::FreeSpaceResult),
    AppSettingsGet(Settings),
    AppSettingsSet(SettingsSetResult),
    ImdbLookup(ImdbLookupResult),
    Hello(HelloResult),
    AppInfo(AppInfo),
    AppClient(EmptyResult),
    AppShutdown(PendingResult),
    AppRestart(PendingResult),
    AppNetwork(NetworkResult),
    TransferSetLimits(LimitsResult),
    TransferSetAlt(AltResult),
    TransferInfo(TransferInfo),
    TorrentsList(ListResult),
    TorrentsGet(GetResult),
    TorrentsAdd(AddResult),
    TorrentsStart(StartResult),
    TorrentsStop(StopResult),
    TorrentsRemove(RemoveResult),
    TorrentsFiles(FilesResult),
    TorrentsPeers(PeersResult),
    TorrentsAddTracker(DoneResult),
    TorrentsRemoveTracker(DoneResult),
    TorrentsSetFiles(DoneResult),
    TorrentsSetSeed(DoneResult),
    TorrentsSetUploadLimit(MutationResult),
    TorrentsSetDownloadLimit(DoneResult),
    TorrentsOpenTarget(OpenTargetResult),
    Subscribe(SubscribeResult),
    Unsubscribe(EmptyResult),
}
impl Reply {
    pub fn decode(op: &str, result: Value) -> Result<Self, serde_json::Error> {
        let reply = match op {
            "torrents.commit"=>serde_json::from_value(result).map(Self::TorrentsCommit),
            "torrents.discard"=>serde_json::from_value(result).map(Self::TorrentsDiscard),
            "torrents.move"=>serde_json::from_value(result).map(Self::TorrentsMove),
            "torrents.trackers" => serde_json::from_value(result).map(Self::TorrentsTrackers),
            "torrents.reannounce" => serde_json::from_value(result).map(Self::TorrentsReannounce),
            "torrents.force_start" => serde_json::from_value(result).map(Self::TorrentsForceStart),
            "torrents.queue_move" => serde_json::from_value(result).map(Self::TorrentsQueueMove),
            "search.start" => serde_json::from_value(result).map(Self::SearchStart),
            "search.stop" => serde_json::from_value(result).map(Self::SearchStop),
            "search.providers" => serde_json::from_value(result).map(Self::SearchProviders),
            "search.set_provider" => serde_json::from_value(result).map(Self::SearchSetProvider),
            "search.install" => serde_json::from_value(result).map(Self::SearchInstall),
            "search.browse_sources" => serde_json::from_value(result).map(Self::SearchBrowseSources),
            "search.browse" => serde_json::from_value(result).map(Self::SearchBrowse),
            "search.grab" => serde_json::from_value(result).map(Self::SearchGrab),

            "torrents.recheck" => serde_json::from_value(result).map(Self::TorrentsRecheck),
            "shows.list"=>serde_json::from_value(result).map(Self::ShowsList),"shows.follow"=>serde_json::from_value(result).map(Self::ShowsFollow),"shows.set_quality"=>serde_json::from_value(result).map(Self::ShowsSetQuality),"shows.unfollow"=>serde_json::from_value(result).map(Self::ShowsUnfollow),
            "shows.check"=>serde_json::from_value(result).map(Self::ShowsCheck),"shows.releases"=>serde_json::from_value(result).map(Self::ShowsReleases),"shows.set_read"=>serde_json::from_value(result).map(Self::ShowsSetRead),
            "shows.mark_show_read"=>serde_json::from_value(result).map(Self::ShowsMarkShowRead),"shows.undo_read"=>serde_json::from_value(result).map(Self::ShowsUndoRead),"shows.grab"=>serde_json::from_value(result).map(Self::ShowsGrab),
            "rss.feeds"=>serde_json::from_value(result).map(Self::RssFeeds), "rss.add"=>serde_json::from_value(result).map(Self::RssAdd),
            "rss.remove"=>serde_json::from_value(result).map(Self::RssRemove), "rss.refresh"=>serde_json::from_value(result).map(Self::RssRefresh),
            "rss.items"=>serde_json::from_value(result).map(Self::RssItems), "rss.set_read"=>serde_json::from_value(result).map(Self::RssSetRead),
            "rss.mark_feed_read"=>serde_json::from_value(result).map(Self::RssMarkFeedRead), "rss.grab"=>serde_json::from_value(result).map(Self::RssGrab),
            "app.clipboard.dismiss" => serde_json::from_value(result).map(Self::AppClipboardDismiss),
            "app.log" => serde_json::from_value(result).map(Self::AppLog),
            "app.stats" => serde_json::from_value(result).map(Self::AppStats),
            "app.free_space" => serde_json::from_value(result).map(Self::AppFreeSpace),
            "app.settings.get" => serde_json::from_value(result).map(Self::AppSettingsGet),
            "app.settings.set" => serde_json::from_value(result).map(Self::AppSettingsSet),
            "imdb.lookup" => serde_json::from_value(result).map(Self::ImdbLookup),
            "hello" => serde_json::from_value(result).map(Self::Hello),
            "app.info" => serde_json::from_value(result).map(Self::AppInfo),
            "app.client" => serde_json::from_value(result).map(Self::AppClient),
            "app.shutdown" => serde_json::from_value(result).map(Self::AppShutdown),
            "app.restart" => serde_json::from_value(result).map(Self::AppRestart),
            "app.network" => serde_json::from_value(result).map(Self::AppNetwork),
            "transfer.set_limits" => serde_json::from_value(result).map(Self::TransferSetLimits),
            "transfer.set_alt" => serde_json::from_value(result).map(Self::TransferSetAlt),
            "transfer.info" => serde_json::from_value(result).map(Self::TransferInfo),
            "torrents.list" => serde_json::from_value(result).map(Self::TorrentsList),
            "torrents.get" => serde_json::from_value(result).map(Self::TorrentsGet),
            "torrents.add" => serde_json::from_value(result).map(Self::TorrentsAdd),
            "torrents.start" => serde_json::from_value(result).map(Self::TorrentsStart),
            "torrents.stop" => serde_json::from_value(result).map(Self::TorrentsStop),
            "torrents.remove" => serde_json::from_value(result).map(Self::TorrentsRemove),
            "torrents.add_tracker" => serde_json::from_value(result).map(Self::TorrentsAddTracker),
            "torrents.remove_tracker" => serde_json::from_value(result).map(Self::TorrentsRemoveTracker),
            "torrents.peers" => serde_json::from_value(result).map(Self::TorrentsPeers),
            "torrents.files" => serde_json::from_value(result).map(Self::TorrentsFiles),
            "torrents.set_files" => serde_json::from_value(result).map(Self::TorrentsSetFiles),
            "torrents.set_seed" => serde_json::from_value(result).map(Self::TorrentsSetSeed),
            "torrents.set_upload_limit" => {
                serde_json::from_value(result).map(Self::TorrentsSetUploadLimit)
            }
            "torrents.set_download_limit" => serde_json::from_value(result).map(Self::TorrentsSetDownloadLimit),
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

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct QueueMoveResult { pub state: DoneState, pub queue_pos: u64 }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogResult { pub entries: Vec<LogEntry>, pub next_seq: u64 }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsResult {
    pub active_clients: ClientCounts, pub connections: ClientCounts,
    #[serde(skip_serializing_if="Option::is_none")] pub history:Option<super::RateHistory>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackersResult { pub trackers:Vec<Tracker> }
