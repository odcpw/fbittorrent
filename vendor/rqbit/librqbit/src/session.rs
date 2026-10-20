//! Session configuration, shared state and public accessors.
//! Lifecycle, peer networking, admission and control live in private modules.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    io::Read,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    ApiError, CreateTorrentOptions, FileInfos, ManagedTorrent, ManagedTorrentShared,
    api::TorrentIdOrHash,
    api_error::WithStatus,
    bitv_factory::{BitVFactory, NonPersistentBitVFactory},
    create_torrent,
    create_torrent_file::CreateTorrentResult,
    dht_utils::{ReadMetainfoResult, read_metainfo_from_peer_receiver},
    ip_ranges::IpRanges,
    limits::{Limits, LimitsConfig},
    listen::{Accept, ListenerOptions},
    merge_streams::merge_streams,
    peer_connection::PeerConnectionOptions,
    read_buf::ReadBuf,
    session_stats::SessionStats,
    spawn_utils::BlockingSpawner,
    storage::{
        BoxStorageFactory, StorageFactoryExt, TorrentStorage, filesystem::FilesystemStorageFactory,
    },
    stream_connect::{
        ConnectionKind, ConnectionOptions, StreamConnector, StreamConnectorArgs,
    },
    torrent_state::{
        ManagedTorrentHandle, ManagedTorrentLocked, ManagedTorrentOptions, ManagedTorrentState,
        TorrentMetadata, TorrentStateLive, initializing::TorrentStateInitializing,
    },
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite, PeerStream},
};
use anyhow::{Context, bail};
use arc_swap::ArcSwapOption;
use bencode::bencode_serialize_to_writer;
use buffers::ByteBufOwned;
use bytes::Bytes;
use clone_to_owned::CloneToOwned;
use dht::{Dht, DhtBuilder, DhtConfig, DhtPersistenceConfig, Id20, PersistentDht, dht_listen_addr};
use futures::{
    FutureExt, Stream, StreamExt, TryFutureExt,
    future::BoxFuture,
    stream::{BoxStream, FuturesUnordered},
};
use http::StatusCode;
use itertools::Itertools;
use librqbit_core::{
    crate_version,
    directories::get_configuration_directory,
    magnet::Magnet,
    peer_id::generate_azereus_style,
    spawn_utils::spawn_with_cancel,
    torrent_metainfo::{TorrentMetaV1Owned, ValidatedTorrentMetaV1Info},
};
use librqbit_lsd::{LocalServiceDiscovery, LocalServiceDiscoveryOptions};
use librqbit_dualstack_sockets::BindDevice;
use parking_lot::RwLock;
use peer_binary_protocol::Handshake;
use serde::{Deserialize, Serialize};
use rqbit_runtime::{Cx, ChildRegion, ChildRegionSpec, CancellationToken, DropGuard};
use rqbit_runtime::sync::{Notify, Semaphore};
use rqbit_runtime::http::Client;
use tracing::{Instrument, debug, debug_span, info, trace, warn};
use tracker_comms::{TrackerComms, UdpTrackerClient};

mod add;
mod control;
mod lifecycle;
mod peers;

// Keep the tracing target stable when implementations move into child modules.
const LOG_TARGET: &str = module_path!();

#[cfg(test)]
use add::{derived_output_folder, torrent_file_from_info_bytes};

pub const SUPPORTED_SCHEMES: [&str; 3] = ["http:", "https:", "magnet:"];

pub type TorrentId = usize;

#[derive(Default)]
pub struct SessionDatabase {
    torrents: HashMap<TorrentId, ManagedTorrentHandle>,
}

impl SessionDatabase {
    fn add_torrent(&mut self, torrent: ManagedTorrentHandle, id: TorrentId) {
        self.torrents.insert(id, torrent);
    }
}

pub struct Session {
    // Core state and services
    pub(crate) db: RwLock<SessionDatabase>,
    pub(crate) cx: Cx,
    region: parking_lot::Mutex<Option<ChildRegion>>,
    next_id: AtomicUsize,
    pub(crate) bitv_factory: Arc<dyn BitVFactory>,
    spawner: BlockingSpawner,

    // Network
    peer_id: Id20,
    announce_port: Option<u16>,
    listen_addr: Option<SocketAddr>,
    dht: Option<Dht>,
    pub(crate) connector: Arc<StreamConnector>,
    http_client: Client,
    udp_tracker_client: UdpTrackerClient,
    disable_trackers: bool,

    // Lifecycle management
    cancellation_token: CancellationToken,
    _cancellation_token_drop_guard: DropGuard,

    // Runtime settings
    output_folder: PathBuf,
    peer_opts: PeerConnectionOptions,
    default_storage_factory: Option<BoxStorageFactory>,
    trackers: HashSet<url::Url>,

    lsd: Option<LocalServiceDiscovery>,

    // Limits and throttling
    pub(crate) concurrent_initialize_semaphore: Arc<Semaphore>,
    pub ratelimits: Limits,

    pub blocklist: IpRanges,
    pub allowlist: Option<IpRanges>,

    // Monitoring / tracing / logging
    pub(crate) stats: Arc<SessionStats>,
    root_span: Option<tracing::Span>,

    // Feature flags
    #[cfg(feature = "disable-upload")]
    _disable_upload: bool,
    pub(crate) upload_enabled: Arc<AtomicBool>,
    pub ipv4_only: bool,
    pub peer_limit: Option<usize>,
    client_name_and_version: String,
}

/// Options for adding new torrents to the session.
//
// Serialize/deserialize is for Tauri.
#[derive(Default, Serialize, Deserialize)]
pub struct AddTorrentOptions {
    /// Explicit payload permission. None inherits the session policy; the
    /// limiter remains independent (None/zero rates mean unlimited).
    #[serde(default)]
    pub upload_enabled: Option<bool>,
    /// Start in paused state.
    #[serde(default)]
    pub paused: bool,
    /// A regex to only download files matching it.
    pub only_files_regex: Option<String>,
    /// An explicit list of file IDs to download.
    /// To see the file indices, run with "list_only".
    pub only_files: Option<Vec<usize>>,
    /// Allow writing on top of existing files, including when resuming a torrent.
    /// You probably want to set it, however for safety it's not default.
    ///
    /// Even when all the torrent pieces have been written, `overwrite` needs to
    /// be enabled in order to resume/seed the torrent.
    #[serde(default)]
    pub overwrite: bool,
    /// Only list the files in the torrent without starting it.
    #[serde(default)]
    pub list_only: bool,
    /// The output folder for the torrent. If not set, the session's default one will be used.
    pub output_folder: Option<String>,
    /// Sub-folder within session's default output folder. Will error if "output_folder" if also set.
    /// By default, multi-torrent files are downloaded to a sub-folder.
    pub sub_folder: Option<String>,
    /// Peer connection options, timeouts etc. If not set, session's defaults will be used.
    pub peer_opts: Option<PeerConnectionOptions>,

    /// Force a refresh interval for polling trackers.
    pub force_tracker_interval: Option<Duration>,

    #[serde(default)]
    pub disable_trackers: bool,

    #[serde(default)]
    pub ratelimits: LimitsConfig,

    /// Initial peers to start with. Ignored once private metadata is known:
    /// these addresses carry no proof that the active private tracker returned them.
    pub initial_peers: Option<Vec<SocketAddr>>,

    /// Max concurrent connected peers.
    pub peer_limit: Option<usize>,

    /// This is used to restore the session from serialized state.
    pub preferred_id: Option<usize>,

    #[serde(skip)]
    pub storage_factory: Option<BoxStorageFactory>,

    /// App-validated have snapshot. Never accepted from the upstream wire or
    /// persistence format: the embedding owner must validate its clean marker
    /// and every relevant payload file's identity before supplying these bytes.
    #[serde(skip)]
    pub trusted_have_pieces: Option<Vec<u8>>,
    /// App-owned handle replacement retains observations and force throttling.
    #[serde(skip)]
    pub tracker_stats: Option<Arc<tracker_comms::TrackerStats>>,

    /// Additional trackers for public torrents. Ignored for private metainfo.
    pub trackers: Option<Vec<String>>,
    /// Explicit user edits; applied to metainfo/magnet URLs before any announce.
    pub tracker_additions: Vec<String>,
    pub tracker_removals: Vec<String>,
}

pub struct ListOnlyResponse {
    pub info_hash: Id20,
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub only_files: Option<Vec<usize>>,
    pub output_folder: PathBuf,
    pub seen_peers: Vec<SocketAddr>,
    pub torrent_bytes: Bytes,
}

#[allow(clippy::large_enum_variant)]
pub enum AddTorrentResponse {
    AlreadyManaged(TorrentId, ManagedTorrentHandle),
    ListOnly(ListOnlyResponse),
    Added(TorrentId, ManagedTorrentHandle),
}

impl AddTorrentResponse {
    pub fn into_handle(self) -> Option<ManagedTorrentHandle> {
        match self {
            Self::AlreadyManaged(_, handle) => Some(handle),
            Self::ListOnly(_) => None,
            Self::Added(_, handle) => Some(handle),
        }
    }
}

pub fn read_local_file_including_stdin(filename: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if filename == "-" {
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("error reading stdin")?;
    } else {
        std::fs::File::open(filename)
            .context("error opening")?
            .read_to_end(&mut buf)
            .context("error reading")?;
    }
    Ok(buf)
}

pub enum AddTorrent<'a> {
    Url(Cow<'a, str>),
    TorrentFileBytes(Bytes),
}

impl<'a> AddTorrent<'a> {
    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_cli_argument(path: &'a str) -> anyhow::Result<Self> {
        if SUPPORTED_SCHEMES.iter().any(|s| path.starts_with(s)) {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        if path.len() == 40 && !Path::new(path).exists() && Magnet::parse(path).is_ok() {
            return Ok(Self::Url(Cow::Borrowed(path)));
        }
        Self::from_local_filename(path)
    }

    pub fn from_url(url: impl Into<Cow<'a, str>>) -> Self {
        Self::Url(url.into())
    }

    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self::TorrentFileBytes(bytes.into())
    }

    // Don't call this from HTTP API.
    #[inline(never)]
    pub fn from_local_filename(filename: &str) -> anyhow::Result<Self> {
        let file = read_local_file_including_stdin(filename)
            .with_context(|| format!("error reading local file {filename:?}"))?;
        Ok(Self::TorrentFileBytes(file.into()))
    }

    pub fn into_bytes(self) -> Bytes {
        match self {
            Self::Url(s) => s.into_owned().into_bytes().into(),
            Self::TorrentFileBytes(b) => b,
        }
    }
}

pub enum SessionPersistenceConfig {
    /// The filename for persistence. By default uses an OS-specific folder.
    Json { folder: Option<PathBuf> },
    #[cfg(any())]
    Postgres { connection_string: String },
}

impl SessionPersistenceConfig {
    pub fn default_json_persistence_folder() -> anyhow::Result<PathBuf> {
        let dir = get_configuration_directory("session")?;
        Ok(dir.data_dir().to_owned())
    }
}

/// Configuration for the DHT subsystem.
/// Set to `None` in `SessionOptions::dht` to disable DHT entirely.
pub struct DhtSessionConfig {
    /// Bootstrap nodes (host:port or ip:port). Uses built-in defaults if None.
    pub bootstrap_addrs: Option<Vec<String>>,
    /// The DHT listen port. Priority: this explicit port -> persisted port
    /// (when persistence is enabled) -> random. The bind IP is derived from
    /// `SessionOptions::ipv4_only` (`0.0.0.0` if true, `[::]` otherwise).
    /// Use `SessionOptions::bind_device_name` to scope the bind to a specific
    /// network interface.
    pub port: Option<u16>,
    /// Persistence behavior. If None, persistence is disabled.
    pub persistence: Option<DhtPersistenceConfig>,
}

impl Default for DhtSessionConfig {
    fn default() -> Self {
        Self {
            bootstrap_addrs: None,
            port: None,
            persistence: Some(DhtPersistenceConfig::default()),
        }
    }
}

pub struct SessionOptions {
    /// Payload upload permission for torrents without an explicit override.
    pub upload_enabled: bool,
    /// Encryption policy for incoming peers, outgoing peers and magnet metadata.
    pub peer_encryption: crate::PeerEncryption,

    /// DHT configuration. Set to None to disable DHT entirely.
    /// Defaults to DHT enabled with persistence.
    pub dht: Option<DhtSessionConfig>,

    /// Required Linux network interface for peers, listeners, DHT, UDP/HTTP
    /// trackers, torrent URL/IP-list fetches, DNS and LSD. Missing/down/replaced
    /// devices or denied SO_BINDTODEVICE fail without an ordinary-network retry.
    pub bind_device_name: Option<String>,

    /// Disable tracker communication
    pub disable_trackers: bool,

    /// Enable fastresume, to restore state quickly after restart.
    pub fastresume: bool,

    /// Turn on to dump session contents into a file periodically, so that on next start
    /// all remembered torrents will continue where they left off.
    pub persistence: Option<SessionPersistenceConfig>,

    /// The peer ID to use. If not specified, a random one will be generated.
    pub peer_id: Option<Id20>,

    /// Options for listening on TCP. uTP is unavailable in this integration.
    pub listen: Option<ListenerOptions>,
    /// Options for connecting to peers (for outgiong connections).
    pub connect: Option<ConnectionOptions>,

    pub default_storage_factory: Option<BoxStorageFactory>,

    pub cancellation_token: Option<CancellationToken>,

    /// how many concurrent torrent initializations can happen
    pub concurrent_init_limit: Option<usize>,

    /// Maximum concurrent owned blocking operations on the native runtime.
    pub runtime_worker_threads: Option<usize>,

    /// the root span to use. If not set will be None.
    pub root_span: Option<tracing::Span>,

    pub ratelimits: LimitsConfig,

    pub blocklist_url: Option<String>,
    pub allowlist_url: Option<String>,

    // The list of tracker URLs to always use for each torrent.
    pub trackers: HashSet<url::Url>,

    /// Default peer limit per torrent.
    pub peer_limit: Option<usize>,

    #[cfg(feature = "disable-upload")]
    pub disable_upload: bool,

    /// Disable LSD multicast
    pub disable_local_service_discovery: bool,

    /// Force IPv4 only.
    pub ipv4_only: bool,

    /// Override the client name and version used in User-Agent headers and
    /// peer extended handshakes. Defaults to "rqbit X.Y.Z".
    pub client_name_and_version: Option<String>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            upload_enabled: true,
            peer_encryption: crate::PeerEncryption::default(),
            dht: Some(DhtSessionConfig::default()),
            bind_device_name: None,
            disable_trackers: false,
            fastresume: false,
            persistence: None,
            peer_id: None,
            listen: None,
            connect: None,
            default_storage_factory: None,
            cancellation_token: None,
            concurrent_init_limit: None,
            runtime_worker_threads: None,
            root_span: None,
            ratelimits: LimitsConfig::default(),
            blocklist_url: None,
            allowlist_url: None,
            trackers: HashSet::new(),
            peer_limit: None,
            #[cfg(feature = "disable-upload")]
            disable_upload: false,
            disable_local_service_discovery: false,
            ipv4_only: false,
            client_name_and_version: None,
        }
    }
}

pub(crate) struct CheckedIncomingConnection {
    pub encrypted: bool,
    pub kind: ConnectionKind,
    pub addr: SocketAddr,
    pub reader: BoxAsyncReadVectored,
    pub writer: BoxAsyncWrite,
    pub read_buf: ReadBuf,
    pub handshake: Handshake,
}

impl Session {
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    pub fn client_name_and_version(&self) -> &str {
        &self.client_name_and_version
    }

    pub fn get_dht(&self) -> Option<&Dht> {
        self.dht.as_ref()
    }

    /// Spawn a task in the context of the session.
    #[track_caller]
    pub fn spawn<F, Fut>(&self, span: tracing::Span, name: impl Into<Cow<'static, str>>, make_fut: F)
    where F: FnOnce(Cx) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        spawn_with_cancel(&self.cx, span, name, self.cancellation_token.clone(), make_fut);
    }

    pub(crate) fn rs(&self) -> Option<tracing::Id> {
        self.root_span.as_ref().and_then(|s| s.id())
    }

    pub fn upload_enabled(&self) -> bool { self.upload_enabled.load(Ordering::Acquire) }

    /// Refresh peers outside the session database lock: live state has its own lock.
    pub fn set_upload_enabled(&self, enabled: bool) {
        if self.upload_enabled.swap(enabled, Ordering::AcqRel) != enabled {
            let handles = self.with_torrents(|items| items.map(|(_, h)| h.clone()).collect::<Vec<_>>());
            for handle in handles { handle.refresh_upload_policy(); }
        }
    }

    /// Run a callback given the currently managed torrents.
    pub fn with_torrents<R>(
        &self,
        callback: impl Fn(&mut dyn Iterator<Item = (TorrentId, &ManagedTorrentHandle)>) -> R,
    ) -> R {
        callback(&mut self.db.read().torrents.iter().map(|(id, t)| (*id, t)))
    }

    pub fn get(&self, id: TorrentIdOrHash) -> Option<ManagedTorrentHandle> {
        match id {
            TorrentIdOrHash::Id(id) => self.db.read().torrents.get(&id).cloned(),
            TorrentIdOrHash::Hash(id) => self.db.read().torrents.iter().find_map(|(_, v)| {
                if v.info_hash() == id {
                    Some(v.clone())
                } else {
                    None
                }
            }),
        }
    }

    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.listen_addr
    }

    pub fn announce_port(&self) -> Option<u16> {
        self.announce_port
    }
}

pub(crate) struct ResolveMagnetResult {
    pub metadata: TorrentMetadata,
    pub seen_peers: Vec<SocketAddr>,
}

#[cfg(test)]
mod derived_folder_tests {
    use super::derived_output_folder;
    use std::path::{Path, PathBuf};

    #[test]
    fn only_the_session_derived_torrent_folder_counts_as_created() {
        let session = Path::new("/data/downloads");
        let name = || Some(PathBuf::from("Show.S01"));
        // Multi-file torrent without a save path: rqbit's own subfolder.
        assert!(derived_output_folder(session, &session.join("Show.S01"), name()));
        // Single-file torrents have no subfolder; their folder is the user's.
        assert!(!derived_output_folder(session, Path::new("/home/me/Videos"), None));
        assert!(!derived_output_folder(session, session, None));
        // A user save path, even for a multi-file torrent, is never ours.
        assert!(!derived_output_folder(session, Path::new("/home/me/Videos"), name()));
        assert!(!derived_output_folder(session, session, Some(PathBuf::new())));
    }
}

#[cfg(test)]
mod tests {
    use buffers::ByteBuf;
    use itertools::Itertools;
    use librqbit_core::torrent_metainfo::{TorrentMetaV1, torrent_from_bytes};

    use super::torrent_file_from_info_bytes;

    #[test]
    fn test_torrent_file_from_info_and_bytes() {
        fn get_trackers(info: &TorrentMetaV1<ByteBuf>) -> Vec<url::Url> {
            info.iter_announce()
                .filter_map(|t| std::str::from_utf8(t.as_ref()).ok().map(|t| t.to_owned()))
                .filter_map(|t| t.parse().ok())
                .collect_vec()
        }

        let orig_full_torrent =
            include_bytes!("../resources/ubuntu-21.04-desktop-amd64.iso.torrent");
        let parsed = torrent_from_bytes(&orig_full_torrent[..]).unwrap();
        let parsed_trackers = get_trackers(&parsed);

        let generated_torrent =
            torrent_file_from_info_bytes(parsed.info.raw_bytes.as_ref(), &parsed_trackers).unwrap();
        let generated_parsed = torrent_from_bytes(generated_torrent.as_ref()).unwrap();
        assert_eq!(parsed.info_hash, generated_parsed.info_hash);
        assert_eq!(parsed.info, generated_parsed.info);
        assert_eq!(parsed_trackers, get_trackers(&generated_parsed));
    }
}
