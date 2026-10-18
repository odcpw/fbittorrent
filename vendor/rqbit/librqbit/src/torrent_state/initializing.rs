use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::Context;

use rqbit_runtime::{Cx, sync::Notify};
use size_format::SizeFormatterBinary as SF;
use tracing::{info, trace, warn};

use crate::{
    api::TorrentIdOrHash,
    bitv::BitV,
    chunk_tracker::{ChunkTracker, compute_selected_pieces},
    file_ops::FileOps,
    type_aliases::FileStorage,
};

use super::{ManagedTorrentShared, TorrentMetadata, paused::TorrentStatePaused};

pub struct TorrentStateInitializing {
    pub(crate) files: FileStorage,
    pub(crate) shared: Arc<ManagedTorrentShared>,
    pub(crate) metadata: Arc<TorrentMetadata>,
    pub(crate) only_files: Option<Vec<usize>>,
    pub(crate) checked_bytes: AtomicU64,
    pause_requested: AtomicBool,
    check_running: AtomicBool,
    blocking_operation_running: AtomicBool,
    blocking_operation_finished: Notify,
    previously_errored: bool,
}

// A cancelled async join can return while its blocking closure is still
// unwinding. Keep initialization admission closed until that closure releases
// its storage references.
struct BlockingCheckGuard(Arc<TorrentStateInitializing>);

impl Drop for BlockingCheckGuard {
    fn drop(&mut self) {
        self.0
            .blocking_operation_running
            .store(false, Ordering::Release);
        self.0.blocking_operation_finished.notify_waiters();
    }
}

impl TorrentStateInitializing {
    pub fn new(
        shared: Arc<ManagedTorrentShared>,
        metadata: Arc<TorrentMetadata>,
        only_files: Option<Vec<usize>>,
        files: FileStorage,
        previously_errored: bool,
    ) -> Self {
        Self {
            shared,
            metadata,
            only_files,
            files,
            checked_bytes: AtomicU64::new(0),
            pause_requested: AtomicBool::new(false),
            check_running: AtomicBool::new(false),
            blocking_operation_running: AtomicBool::new(false),
            blocking_operation_finished: Notify::new(),
            previously_errored,
        }
    }

    pub fn get_checked_bytes(&self) -> u64 {
        self.checked_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn request_pause(&self) {
        self.pause_requested.store(true, Ordering::Relaxed);
    }

    pub(crate) fn clear_pause_request(&self) {
        self.pause_requested.store(false, Ordering::Relaxed);
    }

    pub(crate) fn is_pause_requested(&self) -> bool {
        self.pause_requested.load(Ordering::Relaxed)
    }

    pub(crate) fn try_start_check(&self) -> bool {
        if self.blocking_operation_running.load(Ordering::Acquire) {
            return false;
        }
        self.check_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn finish_check(&self) {
        self.check_running.store(false, Ordering::Release);
    }

    pub(crate) async fn wait_for_blocking_work(&self, cx: &Cx) -> anyhow::Result<()> {
        cx.checkpoint()?;
        self.blocking_operation_finished
            .wait_until(|| !self.blocking_operation_running.load(Ordering::Acquire))
            .await;
        cx.checkpoint()?;
        Ok(())
    }

    /// The native embedded client verifies every piece from disk at startup.
    /// The client owns persistence and never trusts an upstream fastresume bitfield.
    pub async fn check(self: Arc<Self>, cx: &Cx) -> anyhow::Result<TorrentStatePaused> {
        cx.checkpoint()?;
        anyhow::ensure!(!self.is_pause_requested(), "initial check paused");
        let id: TorrentIdOrHash = self.shared.info_hash.into();
        let bitv_factory = self
            .shared
            .session
            .upgrade()
            .context("session is dead")?
            .bitv_factory
            .clone();
        if self.previously_errored {
            if let Err(e) = bitv_factory.clear(cx, id).await {
                warn!(id=?self.shared.id, info_hash = ?self.shared.info_hash, error=?e, "error clearing bitfield");
            }
        }

        info!("Doing initial checksum validation, this might take a while...");
        self.checked_bytes.store(0, Ordering::Relaxed);
        let operation = self.clone();
        let operation_cx = cx.clone();
        self.blocking_operation_running
            .store(true, Ordering::Release);
        let work_guard = BlockingCheckGuard(self.clone());
        let have_pieces = self
            .shared
            .spawner
            .run(cx, move || {
                let _guard = work_guard;
                FileOps::new(
                    &operation.metadata.info,
                    &operation.files,
                    &operation.metadata.file_infos,
                )
                .initial_check(
                    &operation_cx,
                    &operation.checked_bytes,
                    &operation.pause_requested,
                )
            })
            .await??;
        cx.checkpoint()?;
        anyhow::ensure!(!self.is_pause_requested(), "initial check paused");
        let have_pieces = bitv_factory
            .store_initial_check(cx, id, have_pieces)
            .await
            .context("error storing initial check bitfield")?;

        let selected_pieces = compute_selected_pieces(
            self.metadata.lengths(),
            |idx| {
                self.only_files
                    .as_ref()
                    .map(|o| o.contains(&idx))
                    .unwrap_or(true)
            },
            &self.metadata.file_infos,
        );

        let chunk_tracker = ChunkTracker::new(
            have_pieces.into_dyn(),
            selected_pieces,
            *self.metadata.lengths(),
            &self.metadata.file_infos,
        )
        .context("error creating chunk tracker")?;

        let hns = chunk_tracker.get_hns();

        info!(
            torrent=?self.shared.id,
            "Initial check results: have {}, needed {}, total selected {}",
            SF::new(hns.have_bytes),
            SF::new(hns.needed_bytes),
            SF::new(hns.selected_bytes)
        );

        // Ensure file lengths are correct, and reopen read-only.
        let operation = self.clone();
        let operation_cx = cx.clone();
        self.blocking_operation_running
            .store(true, Ordering::Release);
        let work_guard = BlockingCheckGuard(self.clone());
        self.shared
            .spawner
            .run(cx, move || {
                let _guard = work_guard;
                for (idx, fi) in operation.metadata.file_infos.iter().enumerate() {
                    operation_cx.checkpoint()?;
                    anyhow::ensure!(!operation.is_pause_requested(), "initial check paused");
                    if operation
                        .only_files
                        .as_ref()
                        .map(|v| v.contains(&idx))
                        .unwrap_or(true)
                    {
                        let now = Instant::now();
                        if fi.attrs.padding {
                            continue;
                        }
                        if let Err(err) = operation.files.ensure_file_length(idx, fi.len) {
                            warn!(
                                id=?operation.shared.id, info_hash = ?operation.shared.info_hash,
                                "Error setting length for file {:?} to {}: {:#?}",
                                fi.relative_filename, fi.len, err
                            );
                        } else {
                            trace!(
                                "Set length for file {:?} to {} in {:?}",
                                fi.relative_filename,
                                SF::new(fi.len),
                                now.elapsed()
                            );
                        }
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .await??;

        cx.checkpoint()?;
        anyhow::ensure!(!self.is_pause_requested(), "initial check paused");
        let paused = TorrentStatePaused {
            shared: self.shared.clone(),
            metadata: self.metadata.clone(),
            files: self.files.take()?,
            chunk_tracker,
            streams: Arc::new(Default::default()),
        };
        Ok(paused)
    }
}
