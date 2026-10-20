pub mod initializing;
mod lifecycle;
pub mod live;
pub mod paused;
pub mod stats;
mod streaming;
#[cfg(test)]
mod live_limits_test;
pub mod utils;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use arc_swap::ArcSwapOption;
use buffers::ByteBufOwned;
use bytes::Bytes;
use librqbit_core::hash_id::Id20;
use librqbit_core::lengths::Lengths;

use librqbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info;
pub use live::*;
use parking_lot::RwLock;

use rqbit_runtime::{Cx, ChildRegion};
use rqbit_runtime::sync::Notify;
use rqbit_runtime::time::timeout;

use crate::Session;
use crate::BitV;
use crate::chunk_tracker::ChunkTracker;
use crate::file_info::FileInfo;
use crate::limits::Limits;
use crate::session::TorrentId;
use crate::spawn_utils::BlockingSpawner;
use crate::storage::BoxStorageFactory;
use crate::stream_connect::StreamConnector;
use crate::torrent_state::stats::LiveStats;
use crate::type_aliases::FileInfos;

use initializing::TorrentStateInitializing;

use self::paused::TorrentStatePaused;
pub use self::stats::{TorrentStats, TorrentStatsState};

// Preserve tracing filters across the lifecycle module move.
const LOG_TARGET: &str = module_path!();

// State machine transitions.
//
// - error -> initializing
// - initializing -> paused
// - paused -> live
// - live -> paused
//
// - initializing -> error
// - live -> error
pub enum ManagedTorrentState {
    Initializing(Arc<TorrentStateInitializing>),
    Paused(TorrentStatePaused),
    Live(Arc<TorrentStateLive>),
    Error(anyhow::Error),

    // This is used when swapping between states, outside world should never see it.
    None,
}

impl ManagedTorrentState {
    pub fn name(&self) -> &'static str {
        match self {
            ManagedTorrentState::Initializing(_) => "initializing",
            ManagedTorrentState::Paused(_) => "paused",
            ManagedTorrentState::Live(_) => "live",
            ManagedTorrentState::Error(_) => "error",
            ManagedTorrentState::None => "<invalid: none>",
        }
    }

    fn assert_paused(self) -> TorrentStatePaused {
        match self {
            Self::Paused(paused) => paused,
            _ => panic!("Expected paused state"),
        }
    }

    pub(crate) fn take(&mut self) -> Self {
        std::mem::replace(self, Self::None)
    }
}

pub(crate) struct ManagedTorrentLocked {
    // The torrent might not be in "paused" state technically,
    // but the intention might be for it to stay paused.
    //
    // This should change only on "unpause".
    pub(crate) paused: bool,
    pub(crate) state: ManagedTorrentState,
    pub(crate) only_files: Option<Vec<usize>>,
}

#[derive(Default)]
pub(crate) struct ManagedTorrentOptions {
    // Consumed once. A later error, recheck or rebuilt initializer must never
    // re-use an admission-time trusted snapshot.
    pub trusted_have_pieces: parking_lot::Mutex<Option<Vec<u8>>>,
    pub force_tracker_interval: Option<Duration>,
    pub peer_connect_timeout: Option<Duration>,
    pub peer_encryption: crate::PeerEncryption,
    pub peer_read_write_timeout: Option<Duration>,
    pub allow_overwrite: bool,
    pub output_folder: PathBuf,
    /// output_folder is the session folder joined with this torrent's name.
    pub derived_output_folder: bool,
    // Shared by all run states; pause/resume must not recreate an old cap.
    pub ratelimits: Limits,
    pub initial_peers: Vec<SocketAddr>,
    pub peer_limit: Option<usize>,
    pub session_upload_enabled: Arc<std::sync::atomic::AtomicBool>,
    // 0 inherits the session, 1 refuses payload, 2 explicitly permits it.
    pub upload_override: std::sync::atomic::AtomicU8,
    #[cfg(feature = "disable-upload")]
    pub _disable_upload: bool,
}

impl ManagedTorrentOptions {
    pub fn disable_upload(&self) -> bool {
        #[cfg(feature = "disable-upload")]
        if self._disable_upload { return true; }
        match self.upload_override.load(std::sync::atomic::Ordering::Acquire) {
            1 => true,
            2 => false,
            _ => !self.session_upload_enabled.load(std::sync::atomic::Ordering::Acquire),
        }
    }
}

// Torrent bencodee "info" + some precomputed fields based on it for frequent access.
pub struct TorrentMetadata {
    pub info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
    pub torrent_bytes: Bytes,
    pub info_bytes: Bytes,
    pub file_infos: FileInfos,
}

impl TorrentMetadata {
    pub(crate) fn new(
        info: ValidatedTorrentMetaV1Info<ByteBufOwned>,
        torrent_bytes: Bytes,
        info_bytes: Bytes,
    ) -> anyhow::Result<Self> {
        let file_infos = info
            .iter_file_details_ext()
            .map(|fd| {
                Ok::<_, anyhow::Error>(FileInfo {
                    relative_filename: fd.details.filename.to_pathbuf(),
                    offset_in_torrent: fd.offset,
                    piece_range: fd.pieces,
                    len: fd.details.len,
                    attrs: fd.details.attrs(),
                })
            })
            .collect::<anyhow::Result<Vec<FileInfo>>>()?;

        Ok(Self {
            info,
            torrent_bytes,
            info_bytes,
            file_infos,
        })
    }

    pub fn lengths(&self) -> &Lengths {
        self.info.lengths()
    }
}

/// Common information about torrent shared among all possible states.
///
// The reason it's not inlined into ManagedTorrent is to break the Arc cycle:
// ManagedTorrent contains the current torrent state, which in turn needs access to a bunch
// of stuff, but it shouldn't access the state.
pub struct ManagedTorrentShared {
    pub(crate) cx: Cx,
    pub id: TorrentId,
    pub info_hash: Id20,
    pub(crate) spawner: BlockingSpawner,
    /// Metainfo order (tiers first, then URLs within each tier). In particular,
    /// private first-tracker selection must happen before any set conversion.
    pub trackers: Arc<tracker_comms::TrackerSet>,
    pub tracker_stats: Arc<tracker_comms::TrackerStats>,
    pub peer_id: Id20,
    pub span: tracing::Span,
    pub(crate) options: ManagedTorrentOptions,
    pub(crate) connector: Arc<StreamConnector>,
    pub(crate) storage_factory: BoxStorageFactory,
    pub(crate) session: Weak<Session>,

    // "dn" from magnet link
    pub(crate) magnet_name: Option<String>,

    pub(crate) client_name_and_version: String,
}

impl ManagedTorrentShared {
    pub(crate) fn client_name_and_version(&self) -> &str {
        &self.client_name_and_version
    }
}

pub struct ManagedTorrent {
    pub(crate) run_region: parking_lot::Mutex<Option<ChildRegion>>,
    // Shared torrent configuration, including live-mutable rate limits.
    pub shared: Arc<ManagedTorrentShared>,
    // Torrent metadata. Maybe be None when the magnet is resolving (not implemented yet)
    pub metadata: ArcSwapOption<TorrentMetadata>,
    pub(crate) state_change_notify: Notify,
    pub(crate) locked: RwLock<ManagedTorrentLocked>,
}

impl ManagedTorrent {
    /// Wake on every state transition, including completion (which broadcasts
    /// only on the live state's finished notifier). Consumers must poll this
    /// future before sampling and sample again after it wakes. In particular an
    /// error broadcast can precede the assignment under the state write lock.
    pub async fn state_changed(&self) {
        let state = self.state_change_notify.notified();
        let live = self.live();
        if let Some(live) = live {
            rqbit_runtime::select! {
                _ = state => {},
                _ = live.finished_notify.notified() => {},
            }
        } else {
            state.await;
        }
    }

    pub fn id(&self) -> TorrentId {
        self.shared.id
    }

    pub fn name(&self) -> Option<String> {
        if let Some(m) = &*self.metadata.load() {
            return m
                .info
                .name()
                .map(|n| n.into_owned())
                .or_else(|| self.shared.magnet_name.clone());
        }
        self.shared.magnet_name.clone()
    }

    pub fn tracker_urls(&self) -> Vec<url::Url> { self.shared.trackers.snapshot() }

    pub fn add_tracker(&self, url: url::Url) -> anyhow::Result<bool> { self.shared.trackers.add(url) }

    pub fn remove_tracker(&self, url: &url::Url) -> bool { self.shared.trackers.remove(url) }

    /// Ordered observations survive pause/run-region replacement.
    pub fn tracker_stats(&self) -> Vec<tracker_comms::TrackerStat> {
        self.shared.tracker_stats.snapshot()
    }

    /// Wake tracker interval/backoff sleeps; repeated requests within 60 s coalesce.
    pub fn announce_now(&self) { self.shared.tracker_stats.announce_now(); }

    pub fn shared(&self) -> &ManagedTorrentShared {
        &self.shared
    }

    /// The resolved on-disk folder this torrent's files are written under.
    pub fn output_folder(&self) -> &Path {
        &self.shared.options.output_folder
    }

    /// Change the per-torrent payload upload cap without replacing its handle,
    /// storage or run state. None is unlimited; finite caps must accommodate a
    /// complete 16 KiB protocol chunk. The value survives pause/resume.
    pub fn set_upload_bps(&self, bps: Option<u64>) -> anyhow::Result<()> {
        let limit = bps.map(|bps| {
            anyhow::ensure!(bps >= 16 * 1024, "upload cap must be at least 16 KiB/s");
            let bps = u32::try_from(bps).context("upload cap exceeds the native engine maximum")?;
            Ok::<_, anyhow::Error>(std::num::NonZeroU32::new(bps).unwrap())
        }).transpose()?;
        let limits = &self.shared.options.ratelimits;
        if limits.get_upload_bps() != limit { limits.set_upload_bps(limit); }
        Ok(())
    }

    /// Permission is independent of rate. Updating a cap never opts into upload.
    pub fn set_upload_enabled(&self, enabled: Option<bool>) {
        let value = match enabled {None=>0,Some(false)=>1,Some(true)=>2};
        if self.shared.options.upload_override.swap(value, std::sync::atomic::Ordering::AcqRel) != value {
            self.refresh_upload_policy();
        }
    }

    pub fn upload_enabled(&self) -> bool { !self.shared.options.disable_upload() }

    pub(crate) fn refresh_upload_policy(&self) {
        if let Some(live) = self.live() { live.refresh_upload_policy(); }
    }

    /// Replace the download bucket in place; no handle/storage/state transition.
    pub fn set_download_bps(&self, bps: Option<u64>) -> anyhow::Result<()> {
        let limit=bps.map(|bps|{
            anyhow::ensure!(bps>=16384,"download cap must be at least 16 KiB/s");
            Ok::<_,anyhow::Error>(std::num::NonZeroU32::new(u32::try_from(bps).context("download cap exceeds the native engine maximum")?).unwrap())
        }).transpose()?;
        let limits=&self.shared.options.ratelimits;
        if limits.get_download_bps()!=limit {limits.set_download_bps(limit);}
        Ok(())
    }

    /// The session bucket already reflects the selected global/turtle policy.
    pub fn effective_download_bps(&self) -> Option<u64> {
        crate::limits::effective_rate([
            self.shared.options.ratelimits.get_download_bps(),
            self.shared.session.upgrade().and_then(|s|s.ratelimits.get_download_bps()),
        ]).map(|v|u64::from(v.get()))
    }

    pub fn with_metadata<R>(
        &self,
        mut f: impl FnMut(&Arc<TorrentMetadata>) -> R,
    ) -> anyhow::Result<R> {
        let r = self.metadata.load();
        let r = r.as_ref().context("torrent is not resolved")?;
        Ok(f(r))
    }

    pub fn info_hash(&self) -> Id20 {
        self.shared.info_hash
    }

    pub fn only_files(&self) -> Option<Vec<usize>> {
        self.locked.read().only_files.clone()
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&ManagedTorrentState) -> R) -> R {
        f(&self.locked.read().state)
    }

    pub(crate) fn with_state_mut<R>(&self, f: impl FnOnce(&mut ManagedTorrentState) -> R) -> R {
        f(&mut self.locked.write().state)
    }

    pub(crate) fn with_chunk_tracker<R>(
        &self,
        f: impl FnOnce(&ChunkTracker) -> R,
    ) -> anyhow::Result<R> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Paused(p) => Ok(f(&p.chunk_tracker)),
            ManagedTorrentState::Live(l) => Ok(f(l
                .lock_read("chunk_tracker")
                .get_chunks()
                .context("error getting chunks")?)),
            _ => bail!("no chunk tracker, torrent neither paused nor live"),
        }
    }

    /// Copy the verified have-bitfield into an independent in-memory snapshot.
    ///
    /// The copy is made while holding the tracker read locks. It retains the
    /// tracker's MSB-first byte layout, including zero padding in the last byte;
    /// metadata's piece count determines which bits are meaningful. Mutating or
    /// dropping the snapshot never changes the tracker or its persisted state.
    /// Returns `None` during initialization or whenever no tracker is available.
    /// A paused torrent with an initialized tracker still returns a snapshot.
    ///
    /// ```
    /// use librqbit::{BitV, ManagedTorrent};
    /// fn snapshot(torrent: &ManagedTorrent) -> Option<Box<dyn BitV>> {
    ///     torrent.have_pieces_snapshot()
    /// }
    /// ```
    pub fn have_pieces_snapshot(&self) -> Option<Box<dyn BitV>> {
        // The critical section allocates only the bitfield copy. Box the trait
        // object after with_chunk_tracker releases the tracker read guards.
        self.with_chunk_tracker(|tracker| {
            tracker
                .get_have_pieces()
                .as_slice()
                .to_bitvec()
                .into_boxed_bitslice()
        })
        .ok()
        .map(|bits| bits.into_dyn())
    }

    /// Get the live state if the torrent is live.
    pub fn live(&self) -> Option<Arc<TorrentStateLive>> {
        let g = self.locked.read();
        match &g.state {
            ManagedTorrentState::Live(live) => Some(live.clone()),
            _ => None,
        }
    }

    // Get live torrent but wait a bit until it's initialized if it is
    pub(crate) async fn live_wait_initializing(
        &self,
        cx: &Cx,
        duration: Duration,
    ) -> Option<Arc<TorrentStateLive>> {
        timeout(cx, duration, self.wait_until_initialized(cx))
            .await
            .ok()?
            .ok()?;
        self.live()
    }

    /// Get stats.
    pub fn stats(&self) -> TorrentStats {
        use stats::TorrentStatsState as S;
        let mut resp = TorrentStats {
            total_bytes: self
                .metadata
                .load()
                .as_ref()
                .map(|r| r.info.lengths().total_length())
                .unwrap_or_default(),
            file_progress: Vec::new(),
            state: S::Error,
            error: None,
            progress_bytes: 0,
            uploaded_bytes: 0,
            finished: false,
            live: None,
        };

        {
            let g = self.locked.read();
            match &g.state {
                ManagedTorrentState::Initializing(i) => {
                    resp.state = S::Initializing { paused: g.paused };
                    resp.progress_bytes = i.checked_bytes.load(Ordering::Relaxed);
                }
                ManagedTorrentState::Paused(p) => {
                    resp.state = S::Paused;
                    let hns = p.hns();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.file_progress = p.chunk_tracker.per_file_have_bytes().to_owned();
                }
                ManagedTorrentState::Live(l) => {
                    resp.state = S::Live;
                    let live_stats = LiveStats::from(l.as_ref());
                    let hns = l.get_hns().unwrap_or_default();
                    resp.total_bytes = hns.total();
                    resp.progress_bytes = hns.progress();
                    resp.finished = hns.finished();
                    resp.uploaded_bytes = l.get_uploaded_bytes();
                    resp.file_progress = l
                        .lock_read("file_progress")
                        .get_chunks()
                        .ok()
                        .map(|c| c.per_file_have_bytes().to_owned())
                        .unwrap_or_default();
                    resp.live = Some(live_stats);
                }
                ManagedTorrentState::Error(e) => {
                    resp.state = S::Error;
                    resp.error = Some(format!("{e:?}"))
                }
                ManagedTorrentState::None => {
                    resp.state = S::Error;
                    resp.error = Some("bug: torrent in broken \"None\" state".to_string());
                }
            }
        }

        resp
    }

    // Returns true if needed to unpause torrent.
    // This is just implementation detail - it's easier to pause/unpause than to tinker with internals.
    pub(crate) fn update_only_files(&self, only_files: &HashSet<usize>) -> anyhow::Result<()> {
        let metadata = self.metadata.load();
        let metadata = metadata.as_ref().context("torrent is not resolved")?;
        let file_count = metadata.file_infos.len();
        for f in only_files.iter().copied() {
            if f >= file_count {
                anyhow::bail!("only_files contains invalid value {f}")
            }
        }

        // if live, need to update chunk tracker
        // - if already finished: need to pause, then unpause (to reopen files etc)
        // if paused, need to update chunk tracker

        let mut g = self.locked.write();
        match &mut g.state {
            ManagedTorrentState::Initializing(_) => bail!("can't update initializing torrent"),
            ManagedTorrentState::Error(_) => {}
            ManagedTorrentState::None => {}
            ManagedTorrentState::Paused(p) => {
                p.update_only_files(only_files)?;
            }
            ManagedTorrentState::Live(l) => {
                l.update_only_files(only_files)?;
            }
        };

        g.only_files = Some(only_files.iter().copied().collect());
        Ok(())
    }
}

pub type ManagedTorrentHandle = Arc<ManagedTorrent>;

#[cfg(test)]
mod change_tests {
    use super::*;
    use std::{future::Future, task::Poll};
    async fn arm(wait: std::pin::Pin<&mut impl Future<Output=()>>) {
        let mut wait=wait;
        std::future::poll_fn(|context| { assert!(wait.as_mut().poll(context).is_pending(), "wait must register before the transition"); Poll::Ready(()) }).await;
    }
    #[test]
    fn state_changed_pause_unpause_completion_error() {
        let seed_dir=tempfile::tempdir().unwrap();
        let client_dir=tempfile::tempdir().unwrap();
        std::fs::write(seed_dir.path().join("hello.txt"),b"hello").unwrap();
        let mut bytes=b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[0xaa,0xf4,0xc6,0x1d,0xdc,0xc5,0xe8,0xa2,0xda,0xbe,0xde,0x0f,0x3b,0x48,0x2c,0xd9,0xae,0xa9,0x43,0x4d]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let runtime=rqbit_runtime::RuntimeBuilder::new().worker_threads(2).enable_time().build().unwrap();
        let cx=runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        runtime.block_on(async {
            let opts=||crate::SessionOptions{dht:None,disable_trackers:true,disable_local_service_discovery:true,..Default::default()};
            let seed=Session::new_with_opts(&cx,seed_dir.path().to_owned(),crate::SessionOptions{listen:Some(crate::ListenerOptions{listen_addr:"127.0.0.1:0".parse().unwrap(),..Default::default()}),..opts()}).await.unwrap();
            let seed_handle=seed.add_torrent(&cx,crate::AddTorrent::from_bytes(bytes.clone()),Some(crate::AddTorrentOptions{overwrite:true,..Default::default()})).await.unwrap().into_handle().unwrap();
            timeout(&cx,Duration::from_secs(5),seed_handle.wait_until_completed(&cx)).await.unwrap().unwrap();
            let client=Session::new_with_opts(&cx,client_dir.path().to_owned(),opts()).await.unwrap();
            let handle=client.add_torrent(&cx,crate::AddTorrent::from_bytes(bytes),Some(crate::AddTorrentOptions{paused:true,overwrite:true,..Default::default()})).await.unwrap().into_handle().unwrap();
            timeout(&cx,Duration::from_secs(5),handle.wait_until_initialized(&cx)).await.unwrap().unwrap();
            let mut wait=std::pin::pin!(handle.state_changed());arm(wait.as_mut()).await;
            client.unpause(&cx,&handle).await.unwrap();timeout(&cx,Duration::from_secs(5),wait).await.unwrap();
            assert!(matches!(handle.stats().state,TorrentStatsState::Live));
            let mut wait=std::pin::pin!(handle.state_changed());arm(wait.as_mut()).await;
            client.pause(&cx,&handle).await.unwrap();timeout(&cx,Duration::from_secs(5),wait).await.unwrap();
            assert!(handle.is_paused());
            client.unpause(&cx,&handle).await.unwrap();
            let mut wait=std::pin::pin!(handle.state_changed());arm(wait.as_mut()).await;
            handle.live().unwrap().add_peer_from_discovery(seed.listen_addr().unwrap()).unwrap();
            timeout(&cx,Duration::from_secs(10),wait).await.unwrap();
            assert!(handle.stats().finished,"completion must wake the live finished notifier");
            assert_eq!(std::fs::read(client_dir.path().join("hello.txt")).unwrap(),b"hello");
            let mut wait=std::pin::pin!(handle.state_changed());arm(wait.as_mut()).await;
            handle.stop_with_error(anyhow::anyhow!("state_changed error regression"));
            timeout(&cx,Duration::from_secs(5),wait).await.unwrap();
            assert!(matches!(handle.stats().state,TorrentStatsState::Error));
            timeout(&cx,Duration::from_secs(5),client.stop(&cx)).await.unwrap().unwrap();
            timeout(&cx,Duration::from_secs(5),seed.stop(&cx)).await.unwrap().unwrap();
            println!("state_changed woke on real pause, unpause, loopback completion and error");
        });
    }
}
