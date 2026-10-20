use super::{BlockingCheckGuard, TorrentStateInitializing};
use crate::{file_ops::FileOps, type_aliases::BF};
use rqbit_runtime::Cx;
use std::sync::{Arc, atomic::Ordering};

// At most a quarter of logical CPUs and never more than two workers per check.
// Jobs use the existing storage admission and bounded native blocking pool;
// concurrent torrents do not create extra threads or a second runtime/pool.
fn workers(bytes: u64, pieces: u32) -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() / 4)
        .clamp(1, 2)
        .min((bytes / (16 * 1024 * 1024)).clamp(1, 2) as usize)
        .min(pieces.max(1) as usize)
}

pub(super) async fn check(operation: Arc<TorrentStateInitializing>, cx: &Cx) -> anyhow::Result<BF> {
    let lengths = operation.metadata.lengths();
    let count = workers(lengths.total_length(), lengths.total_pieces());
    tracing::debug!(
        workers = count,
        pieces = lengths.total_pieces(),
        "checking disjoint piece ranges"
    );
    let stride = lengths.total_pieces().div_ceil(count as u32);
    operation
        .blocking_operation_running
        .store(true, Ordering::Release);
    // Shared guard outlives every physical job, even if its async waiter is
    // cancelled. A pause/resume cannot admit replacement storage prematurely.
    let guard = Arc::new(BlockingCheckGuard(operation.clone()));
    let jobs = (0..count).map(|index| {
        let guard = guard.clone();
        let operation = operation.clone();
        let operation_cx = cx.clone();
        let start = index as u32 * stride;
        let end = start.saturating_add(stride).min(lengths.total_pieces());
        async move {
            operation
                .shared
                .spawner
                .run(cx, {
                    let operation = operation.clone();
                    move || {
                        let _guard = guard;
                        FileOps::new(
                            &operation.metadata.info,
                            &operation.files,
                            &operation.metadata.file_infos,
                        )
                        .initial_check_range(
                            &operation_cx,
                            &operation.checked_bytes,
                            &operation.pause_requested,
                            start..end,
                        )
                    }
                })
                .await?
        }
    });
    let results = futures::future::join_all(jobs).await;
    cx.checkpoint()?;
    anyhow::ensure!(!operation.is_pause_requested(), "initial check paused");
    let mut have = BF::from_boxed_slice(vec![0; lengths.piece_bitfield_bytes()].into());
    for result in results {
        let verified = result?;
        for (target, source) in have
            .as_raw_mut_slice()
            .iter_mut()
            .zip(verified.as_raw_slice())
        {
            *target |= *source;
        }
    }
    Ok(have)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verify_worker_budget_is_bounded_and_small_checks_stay_serial() {
        assert_eq!(workers(1024, 1024), 1);
        assert_eq!(workers(u64::MAX, 1), 1);
        let limit = (std::thread::available_parallelism().unwrap().get() / 4).clamp(1, 2);
        assert_eq!(workers(4 * 1024 * 1024 * 1024, 16384), limit);
    }
}
