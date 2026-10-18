use std::sync::{Arc, OnceLock};

use rqbit_runtime::{Cx, cx::cx::RegionOwnerKey, sync::Semaphore};

mod storage_owner;

/// Bounds concurrent storage operations and dispatches owned blocking work to
/// the native runtime's blocking pool.
#[derive(Clone, Debug)]
pub struct BlockingSpawner {
    concurrent_block_in_place_semaphore: Arc<Semaphore>,
    owners: Arc<storage_owner::OwnerCache>,
}

impl BlockingSpawner {
    pub fn new(max_blocking_threads: usize) -> Self {
        let limit = max_blocking_threads.max(1);
        let admission = Arc::new(Semaphore::new(limit));
        Self {
            concurrent_block_in_place_semaphore: admission.clone(),
            owners: Arc::new(storage_owner::OwnerCache::new(admission, limit)),
        }
    }

    /// Executes a short borrowed operation synchronously.
    ///
    /// Asupersync cannot move a borrowed closure to its blocking pool. Longer
    /// operations should own their input and use `run` instead.
    pub fn block_in_place<F: FnOnce() -> R, R>(&self, f: F) -> R {
        f()
    }

    /// Limits concurrent borrowed operations. The closure runs on the calling
    /// thread after admission; it must be bounded in duration.
    pub async fn block_in_place_with_semaphore<F: FnOnce() -> R, R>(
        &self,
        cx: &Cx,
        f: F,
    ) -> Result<R, rqbit_runtime::sync::AcquireError> {
        let _permit = self
            .concurrent_block_in_place_semaphore
            .acquire(cx, 1)
            .await?;
        Ok(f())
    }

    /// Runs owned storage work on the native blocking pool and checks its join.
    pub async fn run<F, R>(&self, cx: &Cx, f: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        cx.checkpoint()?;
        storage_owner::per_operation(cx, self.concurrent_block_in_place_semaphore.clone(), f).await
    }

    /// Explicit lifetime for storage owner reuse; keep this with the peer handler.
    pub(crate) async fn lease(&self, cx: &Cx) -> anyhow::Result<StorageLease> {
        cx.checkpoint()?;
        Ok(StorageLease {
            spawner: self.clone(),
            // Key includes the original priority; it holds only weak identity.
            domain: cx.reusable_region_owner_key(),
            group: OnceLock::new(),
        })
    }

    pub fn semaphore(&self) -> Arc<Semaphore> {
        self.concurrent_block_in_place_semaphore.clone()
    }
}

/// The last lease drops the group sender. Cache metadata has no strong group
/// reference; workers drain accepted jobs and their physical futures, then exit.
pub(crate) struct StorageLease {
    spawner: BlockingSpawner,
    domain: Option<RegionOwnerKey>,
    group: OnceLock<Arc<storage_owner::Group>>,
}

impl StorageLease {
    /// Batching reuses an already-live owner only. First use, cache saturation,
    /// changed authority and per-operation fallback stay on one frame.
    pub(crate) fn batch_domain(&self, cx: &Cx) -> Option<RegionOwnerKey> {
        let group = self.group.get()?;
        let key = cx.reusable_region_owner_key()?;
        (self.domain.as_ref() == Some(&key) && group.accepts(cx)).then_some(key)
    }

    pub(crate) async fn run<F, R>(&self, cx: &Cx, work: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        cx.checkpoint()?;
        if self.domain.is_none() || cx.reusable_region_owner_key().as_ref() != self.domain.as_ref()
        {
            return storage_owner::per_operation(cx, self.spawner.semaphore(), work).await;
        }
        if self.group.get().is_none() {
            if let Some(candidate) = self.spawner.owners.group(cx).await? {
                // Arbitration happened before F acceptance. Recheck after the
                // await, then publish without another yield. Concurrent first
                // calls use the cache's same-key group; losing set drops only
                // an unaccepted Arc, never retries an accepted callback.
                if cx.reusable_region_owner_key().as_ref() == self.domain.as_ref()
                    && candidate.accepts(cx)
                {
                    let _ = self.group.set(candidate);
                }
            }
        }
        if let Some(group) = self.group.get() {
            if group.accepts(cx) {
                return group.run(cx, work).await;
            }
        }
        // None is never cached: a full/draining cache can recover next call.
        storage_owner::per_operation(cx, self.spawner.semaphore(), work).await
    }
}
