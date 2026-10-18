// Region-owned storage dispatch. Physical callbacks keep their admission
// permit and live task principal even when their result waiter is dropped.
use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::{
    channel::oneshot,
    future::{Either, select},
};
use rqbit_runtime::{
    CancellationToken, Cx,
    cx::cx::RegionOwnerKey,
    sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, mpsc},
};

// Bound persistent metadata independently of the number of peers. A full
// cache takes the same safe per-operation ownership path as ineligible Cx.
const MAX_CACHED_GROUPS: usize = 16;

pub(super) struct OwnerCache {
    entries: Mutex<Vec<Entry>>,
    live_groups: Arc<AtomicUsize>,
    admission: Arc<Semaphore>,
    worker_limit: usize,
}

struct Entry {
    key: RegionOwnerKey,
    group: Arc<GroupSlot>,
}

#[derive(Default)]
struct GroupSlot {
    result: Mutex<Option<Result<(Weak<Group>, Weak<Shared>), String>>>,
    ready: rqbit_runtime::sync::Notify,
}

impl GroupSlot {
    fn publish(&self, result: Result<Arc<Group>, String>) {
        *self.result.lock().unwrap() =
            Some(result.map(|group| (Arc::downgrade(&group), Arc::downgrade(&group.shared))));
        self.ready.notify_waiters();
    }
    fn retired(&self) -> bool {
        self.result
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|result| match result {
                Err(_) => true,
                Ok((_, shared)) => shared
                    .upgrade()
                    .is_none_or(|shared| shared.alive.load(Ordering::Acquire) == 0),
            })
    }
    async fn get(&self) -> anyhow::Result<Option<Arc<Group>>> {
        self.ready
            .wait_until(|| self.result.lock().unwrap().is_some())
            .await;
        self.result
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .map(|(group, _)| group.upgrade())
            .map_err(anyhow::Error::msg)
    }
}

struct PublishFailureOnDrop(Arc<GroupSlot>);
impl Drop for PublishFailureOnDrop {
    fn drop(&mut self) {
        let unpublished = self.0.result.lock().unwrap().is_none();
        if unpublished {
            self.0.publish(Err(
                "storage owner initialization did not complete".to_owned()
            ));
        }
    }
}

impl OwnerCache {
    pub(super) fn new(admission: Arc<Semaphore>, worker_limit: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            live_groups: Arc::new(AtomicUsize::new(0)),
            admission,
            worker_limit: worker_limit.max(1),
        }
    }

    pub(super) async fn group(&self, cx: &Cx) -> anyhow::Result<Option<Arc<Group>>> {
        let Some(key) = cx.reusable_region_owner_key() else {
            return Ok(None);
        };
        let mut retired = Vec::new();
        let selected = {
            let mut entries = self.entries.lock().unwrap();
            let mut index = 0;
            while index < entries.len() {
                if entries[index].group.retired() {
                    retired.push(entries.remove(index));
                } else {
                    index += 1;
                }
            }
            if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
                Some((entry.group.clone(), None))
            } else if entries.len() < MAX_CACHED_GROUPS
                && self.live_groups.load(Ordering::Acquire) < MAX_CACHED_GROUPS
            {
                let group = Arc::new(GroupSlot::default());
                entries.push(Entry {
                    key,
                    group: group.clone(),
                });
                self.live_groups.fetch_add(1, Ordering::AcqRel);
                Some((group, Some(LiveSlot(self.live_groups.clone()))))
            } else {
                None
            }
        };
        // Sender/receiver/job destructors can wake callers: no cache lock.
        drop(retired);
        let Some((slot, reservation)) = selected else {
            return Ok(None);
        };
        let group = if let Some(reservation) = reservation {
            let _failure = PublishFailureOnDrop(slot.clone());
            let created = Group::new(cx, self.admission.clone(), self.worker_limit, reservation)
                .map_err(|error| format!("blocking storage operation failed: {error}"));
            // Keep the creator's strong group alive through weak publication.
            slot.publish(created.clone());
            Some(created.map_err(anyhow::Error::msg)?)
        } else {
            // A departed group may still have live draining workers. Its weak
            // group cannot be resurrected: use fallback without creating a
            // second group for this key until the old workers have exited.
            slot.get().await?
        };
        let Some(group) = group else {
            return Ok(None);
        };
        if group.shared.stopping.is_cancelled() {
            return Ok(None);
        }
        Ok(Some(group))
    }
}

struct LiveSlot(Arc<AtomicUsize>);
impl Drop for LiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

trait Job: Send {
    fn checkpoint(&self) -> anyhow::Result<()>;
    fn reject(self: Box<Self>, error: anyhow::Error);
    fn execute(self: Box<Self>, permit: OwnedSemaphorePermit, owner: Cx) -> Reply;
}

type Reply = Box<dyn FnOnce() + Send>;

struct TypedJob<F, R> {
    origin: Cx,
    domain: RegionOwnerKey,
    work: Option<F>,
    reply: Option<oneshot::Sender<anyhow::Result<R>>>,
    _request: Request,
}

impl<F, R> Job for TypedJob<F, R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    fn checkpoint(&self) -> anyhow::Result<()> {
        self.origin
            .checkpoint()
            .map_err(|_| cancelled(&self.origin))?;
        if self.origin.reusable_region_owner_key().as_ref() != Some(&self.domain) {
            // Shared budget/accounting state can be tightened after enqueue.
            // Fail closed before F; never borrow the warmed owner's envelope.
            return Err(anyhow::anyhow!(
                "blocking storage operation failed: caller authority changed before physical entry"
            ));
        }
        Ok(())
    }
    fn reject(mut self: Box<Self>, error: anyhow::Error) {
        let error = dispose_work(&mut self.work).err().unwrap_or(error);
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(error));
        }
    }
    fn execute(mut self: Box<Self>, permit: OwnedSemaphorePermit, owner: Cx) -> Reply {
        let mut result = self.checkpoint().and_then(|()| {
            owner.checkpoint().map_err(|_| cancelled(&self.origin))?;
            let work = self.work.take().unwrap();
            // The LIVE owner owns native effects/obligations. Its inherited
            // handles match this caller's opaque key; the original F stays intact.
            let _ambient = Cx::set_current(Some(owner));
            catch_unwind(AssertUnwindSafe(work)).map_err(panicked)
        });
        if let Err(error) = dispose_work(&mut self.work) {
            result = Err(error);
        }
        drop(permit);
        let reply = self.reply.take();
        // Only typed result state crosses back to the owner. The physical
        // permit, ambient guard, input captures and job all drop before the
        // pool completion send; the owner then publishes this typed reply.
        Box::new(move || {
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
        })
    }
}

impl<F, R> Drop for TypedJob<F, R> {
    fn drop(&mut self) {
        let disposed = dispose_work(&mut self.work);
        if let Some(reply) = self.reply.take() {
            let error = disposed.err().unwrap_or_else(|| cancelled(&self.origin));
            let _ = reply.send(Err(error));
        }
    }
}

fn dispose_work<F>(work: &mut Option<F>) -> anyhow::Result<()> {
    // A rejected callback is still user code: its capture destructors may
    // panic. Settle that panic before replying without unwinding the owner
    // which may be draining a different, already-running physical callback.
    let Some(work) = work.take() else {
        return Ok(());
    };
    catch_unwind(AssertUnwindSafe(|| drop(work))).map_err(panicked)
}

type Inbox = Arc<rqbit_runtime::sync::Mutex<mpsc::Receiver<Box<dyn Job>>>>;

struct Shared {
    live_slot: Mutex<Option<LiveSlot>>,
    #[cfg(test)]
    spawn_reserved_pause: Mutex<Option<oneshot::Receiver<()>>>,
    domain: RegionOwnerKey,
    admission: Arc<Semaphore>,
    receiver: Weak<rqbit_runtime::sync::Mutex<mpsc::Receiver<Box<dyn Job>>>>,
    stopping: CancellationToken,
    alive: AtomicUsize,
    requests: AtomicUsize,
    worker_limit: usize,
}

impl Shared {
    fn retire_worker(&self) {
        if self.alive.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Both a drained physical worker and an abandoned spawn reservation
            // can be the last owner. Retained Group Arcs must not retain quota.
            drop(self.live_slot.lock().unwrap().take());
        }
    }
}

pub(super) struct Group {
    sender: mpsc::Sender<Box<dyn Job>>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for OwnerCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerCache")
            .field("worker_limit", &self.worker_limit)
            .field("cached_groups", &self.entries.lock().unwrap().len())
            .finish()
    }
}

struct Request(Arc<Shared>);
impl Drop for Request {
    fn drop(&mut self) {
        self.0.requests.fetch_sub(1, Ordering::AcqRel);
    }
}
struct WorkerAlive {
    shared: Arc<Shared>,
    graceful: bool,
}

#[derive(Clone, Copy)]
enum WorkerExit {
    Graceful,
    Stopped,
}
impl Drop for WorkerAlive {
    fn drop(&mut self) {
        // Also cover cancellation/denial before the worker body first polls.
        if !self.graceful {
            self.shared.stopping.cancel();
            if let Some(receiver) = self.shared.receiver.upgrade() {
                close_inbox(&receiver);
            }
        }
        self.shared.retire_worker();
    }
}

struct PhysicalJob {
    job: Option<Box<dyn Job>>,
    permit: Option<OwnedSemaphorePermit>,
    owner: Option<Cx>,
}
impl PhysicalJob {
    fn run(mut self) -> Reply {
        self.job
            .take()
            .unwrap()
            .execute(self.permit.take().unwrap(), self.owner.take().unwrap())
    }
}
impl Drop for PhysicalJob {
    fn drop(&mut self) {
        // Pool rejection also releases physical ownership before the typed
        // job's drop guard can wake its waiter with an error.
        drop(self.permit.take());
        drop(self.owner.take());
        drop(self.job.take());
    }
}

impl Group {
    fn new(
        cx: &Cx,
        admission: Arc<Semaphore>,
        worker_limit: usize,
        live_slot: LiveSlot,
    ) -> anyhow::Result<Arc<Self>> {
        let domain = cx.reusable_region_owner_key().ok_or_else(|| {
            anyhow::anyhow!("caller authority changed before storage owner creation")
        })?;
        let (sender, receiver) = mpsc::channel(worker_limit);
        let receiver = Arc::new(rqbit_runtime::sync::Mutex::new(receiver));
        let shared = Arc::new(Shared {
            live_slot: Mutex::new(Some(live_slot)),
            #[cfg(test)]
            spawn_reserved_pause: Mutex::new(None),
            domain,
            admission,
            receiver: Arc::downgrade(&receiver),
            stopping: CancellationToken::new(),
            alive: AtomicUsize::new(1),
            requests: AtomicUsize::new(0),
            worker_limit,
        });
        spawn_worker(cx, shared.clone(), receiver)?;
        Ok(Arc::new(Self { sender, shared }))
    }

    pub(super) fn accepts(&self, cx: &Cx) -> bool {
        self.shared.alive.load(Ordering::Acquire) > 0
            && !self.shared.stopping.is_cancelled()
            && cx.reusable_region_owner_key().as_ref() == Some(&self.shared.domain)
    }

    pub(super) async fn run<F, R>(&self, cx: &Cx, work: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        if self.shared.stopping.is_cancelled() {
            return Err(cancelled(cx));
        }
        let demand = self.shared.requests.fetch_add(1, Ordering::AcqRel) + 1;
        let request = Request(self.shared.clone());
        let workers = self.shared.alive.load(Ordering::Acquire);
        // A retired group can never acquire a new owner after alive reaches
        // zero: this keeps the cache's live-group bound valid during pruning.
        if demand > workers && workers > 0 && workers < self.shared.worker_limit {
            if self
                .shared
                .alive
                .compare_exchange(workers, workers + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                #[cfg(test)]
                {
                    // Deterministically exercise last-worker exit between the
                    // real spawn reservation and receiver-upgrade rollback.
                    let pause = self.shared.spawn_reserved_pause.lock().unwrap().take();
                    if let Some(pause) = pause {
                        let _ = pause.await;
                    }
                }
                if let Some(receiver) = self.shared.receiver.upgrade() {
                    if let Err(error) = spawn_worker(cx, self.shared.clone(), receiver) {
                        return Err(anyhow::anyhow!(
                            "blocking storage operation failed: {error}"
                        ));
                    }
                } else {
                    self.shared.retire_worker();
                    return Err(cancelled(cx));
                }
            }
        }
        let (reply, result) = oneshot::channel();
        let job = Box::new(TypedJob {
            origin: cx.clone(),
            domain: self.shared.domain.clone(),
            work: Some(work),
            reply: Some(reply),
            _request: request,
        });
        self.sender
            .send(cx, job as Box<dyn Job>)
            .await
            .map_err(|_| cancelled(cx))?;
        // Terminal reply is intentionally uninterruptible: a physical panic
        // must not be hidden by a simultaneous cancellation request.
        let result = result.await.map_err(|_| cancelled(cx))?;
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                cx.checkpoint().map_err(|_| cancelled(cx))?;
                Ok(value)
            }
        }
    }
}

fn spawn_worker(cx: &Cx, shared: Arc<Shared>, receiver: Inbox) -> anyhow::Result<()> {
    let alive = WorkerAlive {
        shared: shared.clone(),
        graceful: false,
    };
    let operation = cx.spawn(move |owner| async move {
        let mut alive = alive;
        if matches!(worker(&owner, shared, receiver).await, WorkerExit::Graceful) {
            alive.graceful = true;
        }
    })?;
    // TaskHandle drop detaches the waiter; the exact region owns this worker.
    drop(operation);
    Ok(())
}

async fn worker(cx: &Cx, shared: Arc<Shared>, receiver: Inbox) -> WorkerExit {
    // Keep one cancellation-aware receive and its registration for this
    // owner's entire lifetime. The sender never publishes a value: only Cx
    // cancellation makes this branch ready. Notify::wait_until alone cannot
    // observe a Cx wake while its internal notification remains pending.
    let (_keep_open, mut cancel_rx) = rqbit_runtime::sync::oneshot::channel::<()>();
    let mut owner_cancelled = std::pin::pin!(cancel_rx.recv(cx));
    let mut group_stopping = std::pin::pin!(shared.stopping.cancelled());
    loop {
        let job = {
            let mut receive = match OwnedMutexGuard::lock(receiver.clone(), cx).await {
                Ok(receive) => receive,
                Err(_) => {
                    shared.stopping.cancel();
                    close_inbox(&receiver);
                    return WorkerExit::Stopped;
                }
            };
            let received = {
                let recv = std::pin::pin!(receive.recv(cx));
                match select(recv, group_stopping.as_mut()).await {
                    Either::Left((job, _)) => Some(job),
                    Either::Right(_) => None,
                }
            };
            match received {
                None => {
                    shared.stopping.cancel();
                    drop(receive);
                    close_inbox(&receiver);
                    return WorkerExit::Stopped;
                }
                Some(job) => match job {
                    Ok(job) => job,
                    Err(mpsc::RecvError::Disconnected)
                        if receive.is_closed()
                            && !shared.stopping.is_cancelled()
                            && cx.checkpoint().is_ok() =>
                    {
                        // Sender EOF is graceful: other owners may already
                        // hold accepted jobs awaiting global admission or raw
                        // completion. Do not cancel or drain those siblings.
                        return WorkerExit::Graceful;
                    }
                    Err(_) => {
                        shared.stopping.cancel();
                        drop(receive);
                        close_inbox(&receiver);
                        return WorkerExit::Stopped;
                    }
                },
            }
        };
        if shared.stopping.is_cancelled() {
            job.reject(cancelled(cx));
            continue;
        }
        if let Err(error) = job.checkpoint() {
            job.reject(error);
            continue;
        }
        let permit = match OwnedSemaphorePermit::acquire(shared.admission.clone(), cx, 1).await {
            Ok(permit) => permit,
            Err(error) => {
                job.reject(error.into());
                continue;
            }
        };
        if shared.stopping.is_cancelled() {
            drop(permit);
            job.reject(cancelled(cx));
            continue;
        }
        if let Err(error) = job.checkpoint() {
            drop(permit);
            job.reject(error);
            continue;
        }
        let physical = PhysicalJob {
            job: Some(job),
            permit: Some(permit),
            owner: Some(cx.clone()),
        };
        // Cancellation may dispose queued jobs, but the same physical future
        // is retained and drained while the callback owns the admission permit.
        let physical = std::pin::pin!(on_inherited_pool(cx, move || physical.run()));
        let stopping = std::pin::pin!(select(group_stopping.as_mut(), owner_cancelled.as_mut()));
        let (completed, stopped) = match select(physical, stopping).await {
            Either::Left((completed, _)) => (completed, false),
            Either::Right((_, physical)) => {
                // Observe region cancellation without abandoning physical
                // work: dispose queued jobs now, then drain this same future.
                shared.stopping.cancel();
                close_inbox(&receiver);
                (physical.await, true)
            }
        };
        match completed {
            Ok(publish) => publish(),
            Err(_) => return WorkerExit::Stopped,
        }
        if stopped {
            return WorkerExit::Stopped;
        }
    }
}

fn close_inbox(receiver: &Inbox) {
    // If another idle owner holds the receiver, stopping wakes that owner;
    // it releases the guard and performs this same close/drain. No spin.
    if let Ok(mut receive) = OwnedMutexGuard::try_lock(receiver.clone()) {
        receive.close();
        let mut queued = Vec::new();
        while let Ok(job) = receive.try_recv() {
            queued.push(job);
        }
        drop(receive);
        // User capture destructors and terminal replies can wake other tasks.
        for job in queued {
            // Each job settles independently. One destructor panic must not
            // skip the rest of the drain or unwind a live physical owner.
            let _ = catch_unwind(AssertUnwindSafe(|| drop(job)));
        }
    }
}

pub(super) async fn per_operation<F, R>(
    cx: &Cx,
    admission: Arc<Semaphore>,
    work: F,
) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let origin = cx.clone();
    let operation = rqbit_runtime::task::try_spawn_with(cx, move |owner| async move {
        let permit = OwnedSemaphorePermit::acquire(admission, &owner, 1).await?;
        let physical_owner = owner.clone();
        let physical_origin = origin.clone();
        let physical = move || {
            let _permit = permit;
            physical_origin
                .checkpoint()
                .map_err(|_| cancelled(&physical_origin))?;
            physical_owner.apply_child_capability_budget(
                physical_origin.capability_budget(),
                rqbit_runtime::types::CapabilityBudgetRequirements::NONE,
            ).map_err(|error| anyhow::anyhow!(
                "blocking storage operation failed: caller capability budget refused at physical entry: {error}"
            ))?;
            physical_owner
                .checkpoint()
                .map_err(|_| cancelled(&physical_origin))?;
            let _ambient = Cx::set_current(Some(physical_owner));
            Ok::<_, anyhow::Error>(work())
        };
        // Preserve Cx::spawn_blocking's synchronous no-pool fallback, rather
        // than creating a dedicated thread for an ineligible context.
        let value = if owner.blocking_pool_handle().is_some() {
            on_inherited_pool(&owner, physical).await?
        } else {
            physical()
        }?;
        owner.checkpoint().map_err(|_| cancelled(&origin))?;
        Ok::<_, anyhow::Error>(value)
    })
    .map_err(|error| anyhow::anyhow!("blocking storage operation failed: {error}"))?;
    // This wrapper uses native poll_join, whose receive is uninterruptible.
    // A physical panic therefore outranks a simultaneous cancellation.
    operation
        .await
        .map_err(|error| anyhow::anyhow!("blocking storage operation failed: {error}"))?
}

async fn on_inherited_pool<F, R>(cx: &Cx, work: F) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let pool = cx.blocking_pool_handle().ok_or_else(|| {
        anyhow::anyhow!(
            "blocking storage operation failed: inherited native blocking pool unavailable"
        )
    })?;
    let (done, completed) = oneshot::channel();
    let work = PoolWork {
        work: Some(work),
        done: Some(done),
    };
    #[cfg(feature = "chunk-diagnostics")]
    crate::chunk_diagnostics::storage_pool_submission();
    let _handle = pool.spawn(move || work.run());
    // This receive does not cancel the pool job. Rejection/drop closes it;
    // no closure recovery, scheduler-inline execution or new thread fallback.
    match completed.await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => Err(anyhow::anyhow!(
            "blocking storage operation failed: inherited native blocking pool rejected or discarded work"
        )),
    }
}

struct PoolWork<F, R> {
    work: Option<F>,
    done: Option<oneshot::Sender<std::thread::Result<R>>>,
}
impl<F: FnOnce() -> R, R> PoolWork<F, R> {
    fn run(mut self) {
        let result = catch_unwind(AssertUnwindSafe(self.work.take().unwrap()));
        if let Some(done) = self.done.take() {
            let _ = done.send(result);
        }
    }
}
impl<F, R> Drop for PoolWork<F, R> {
    fn drop(&mut self) {
        // Rejected or discarded work must release every callback capture
        // before closing the owner's uninterruptible completion receive.
        if let Some(work) = self.work.take() {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(work))) {
                if let Some(done) = self.done.take() {
                    let _ = done.send(Err(payload));
                }
            }
        }
        drop(self.done.take());
    }
}

fn cancelled(cx: &Cx) -> anyhow::Error {
    let reason = cx
        .cancel_reason()
        .unwrap_or_else(|| rqbit_runtime::CancelReason::user("storage owner stopped"));
    anyhow::anyhow!(
        "blocking storage operation failed: {}",
        rqbit_runtime::task::JoinError::Cancelled(reason)
    )
}
fn panicked(payload: Box<dyn Any + Send>) -> anyhow::Error {
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    anyhow::anyhow!(
        "blocking storage operation failed: {}",
        rqbit_runtime::task::JoinError::Panicked(rqbit_runtime::types::PanicPayload::new(message),)
    )
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
