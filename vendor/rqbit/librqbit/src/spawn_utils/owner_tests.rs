use super::*;
use crate::spawn_utils::BlockingSpawner;
use rqbit_runtime::{
    Budget,
    cx::{ChildRegion, ChildRegionSpec, cap},
    runtime::{Runtime, RuntimeBuilder},
};
use std::{
    future::{Future, poll_fn},
    sync::{Condvar, mpsc},
    task::Poll,
    time::Duration,
};

const BOUND: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

struct Fixture {
    runtime: Option<Runtime>,
    cleanup: Cx,
    region: Option<ChildRegion>,
    cx: Cx,
    gate: Arc<Gate>,
}
impl Fixture {
    fn new() -> Self {
        let runtime = RuntimeBuilder::new()
            .worker_threads(1)
            .blocking_threads(1, 2)
            .enable_time()
            .build()
            .unwrap();
        let cleanup = runtime.request_cx_with_budget(Budget::INFINITE);
        let region = runtime
            .block_on(cleanup.open_child_region(ChildRegionSpec::inherit()))
            .unwrap();
        let cx = region.cx().clone();
        Self {
            runtime: Some(runtime),
            cleanup,
            region: Some(region),
            cx,
            gate: Arc::new(Gate::default()),
        }
    }
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.as_ref().unwrap().block_on(future)
    }
    fn finish(mut self) {
        self.gate.release();
        let close = self.region.take().map(|region| {
            self.block_on(rqbit_runtime::time::timeout(
                &self.cleanup,
                BOUND,
                region.close(),
            ))
        });
        let leaks = self
            .runtime
            .as_ref()
            .unwrap()
            .diagnostics()
            .find_leaked_obligations()
            .len();
        let shutdown = self.runtime.take().unwrap().shutdown_timeout(BOUND);
        assert!(
            close
                .as_ref()
                .is_none_or(|close| matches!(close, Ok(Ok(())))),
            "region close: {close:?}"
        );
        assert_eq!(leaks, 0, "structured close leaves no reserved obligations");
        assert!(shutdown, "native pool and async workers must join");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Release physical barriers before any cleanup, even after a panic.
        self.gate.release();
        if let Some(runtime) = self.runtime.take() {
            if let Some(region) = self.region.take() {
                let _ = runtime.block_on(rqbit_runtime::time::timeout(
                    &self.cleanup,
                    BOUND,
                    region.close(),
                ));
            }
            let _ = runtime.shutdown_timeout(BOUND);
        }
    }
}

#[test]
fn old_native_blocking_oracle_classifies_panics_and_ungated_fs() {
    let fixture = Fixture::new();
    let native_panic = fixture
        .block_on(rqbit_runtime::task::spawn_blocking(
            &fixture.cx,
            || -> () {
                panic!("OLD native panic marker");
            },
        ))
        .unwrap_err();
    assert!(
        matches!(&native_panic, rqbit_runtime::task::JoinError::Panicked(payload) if payload.message() == "OLD native panic marker")
    );
    assert_eq!(
        native_panic.to_string(),
        "task panicked: panic: OLD native panic marker"
    );
    eprintln!("OLD native panic receipt: {native_panic}");
    type WithoutIo = cap::CapSet<true, true, true, false, true>;
    let no_io = {
        let _restricted = fixture.cx.restrict::<WithoutIo>().set_current_restricted();
        Cx::current().unwrap()
    };
    assert!(!no_io.capabilities().effective.io);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("OLD-free-fs-boundary");
    let old_path = path.clone();
    let (had_ambient, file) = fixture
        .block_on(rqbit_runtime::task::spawn_blocking(&no_io, move || {
            (
                Cx::current().is_some(),
                futures::executor::block_on(rqbit_runtime::fs::File::create(old_path)),
            )
        }))
        .unwrap();
    drop(file.unwrap());
    assert!(
        path.exists(),
        "OLD free filesystem API is a host boundary, not an IO-mask sandbox"
    );
    eprintln!(
        "OLD free-fs receipt: origin_io=false, raw_callback_ambient={had_ambient}, physical_file_created=true"
    );
    let origin = fixture.cx.clone();
    let precedence = fixture
        .block_on(rqbit_runtime::task::spawn_blocking(
            &fixture.cx,
            move || -> () {
                origin.set_cancel_reason(rqbit_runtime::CancelReason::user(
                    "OLD simultaneous cancel",
                ));
                panic!("OLD panic outranks cancel");
            },
        ))
        .unwrap_err();
    assert!(
        matches!(&precedence, rqbit_runtime::task::JoinError::Panicked(payload) if payload.message() == "OLD panic outranks cancel")
    );
    assert_eq!(
        precedence.to_string(),
        "task panicked: panic: OLD panic outranks cancel"
    );
    eprintln!("OLD classification receipt: {precedence}");
    fixture.finish();
}

#[test]
fn sequential_work_reuses_one_live_owner_and_preserves_generic_results() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(2);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let mut owners = Vec::new();
    for value in 0_u32..64 {
        let input = Box::new(value);
        let address = (&*input as *const u32) as usize;
        let (output, owner) = fixture
            .block_on(lease.run(&fixture.cx, move || {
                let ambient = Cx::current().expect("physical work has a live principal");
                (input, ambient.task_id())
            }))
            .unwrap();
        assert_eq!(
            (&*output as *const u32) as usize,
            address,
            "owned input is moved, never copied"
        );
        assert_eq!(*output, value);
        owners.push(owner);
    }
    assert!(owners.iter().all(|owner| *owner == owners[0]));
    assert_ne!(
        owners[0],
        fixture.cx.task_id(),
        "physical work belongs to the live owner"
    );
    let nested = fixture
        .block_on(lease.run(&fixture.cx, || {
            Err::<(), _>(std::io::Error::other("physical I/O failure"))
        }))
        .unwrap();
    assert_eq!(nested.unwrap_err().to_string(), "physical I/O failure");
    let panic = fixture
        .block_on(lease.run(&fixture.cx, || -> () { panic!("physical panic marker") }))
        .unwrap_err();
    assert_eq!(
        panic.to_string(),
        "blocking storage operation failed: task panicked: panic: physical panic marker"
    );
    let after_panic = fixture
        .block_on(lease.run(&fixture.cx, || Cx::current().unwrap().task_id()))
        .unwrap();
    assert_eq!(
        after_panic, owners[0],
        "one physical panic must not kill the owner"
    );
    let group = fixture
        .block_on(spawner.owners.group(&fixture.cx))
        .unwrap()
        .unwrap();
    assert_eq!(
        group.shared.alive.load(Ordering::Acquire),
        1,
        "idle second owner is never allocated"
    );
    assert_eq!(group.shared.requests.load(Ordering::Acquire), 0);
    assert_eq!(spawner.semaphore().available_permits(), 2);
    fixture.finish();
}

#[test]
fn dropping_an_accepted_queued_waiter_preserves_its_work_exactly_once() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let group = fixture
        .block_on(spawner.owners.group(&fixture.cx))
        .unwrap()
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let gate = fixture.gate.clone();
    let mut first = Box::pin(lease.run(&fixture.cx, move || {
        entered_tx.send(()).unwrap();
        gate.wait();
        1_u32
    }));
    let first_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(first.as_mut().poll(task).is_pending())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let executed = Arc::new(AtomicUsize::new(0));
    let orphan_executed = executed.clone();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut orphan = Box::pin(lease.run(&fixture.cx, move || {
        orphan_executed.fetch_add(1, Ordering::AcqRel);
        finished_tx.send(()).unwrap();
    }));
    let orphan_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(orphan.as_mut().poll(task).is_pending())
    }));
    let queued = fixture.block_on(async {
        let receiver = group.shared.receiver.upgrade().unwrap();
        OwnedMutexGuard::lock(receiver, &fixture.cx)
            .await
            .unwrap()
            .len()
    });
    drop(orphan);
    let reserved = spawner.semaphore().available_permits();
    let before_release = executed.load(Ordering::Acquire);
    fixture.gate.release();
    let first_result =
        fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, first));
    let orphan_finished = finished_rx.recv_timeout(BOUND);
    // Await the physical owner's next receive so terminal bookkeeping ends.
    let next = fixture.block_on(lease.run(&fixture.cx, || 2_u32));
    let count = executed.load(Ordering::Acquire);
    fixture.finish();
    assert!(
        first_pending && orphan_pending,
        "both call futures were really polled"
    );
    assert_eq!(
        queued, 1,
        "prove the orphan was accepted into the bounded inbox"
    );
    assert_eq!(reserved, 0, "physical work retains admission");
    assert_eq!(before_release, 0, "queued work cannot execute early");
    assert!(
        matches!(first_result, Ok(Ok(1))),
        "first physical result: {first_result:?}"
    );
    orphan_finished.expect("dropping the reply waiter preserves accepted F");
    assert_eq!(count, 1, "accepted work must execute once");
    assert_eq!(next.unwrap(), 2);
}

#[test]
fn individual_queued_cancel_is_deferred_but_region_cancel_disposes_queue_before_physical_end() {
    struct Captured(mpsc::Sender<()>);
    impl Drop for Captured {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let mut fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let group = fixture
        .block_on(spawner.owners.group(&fixture.cx))
        .unwrap()
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let gate = fixture.gate.clone();
    let mut physical = Box::pin(lease.run(&fixture.cx, move || {
        entered_tx.send(()).unwrap();
        gate.wait();
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = physical.as_mut().poll(task);
        Poll::Ready(())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let (disposed_tx, disposed_rx) = mpsc::channel();
    let captured = Captured(disposed_tx);
    let invoked = Arc::new(AtomicUsize::new(0));
    let queued_invoked = invoked.clone();
    let mut queued = Box::pin(lease.run(&fixture.cx, move || {
        let _captured = captured;
        queued_invoked.fetch_add(1, Ordering::AcqRel);
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = queued.as_mut().poll(task);
        Poll::Ready(())
    }));
    let accepted = fixture.block_on(async {
        OwnedMutexGuard::lock(group.shared.receiver.upgrade().unwrap(), &fixture.cx)
            .await
            .unwrap()
            .len()
    });
    fixture
        .cx
        .set_cancel_reason(rqbit_runtime::CancelReason::user(
            "individual queued caller stopped",
        ));
    let individually_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(queued.as_mut().poll(task).is_pending())
    }));
    let individually_retained = matches!(disposed_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    fixture
        .region
        .as_ref()
        .unwrap()
        .cancel(rqbit_runtime::CancelReason::user("whole region stopped"))
        .unwrap();
    let disposed_while_held = disposed_rx.recv_timeout(BOUND);
    let permit_while_held = spawner.semaphore().available_permits();
    // Drop the call futures only after collecting the queued-cancellation
    // evidence, so region-close proof can borrow the fixture independently.
    drop(physical);
    drop(queued);
    let region = fixture.region.take().unwrap();
    let mut close = Box::pin(region.close());
    let close_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(close.as_mut().poll(task).is_pending())
    }));
    fixture.gate.release();
    let closed = fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, close));
    let count = invoked.load(Ordering::Acquire);
    let restored = spawner.semaphore().available_permits();
    fixture.finish();
    assert_eq!(
        accepted, 1,
        "prove the cancelled closure was accepted into the inbox"
    );
    assert!(
        individually_pending && individually_retained,
        "individual cancellation is explicitly deferred behind physical admission"
    );
    disposed_while_held
        .expect("region cancellation must dispose queued captures before physical release");
    assert_eq!(
        permit_while_held, 0,
        "started physical work still owns its permit"
    );
    assert!(
        close_pending,
        "region quiescence cannot precede physical completion"
    );
    assert!(matches!(closed, Ok(Ok(()))), "region close: {closed:?}");
    assert_eq!(count, 0, "origin-cancelled queued F never enters");
    assert_eq!(restored, 1);
}

#[test]
fn a_physical_panic_outranks_simultaneous_origin_cancellation() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let origin = fixture.cx.clone();
    let failure = fixture
        .block_on(lease.run(&fixture.cx, move || -> () {
            origin.set_cancel_reason(rqbit_runtime::CancelReason::user(
                "cancel at physical entry",
            ));
            panic!("panic after cancellation");
        }))
        .unwrap_err();
    fixture.finish();
    assert_eq!(
        failure.to_string(),
        "blocking storage operation failed: task panicked: panic: panic after cancellation"
    );
}

#[test]
fn a_rejected_native_pool_never_executes_work_inline() {
    let fixture = Fixture::new();
    let pool = rqbit_runtime::runtime::BlockingPool::new(0, 1);
    let cx = fixture
        .cx
        .clone()
        .with_blocking_pool_handle(Some(pool.handle()));
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&cx)).unwrap();
    // Warm the real cached owner while this explicitly attached pool is open.
    fixture.block_on(lease.run(&cx, || ())).unwrap();
    pool.shutdown();
    let executed = Arc::new(AtomicUsize::new(0));
    let physical_executed = executed.clone();
    let failure = fixture.block_on(lease.run(&cx, move || {
        physical_executed.fetch_add(1, Ordering::AcqRel);
    }));
    let count = executed.load(Ordering::Acquire);
    let permits = spawner.semaphore().available_permits();
    let pool_join = pool.shutdown_and_wait(BOUND);
    fixture.finish();
    assert!(failure.is_err(), "closed native pool fails explicitly");
    assert_eq!(
        count, 0,
        "rejection must not execute F on the scheduler or a fallback thread"
    );
    assert_eq!(permits, 1);
    assert!(pool_join);
}

#[test]
fn warmed_owner_cannot_broaden_denied_io_or_spawn() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(2);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    fixture.block_on(lease.run(&fixture.cx, || ())).unwrap();
    type WithoutIo = cap::CapSet<true, true, true, false, true>;
    let no_io = {
        let _restricted = fixture.cx.restrict::<WithoutIo>().set_current_restricted();
        Cx::current().unwrap()
    };
    assert!(no_io.reusable_region_owner_key().is_none());
    let directory = tempfile::tempdir().unwrap();
    let allowed_path = directory.path().join("allowed-real-write");
    let physical_allowed_path = allowed_path.clone();
    fixture
        .block_on(lease.run(&fixture.cx, move || {
            assert!(Cx::current().unwrap().capabilities().effective.io);
            std::fs::write(physical_allowed_path, b"full-authority physical write")
        }))
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(allowed_path).unwrap(),
        b"full-authority physical write"
    );
    // The executed OLD oracle establishes that the free fs API is a host
    // boundary with no IO-mask check. Preserve a separate NEW receipt; neither
    // bare Fs nor arbitrary F is being claimed to be an OS sandbox here.
    let boundary_path = directory.path().join("NEW-free-fs-boundary");
    let physical_boundary_path = boundary_path.clone();
    let file = fixture
        .block_on(lease.run(&no_io, move || {
            let ambient = Cx::current().unwrap();
            assert!(!ambient.capabilities().effective.io);
            assert!(
                ambient.io().is_none(),
                "fallback must preserve the denied capability getter"
            );
            futures::executor::block_on(rqbit_runtime::fs::File::create(physical_boundary_path))
        }))
        .unwrap()
        .unwrap();
    drop(file);
    assert!(boundary_path.exists());
    eprintln!(
        "NEW free-fs receipt: ambient_io=false, io_capability=None, physical_file_created=true (existing host boundary)"
    );
    let path = directory.path().join("must-not-exist");
    let denied_path = path.clone();
    let io = fixture
        .block_on(lease.run(&no_io, move || {
            let ambient = Cx::current().unwrap();
            assert!(!ambient.capabilities().effective.io);
            // This F explicitly requests the public capability before the
            // native write, matching the Cx::io API's documented contract.
            let _io = ambient.io().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "caller denied the requested I/O capability",
                )
            })?;
            futures::executor::block_on(rqbit_runtime::fs::File::create(denied_path))
        }))
        .unwrap();
    assert_eq!(io.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
    assert!(
        !path.exists(),
        "the capability-requesting closure is denied before its physical write"
    );
    type WithoutSpawn = cap::CapSet<false, true, true, true, true>;
    let no_spawn = {
        let _restricted = fixture
            .cx
            .restrict::<WithoutSpawn>()
            .set_current_restricted();
        Cx::current().unwrap()
    };
    let called = Arc::new(AtomicUsize::new(0));
    let physical_called = called.clone();
    let refusal = fixture.block_on(lease.run(&no_spawn, move || {
        physical_called.fetch_add(1, Ordering::AcqRel);
    }));
    let count = called.load(Ordering::Acquire);
    fixture.finish();
    assert!(refusal.is_err());
    assert_eq!(
        count, 0,
        "denied spawn cannot borrow a warmed owner's authority"
    );
}

#[test]
fn region_cache_saturation_uses_safe_fallback_and_closing_one_region_keeps_siblings() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(2);
    let mut regions = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..MAX_CACHED_GROUPS + 2 {
        let region = fixture
            .block_on(
                fixture
                    .cleanup
                    .open_child_region(ChildRegionSpec::inherit()),
            )
            .unwrap();
        let lease = fixture.block_on(spawner.lease(region.cx())).unwrap();
        fixture.block_on(lease.run(region.cx(), || ())).unwrap();
        leases.push(lease);
        regions.push(region);
    }
    let entries = spawner.owners.entries.lock().unwrap();
    let groups = entries.len();
    let owners: usize = entries
        .iter()
        .map(|entry| {
            entry
                .group
                .result
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .1
                .upgrade()
                .unwrap()
                .alive
                .load(Ordering::Acquire)
        })
        .sum();
    drop(entries);
    let sibling = regions[1].cx().clone();
    let lease = fixture.block_on(spawner.lease(&sibling)).unwrap();
    let sibling_owner = fixture
        .block_on(lease.run(&sibling, || Cx::current().unwrap().task_id()))
        .unwrap();
    let first = regions.remove(0);
    drop(leases.remove(0));
    let first_close = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        first.close(),
    ));
    let sibling_after = fixture
        .block_on(lease.run(&sibling, || Cx::current().unwrap().task_id()))
        .unwrap();
    let mut closes = Vec::new();
    for region in regions {
        closes.push(fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            region.close(),
        )));
    }
    fixture.finish();
    assert_eq!(groups, MAX_CACHED_GROUPS);
    assert_eq!(
        owners, MAX_CACHED_GROUPS,
        "only one owner per sequential region"
    );
    assert_eq!(
        sibling_owner, sibling_after,
        "closing one region must not stop another owner"
    );
    assert!(matches!(first_close, Ok(Ok(()))));
    assert!(closes.into_iter().all(|close| matches!(close, Ok(Ok(())))));
}

struct PanicOnDrop {
    disposed: mpsc::Sender<()>,
    message: &'static str,
}
impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        let _ = self.disposed.send(());
        panic!("{}", self.message);
    }
}

#[test]
fn queued_capture_drop_panic_does_not_release_a_running_physical_owner() {
    use std::io::Write;

    const HELD_OBSERVATION: Duration = Duration::from_millis(50);
    let mut fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let group = fixture
        .block_on(spawner.owners.group(&fixture.cx))
        .unwrap()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("held-write");
    let physical_path = path.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let gate = fixture.gate.clone();
    let mut physical = Box::pin(lease.run(&fixture.cx, move || {
        let mut file = std::fs::File::create(physical_path)?;
        file.write_all(b"begun:")?;
        file.flush()?;
        entered_tx.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed")?;
        file.sync_data()?;
        finished_tx.send(()).unwrap();
        Ok::<(), std::io::Error>(())
    }));
    let physical_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&path).unwrap();
    let (disposed_tx, disposed_rx) = mpsc::channel();
    let captured = PanicOnDrop {
        disposed: disposed_tx,
        message: "queued capture destructor panic",
    };
    let invoked = Arc::new(AtomicUsize::new(0));
    let queued_invoked = invoked.clone();
    let mut queued = Box::pin(lease.run(&fixture.cx, move || {
        let _captured = captured;
        queued_invoked.fetch_add(1, Ordering::AcqRel);
    }));
    let queued_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(queued.as_mut().poll(task).is_pending())
    }));
    let accepted = fixture.block_on(async {
        OwnedMutexGuard::lock(group.shared.receiver.upgrade().unwrap(), &fixture.cx)
            .await
            .unwrap()
            .len()
    });
    fixture
        .region
        .as_ref()
        .unwrap()
        .cancel(rqbit_runtime::CancelReason::user("dispose queued callback"))
        .unwrap();
    let disposed_while_held = disposed_rx.recv_timeout(BOUND);
    let queued_result = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        queued.as_mut(),
    ));
    let permit_while_held = spawner.semaphore().available_permits();
    let owners_while_held = group.shared.alive.load(Ordering::Acquire);
    let physical_still_held = matches!(finished_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    drop(physical);
    drop(queued);
    let region = fixture.region.take().unwrap();
    let mut close = Box::pin(region.close());
    let close_while_held = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        HELD_OBSERVATION,
        close.as_mut(),
    ));
    let close_pending = close_while_held.is_err();
    // Collect the negative evidence before releasing the real file write,
    // including on OLD. Never leave a blocked pool thread after an assertion.
    fixture.gate.release();
    let finished = finished_rx.recv_timeout(BOUND);
    let closed = if close_pending {
        fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, close))
    } else {
        close_while_held
    };
    let full_bytes = std::fs::read(path).unwrap();
    let restored = spawner.semaphore().available_permits();
    let count = invoked.load(Ordering::Acquire);
    eprintln!(
        "capture-drop drain receipt: queued={queued_result:?}, permit_held={permit_while_held}, owners_held={owners_while_held}, close_pending_50ms={close_pending}, physical_still_held={physical_still_held}"
    );
    fixture.finish();
    assert!(
        physical_pending && queued_pending,
        "both operations were polled"
    );
    assert_eq!(prefix, b"begun:", "a real write has physically started");
    assert_eq!(
        accepted, 1,
        "the panicking capture was accepted into the inbox"
    );
    disposed_while_held.expect("region cancellation disposes queued F while physical work is held");
    assert!(
        matches!(&queued_result, Ok(Err(error)) if error.to_string() == "blocking storage operation failed: task panicked: panic: queued capture destructor panic"),
        "capture panic must outrank cancellation: {queued_result:?}"
    );
    assert_eq!(permit_while_held, 0, "physical work retains admission");
    assert_eq!(
        owners_while_held, 1,
        "the draining physical owner remains alive"
    );
    assert!(physical_still_held, "physical work has not completed early");
    assert!(
        close_pending,
        "region close must remain pending for the full held interval"
    );
    finished.expect("released physical work finishes within the original bound");
    assert!(matches!(closed, Ok(Ok(()))), "region close: {closed:?}");
    assert_eq!(full_bytes, b"begun:completed");
    assert_eq!(restored, 1);
    assert_eq!(count, 0, "disposed queued F never executes");
}

#[test]
fn rejected_capture_drop_panic_settles_as_panicked_and_owner_survives() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let owner = fixture
        .block_on(lease.run(&fixture.cx, || Cx::current().unwrap().task_id()))
        .unwrap();
    let group = fixture
        .block_on(spawner.owners.group(&fixture.cx))
        .unwrap()
        .unwrap();
    type WithoutIo = cap::CapSet<true, true, true, false, true>;
    let no_io = {
        let _restricted = fixture.cx.restrict::<WithoutIo>().set_current_restricted();
        Cx::current().unwrap()
    };
    let (disposed_tx, disposed_rx) = mpsc::channel();
    let captured = PanicOnDrop {
        disposed: disposed_tx,
        message: "rejected capture destructor panic",
    };
    let invoked = Arc::new(AtomicUsize::new(0));
    let rejected_invoked = invoked.clone();
    // Send directly through this already-warmed group to exercise its
    // pre-entry authority rejection, rather than public fallback selection.
    let rejected = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        group.run(&no_io, move || {
            let _captured = captured;
            rejected_invoked.fetch_add(1, Ordering::AcqRel);
        }),
    ));
    let disposed = disposed_rx.recv_timeout(BOUND);
    let after = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        lease.run(&fixture.cx, || Cx::current().unwrap().task_id()),
    ));
    let requests = group.shared.requests.load(Ordering::Acquire);
    let permits = spawner.semaphore().available_permits();
    fixture.finish();
    disposed.expect("rejected F is disposed before its terminal reply");
    assert!(
        matches!(&rejected, Ok(Err(error)) if error.to_string() == "blocking storage operation failed: task panicked: panic: rejected capture destructor panic"),
        "capture panic must outrank pre-entry rejection: {rejected:?}"
    );
    assert_eq!(invoked.load(Ordering::Acquire), 0);
    assert!(
        matches!(after, Ok(Ok(id)) if id == owner),
        "same owner survives: {after:?}"
    );
    assert_eq!(requests, 0);
    assert_eq!(permits, 1);
}

// Peer lifetime lease regressions exercise real native ownership and files.
include!("peer_lease_tests.rs");

#[path = "owned_frame_tests.rs"]
mod owned_frame_tests;

#[path = "coalescing_native_tests.rs"]
mod coalescing_native_tests;
