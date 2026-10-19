//! Strict client input. Runtime policies (in-flight IDs, hello deadlines,
//! capabilities, path opening and mutation refs) belong to the server.
use super::{scalar::*, search::*};
use super::rss::*;
use super::shows::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedRequest")]
pub struct Request {
    pub id: RequestId,
    #[serde(flatten)]
    pub call: Call,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<MutationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Instance>,
}

#[derive(Deserialize)]
struct UncheckedRequest {
    id: RequestId,
    #[serde(flatten)]
    call: Call,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    reference: Option<MutationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance: Option<Instance>,
}
impl TryFrom<UncheckedRequest> for Request {
    type Error = super::validation::Violation;
    fn try_from(raw: UncheckedRequest) -> Result<Self, Self::Error> {
        let request = Self {
            id: raw.id,
            call: raw.call,
            reference: raw.reference,
            instance: raw.instance,
        };
        super::validation::Validate::validate(&request)?;
        Ok(request)
    }
}

/// Omitted means unchanged; explicit null means unlimited. `Option<Option<T>>`
/// cannot express this with serde's default null handling.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Update<T> {
    #[default]
    Unset,
    Clear,
    Set(T),
}
impl<T> Update<T> {
    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
}
impl<T: Serialize> Serialize for Update<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(v) => v.serialize(s),
            Self::Clear => s.serialize_none(),
            Self::Unset => Err(serde::ser::Error::custom("omit an unset update")),
        }
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Update<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Option::<T>::deserialize(d)?.map_or(Self::Clear, Self::Set))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Hashes(Vec<Hash>);
impl Hashes {
    pub fn new(hashes: Vec<Hash>) -> Result<Self, super::validation::Violation> {
        super::validation::count(hashes.len(), super::MAX_HASHES, "hashes")?;
        Ok(Self(hashes))
    }
    pub fn as_slice(&self) -> &[Hash] {
        &self.0
    }
}
impl<'de> Deserialize<'de> for Hashes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(Vec::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddSource {
    Magnet(String),
    Url(String),
    Path(String),
    Restore(Hash),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetLimitsArgs {
    #[serde(default, skip_serializing_if = "Update::is_unset")]
    pub download_bps: Update<u32>,
    #[serde(default, skip_serializing_if = "Update::is_unset")]
    pub upload_bps: Update<u32>,
    #[serde(default, skip_serializing_if = "Update::is_unset")]
    pub alt_download_bps: Update<u32>,
    #[serde(default, skip_serializing_if = "Update::is_unset")]
    pub alt_upload_bps: Update<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetAltArgs { pub on: bool }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    pub name: ClientName,
    pub version: String,
    pub kind: ClientKind,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<LogLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Hash>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloArgs {
    pub proto: u64,
    pub client: Client,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppClientArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glance: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartArgs {
    pub scope: RestartScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub states: Option<Vec<TorrentState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashArgs {
    pub hash: Hash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddArgs {
    pub source: AddSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartArgs {
    pub hashes: Hashes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashesArgs {
    pub hashes: Hashes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveArgs {
    pub hashes: Hashes,
    pub delete_files: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetFilesArgs {
    pub hash: Hash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSeedArgs {
    pub hashes: Hashes,
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenTargetArgs {
    pub hash: Hash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeArgs {
    #[serde(deserialize_with = "super::validation::deserialize_topics")]
    pub topics: Vec<Topic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsubscribeArgs {
    pub sub: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadLimitArgs {
    pub hashes: Hashes,
    #[serde(
        default,
        skip_serializing_if = "Update::is_unset",
        deserialize_with = "super::validation::deserialize_upload_update"
    )]
    pub bps: Update<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImdbLookupArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Call {
    #[serde(rename="torrents.trackers")]
    TorrentsTrackers { args: HashArgs },
    #[serde(rename="torrents.reannounce")]
    TorrentsReannounce { args: HashesArgs },
    #[serde(rename="torrents.force_start")]
    TorrentsForceStart { args: SetSeedArgs },
    #[serde(rename="torrents.queue_move")]
    TorrentsQueueMove { args: QueueMoveArgs },
    #[serde(rename = "search.start")]
    SearchStart {
        args: SearchStartArgs,
    },
    #[serde(rename = "search.stop")]
    SearchStop {
        args: SearchIdArgs,
    },
    #[serde(rename = "search.providers")]
    SearchProviders {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "search.set_provider")]
    SearchSetProvider {
        args: SetProviderArgs,
    },
    #[serde(rename = "search.install")]
    SearchInstall {
        args: SearchInstallArgs,
    },
    #[serde(rename = "search.browse_sources")]
    SearchBrowseSources {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "search.browse")]
    SearchBrowse {
        args: SearchBrowseArgs,
    },
    #[serde(rename = "search.grab")]
    SearchGrab {
        args: SearchGrabArgs,
    },

    #[serde(rename = "torrents.recheck")]
    TorrentsRecheck { args: HashesArgs },
    #[serde(rename="shows.list")] ShowsList {#[serde(skip_serializing_if="Option::is_none")] args:Option<EmptyArgs>},
    #[serde(rename="shows.follow")] ShowsFollow {args:ShowFollowArgs},
    #[serde(rename="shows.unfollow")] ShowsUnfollow {args:ShowArgs},
    #[serde(rename="shows.check")] ShowsCheck {#[serde(skip_serializing_if="Option::is_none")] args:Option<ShowCheckArgs>},
    #[serde(rename="shows.releases")] ShowsReleases {args:ShowReleasesArgs},
    #[serde(rename="shows.set_read")] ShowsSetRead {args:ShowReadArgs},
    #[serde(rename="shows.mark_show_read")] ShowsMarkShowRead {args:ShowArgs},
    #[serde(rename="shows.undo_read")] ShowsUndoRead {#[serde(skip_serializing_if="Option::is_none")] args:Option<EmptyArgs>},
    #[serde(rename="shows.grab")] ShowsGrab {args:ShowGrabArgs},
    #[serde(rename="rss.feeds")]
    RssFeeds { #[serde(skip_serializing_if="Option::is_none")] args:Option<EmptyArgs> },
    #[serde(rename="rss.add")] RssAdd { args:FeedAddArgs },
    #[serde(rename="rss.remove")] RssRemove { args:FeedArgs },
    #[serde(rename="rss.refresh")] RssRefresh { #[serde(skip_serializing_if="Option::is_none")] args:Option<FeedRefreshArgs> },
    #[serde(rename="rss.items")] RssItems { args:FeedItemsArgs },
    #[serde(rename="rss.set_read")] RssSetRead { args:FeedReadArgs },
    #[serde(rename="rss.mark_feed_read")] RssMarkFeedRead { args:FeedArgs },
    #[serde(rename="rss.grab")] RssGrab { args:FeedGrabArgs },
    #[serde(rename = "app.clipboard.dismiss")]
    AppClipboardDismiss { args: HashArgs },
    #[serde(rename = "app.log")]
    AppLog { #[serde(skip_serializing_if = "Option::is_none")] args: Option<LogArgs> },
    #[serde(rename = "app.stats")]
    AppStats { #[serde(skip_serializing_if = "Option::is_none")] args: Option<EmptyArgs> },
    #[serde(rename = "app.free_space")]
    AppFreeSpace { args: super::FreeSpaceArgs },
    #[serde(rename = "app.settings.get")]
    AppSettingsGet {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "app.settings.set")]
    AppSettingsSet {
        #[serde(deserialize_with = "deserialize_settings_patch")]
        args: serde_json::Map<String, serde_json::Value>,
    },
    #[serde(rename = "imdb.lookup")]
    ImdbLookup { args: ImdbLookupArgs },
    #[serde(rename = "hello")]
    Hello { args: HelloArgs },
    #[serde(rename = "app.info")]
    AppInfo {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "app.client")]
    AppClient { args: AppClientArgs },
    #[serde(rename = "app.shutdown")]
    AppShutdown { args: ShutdownArgs },
    #[serde(rename = "app.restart")]
    AppRestart { args: RestartArgs },
    #[serde(rename = "app.network")]
    AppNetwork {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "transfer.set_limits")]
    TransferSetLimits { args: SetLimitsArgs },
    #[serde(rename = "transfer.set_alt")]
    TransferSetAlt { args: SetAltArgs },
    #[serde(rename = "transfer.info")]
    TransferInfo {
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<EmptyArgs>,
    },
    #[serde(rename = "torrents.list")]
    TorrentsList { args: ListArgs },
    #[serde(rename = "torrents.get")]
    TorrentsGet { args: HashArgs },
    #[serde(rename = "torrents.add")]
    TorrentsAdd { args: AddArgs },
    #[serde(rename = "torrents.start")]
    TorrentsStart { args: StartArgs },
    #[serde(rename = "torrents.stop")]
    TorrentsStop { args: HashesArgs },
    #[serde(rename = "torrents.remove")]
    TorrentsRemove { args: RemoveArgs },
    #[serde(rename = "torrents.files")]
    TorrentsFiles { args: HashArgs },
    #[serde(rename = "torrents.set_files")]
    TorrentsSetFiles { args: SetFilesArgs },
    #[serde(rename = "torrents.set_seed")]
    TorrentsSetSeed { args: SetSeedArgs },
    #[serde(rename = "torrents.set_upload_limit")]
    TorrentsSetUploadLimit { args: UploadLimitArgs },
    #[serde(rename = "torrents.open_target")]
    TorrentsOpenTarget { args: OpenTargetArgs },
    #[serde(rename = "subscribe")]
    Subscribe { args: SubscribeArgs },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { args: UnsubscribeArgs },
}
impl Call {
    pub fn op(&self) -> &'static str {
        match self {
            Self::TorrentsTrackers {..} => "torrents.trackers",
            Self::TorrentsReannounce {..} => "torrents.reannounce",
            Self::TorrentsForceStart {..} => "torrents.force_start",
            Self::TorrentsQueueMove {..} => "torrents.queue_move",
            Self::SearchStart { .. } => "search.start",
            Self::SearchStop { .. } => "search.stop",
            Self::SearchProviders { .. } => "search.providers",
            Self::SearchSetProvider { .. } => "search.set_provider",
            Self::SearchInstall { .. } => "search.install",
            Self::SearchBrowseSources { .. } => "search.browse_sources",
            Self::SearchBrowse { .. } => "search.browse",
            Self::SearchGrab { .. } => "search.grab",

            Self::TorrentsRecheck { .. } => "torrents.recheck",
            Self::ShowsList {..}=>"shows.list",Self::ShowsFollow {..}=>"shows.follow",Self::ShowsUnfollow {..}=>"shows.unfollow",Self::ShowsCheck {..}=>"shows.check",
            Self::ShowsReleases {..}=>"shows.releases",Self::ShowsSetRead {..}=>"shows.set_read",Self::ShowsMarkShowRead {..}=>"shows.mark_show_read",Self::ShowsUndoRead {..}=>"shows.undo_read",Self::ShowsGrab {..}=>"shows.grab",
            Self::RssFeeds { .. }=>"rss.feeds", Self::RssAdd { .. }=>"rss.add", Self::RssRemove { .. }=>"rss.remove", Self::RssRefresh { .. }=>"rss.refresh",
            Self::RssItems { .. }=>"rss.items", Self::RssSetRead { .. }=>"rss.set_read", Self::RssMarkFeedRead { .. }=>"rss.mark_feed_read", Self::RssGrab { .. }=>"rss.grab",
            Self::AppClipboardDismiss { .. } => "app.clipboard.dismiss",
            Self::AppLog { .. } => "app.log",
            Self::AppStats { .. } => "app.stats",
            Self::AppFreeSpace { .. } => "app.free_space",
            Self::AppSettingsGet { .. } => "app.settings.get",
            Self::AppSettingsSet { .. } => "app.settings.set",
            Self::ImdbLookup { .. } => "imdb.lookup",
            Self::Hello { .. } => "hello",
            Self::AppInfo { .. } => "app.info",
            Self::AppClient { .. } => "app.client",
            Self::AppShutdown { .. } => "app.shutdown",
            Self::AppRestart { .. } => "app.restart",
            Self::AppNetwork { .. } => "app.network",
            Self::TransferSetLimits { .. } => "transfer.set_limits",
            Self::TransferSetAlt { .. } => "transfer.set_alt",
            Self::TransferInfo { .. } => "transfer.info",
            Self::TorrentsList { .. } => "torrents.list",
            Self::TorrentsGet { .. } => "torrents.get",
            Self::TorrentsAdd { .. } => "torrents.add",
            Self::TorrentsStart { .. } => "torrents.start",
            Self::TorrentsStop { .. } => "torrents.stop",
            Self::TorrentsRemove { .. } => "torrents.remove",
            Self::TorrentsFiles { .. } => "torrents.files",
            Self::TorrentsSetFiles { .. } => "torrents.set_files",
            Self::TorrentsSetSeed { .. } => "torrents.set_seed",
            Self::TorrentsSetUploadLimit { .. } => "torrents.set_upload_limit",
            Self::TorrentsOpenTarget { .. } => "torrents.open_target",
            Self::Subscribe { .. } => "subscribe",
            Self::Unsubscribe { .. } => "unsubscribe",
        }
    }
}

pub const SETTINGS_KEYS: &[&str] = &[
    "max_active_downloads", "peer_encryption", "network_interface", "save_path",
    "seed_by_default", "autostart", "idle_exit_secs", "notify", "timeout_secs",
    "handler", "open_command", "clipboard_watch", "reduce_motion", "peer_port",
    "show_quality", "download_limit_bps", "upload_limit_bps", "alt_download_limit_bps",
    "alt_upload_limit_bps", "alt_limits_on",
];

fn deserialize_settings_patch<'de, D: Deserializer<'de>>(d: D) -> Result<serde_json::Map<String, serde_json::Value>, D::Error> {
    let patch = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
    for key in patch.keys() {
        if !SETTINGS_KEYS.contains(&key.as_str()) {
            return Err(serde::de::Error::custom(format!("unknown field `{key}`")));
        }
    }
    Ok(patch)
}

#[derive(Debug,Clone,Copy,PartialEq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum QueueTo { Top, Up, Down, Bottom }
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueMoveArgs { pub hash: Hash, pub to: QueueTo }
