// Included in the unchanged actor test module to reuse its real native Fixture.
fn peer_lease_wait_for(fixture: &Fixture, predicate: impl Fn() -> bool) {
    fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            async {
                while !predicate() {
                    rqbit_runtime::task::yield_now().await;
                }
            },
        ))
        .expect("native lifecycle transition completes within the original bound");
}

#[test]
fn peer_lease_more_than_sixteen_departed_peer_contexts_reuse_capacity() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let mut receipts = Vec::new();
    for _ in 0..MAX_CACHED_GROUPS * 2 + 1 {
        let peer_spawner = spawner.clone();
        receipts.push(fixture.block_on(rqbit_runtime::task::spawn_with(
            &fixture.cx,
            move |peer| async move {
                let lease = peer_spawner.lease(&peer).await?;
                let owner = lease
                    .run(&peer, || Cx::current().unwrap().task_id())
                    .await?;
                let cached = lease.group.get().is_some();
                drop(lease); // Actual peer task ends without retaining its sender.
                Ok::<_, anyhow::Error>((cached, owner != peer.task_id()))
            },
        )));
        peer_lease_wait_for(&fixture, || {
            spawner.owners.live_groups.load(Ordering::Acquire) == 0
        });
    }
    let metadata = spawner.owners.entries.lock().unwrap().len();
    fixture.finish();
    assert!(
        receipts
            .into_iter()
            .all(|result| matches!(result, Ok(Ok((true, true)))))
    );
    assert!(metadata <= MAX_CACHED_GROUPS);
}

#[test]
fn peer_lease_departed_draining_key_keeps_its_slot_and_cannot_create_duplicate_group() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let mut regions = Vec::new();
    let mut leases = Vec::new();
    // Initialize these groups by real operations before holding admission.
    for _ in 1..MAX_CACHED_GROUPS {
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
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
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
    let weak_group = Arc::downgrade(lease.group.get().unwrap());
    drop(physical);
    drop(lease);
    let sender_gone = weak_group.upgrade().is_none();
    let same_key = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let mut same_operation = Box::pin(same_key.run(&fixture.cx, || ()));
    let same_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(same_operation.as_mut().poll(task).is_pending())
    }));
    let same_key_fallback = same_key.group.get().is_none();
    drop(same_operation);
    let saturated_region = fixture
        .block_on(
            fixture
                .cleanup
                .open_child_region(ChildRegionSpec::inherit()),
        )
        .unwrap();
    let saturated = fixture
        .block_on(spawner.lease(saturated_region.cx()))
        .unwrap();
    let mut saturated_operation = Box::pin(saturated.run(saturated_region.cx(), || ()));
    let saturated_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(saturated_operation.as_mut().poll(task).is_pending())
    }));
    let saturation_fallback = saturated.group.get().is_none();
    drop(saturated_operation);
    let live_while_held = spawner.owners.live_groups.load(Ordering::Acquire);
    let permit_while_held = spawner.semaphore().available_permits();
    fixture.gate.release();
    peer_lease_wait_for(&fixture, || {
        spawner.owners.live_groups.load(Ordering::Acquire) == MAX_CACHED_GROUPS - 1
    });
    // The same initially-fallback lease may now acquire the released slot.
    fixture.block_on(same_key.run(&fixture.cx, || ())).unwrap();
    let replacement_cached = same_key.group.get().is_some();
    let count_after_replacement = spawner.owners.live_groups.load(Ordering::Acquire);
    drop(same_key);
    drop(saturated);
    drop(leases);
    let mut closes = Vec::new();
    for region in regions.into_iter().chain([saturated_region]) {
        closes.push(fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            region.close(),
        )));
    }
    fixture.finish();
    assert!(
        sender_gone
            && same_pending
            && saturated_pending
            && same_key_fallback
            && saturation_fallback
    );
    assert_eq!(live_while_held, MAX_CACHED_GROUPS);
    assert_eq!(permit_while_held, 0);
    assert!(replacement_cached);
    assert_eq!(count_after_replacement, MAX_CACHED_GROUPS);
    assert!(
        closes
            .into_iter()
            .all(|result| matches!(result, Ok(Ok(()))))
    );
}

#[test]
fn peer_lease_departure_keeps_real_physical_write_and_exits_after_natural_sender_close() {
    use std::io::Write;
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lease-departure-write");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&fixture.cx, move || {
        let mut file = std::fs::File::create(physical_path)?;
        file.write_all(b"begun:")?;
        file.flush()?;
        entered_tx.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed")?;
        file.sync_data()?;
        finished_tx.send(()).unwrap();
        Ok::<_, std::io::Error>(())
    }));
    let was_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let shared = lease.group.get().unwrap().shared.clone();
    let weak_group = Arc::downgrade(lease.group.get().unwrap());
    let prefix = std::fs::read(&path).unwrap();
    drop(physical);
    drop(lease);
    let sender_gone = weak_group.upgrade().is_none();
    let before_release = finished_rx.recv_timeout(Duration::from_millis(50));
    let owners_while_held = shared.alive.load(Ordering::Acquire);
    let slots_while_held = spawner.owners.live_groups.load(Ordering::Acquire);
    let permit_while_held = spawner.semaphore().available_permits();
    fixture.gate.release();
    let finished = finished_rx.recv_timeout(BOUND);
    // No region cancellation/close caused this transition: sender drop alone.
    peer_lease_wait_for(&fixture, || {
        spawner.owners.live_groups.load(Ordering::Acquire) == 0
    });
    let owners_after = shared.alive.load(Ordering::Acquire);
    let full = std::fs::read(path).unwrap();
    let restored = spawner.semaphore().available_permits();
    fixture.finish();
    assert!(was_pending && sender_gone);
    assert_eq!(prefix, b"begun:");
    assert!(matches!(
        before_release,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        (owners_while_held, slots_while_held, permit_while_held),
        (1, 1, 0)
    );
    finished.expect("real physical callback finishes after explicit release");
    assert_eq!(owners_after, 0);
    assert_eq!(full, b"begun:completed");
    assert_eq!(restored, 1);
}

#[test]
fn peer_lease_region_close_drains_held_work_without_retiring_sibling_lease() {
    let mut fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let first = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let sibling_region = fixture
        .block_on(
            fixture
                .cleanup
                .open_child_region(ChildRegionSpec::inherit()),
        )
        .unwrap();
    let sibling = fixture
        .block_on(spawner.lease(sibling_region.cx()))
        .unwrap();
    let sibling_owner = fixture
        .block_on(sibling.run(sibling_region.cx(), || Cx::current().unwrap().task_id()))
        .unwrap();
    let gate = fixture.gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let mut physical = Box::pin(first.run(&fixture.cx, move || {
        entered_tx.send(()).unwrap();
        gate.wait();
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = physical.as_mut().poll(task);
        Poll::Ready(())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    drop(physical);
    drop(first);
    let region = fixture.region.take().unwrap();
    let mut close = Box::pin(region.close());
    let held_close = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        Duration::from_millis(50),
        close.as_mut(),
    ));
    let close_pending = held_close.is_err();
    fixture.gate.release();
    let closed = if close_pending {
        fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, close))
    } else {
        held_close
    };
    let sibling_after =
        fixture.block_on(sibling.run(sibling_region.cx(), || Cx::current().unwrap().task_id()));
    drop(sibling);
    let sibling_closed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        sibling_region.close(),
    ));
    let slots_after = spawner.owners.live_groups.load(Ordering::Acquire);
    fixture.finish();
    assert!(close_pending);
    assert!(matches!(closed, Ok(Ok(()))));
    assert_eq!(sibling_after.unwrap(), sibling_owner);
    assert!(matches!(sibling_closed, Ok(Ok(()))));
    assert_eq!(slots_after, 0);
}

#[test]
fn peer_lease_more_than_sixteen_idle_eligible_contexts_allocate_no_owners() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let mut regions = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..MAX_CACHED_GROUPS * 2 + 1 {
        let region = fixture
            .block_on(
                fixture
                    .cleanup
                    .open_child_region(ChildRegionSpec::inherit()),
            )
            .unwrap();
        leases.push(fixture.block_on(spawner.lease(region.cx())).unwrap());
        regions.push(region);
    }
    let all_eligible_and_empty = leases
        .iter()
        .all(|lease| lease.domain.is_some() && lease.group.get().is_none());
    let live_while_idle = spawner.owners.live_groups.load(Ordering::Acquire);
    let metadata_while_idle = spawner.owners.entries.lock().unwrap().len();
    let admission_while_idle = spawner.semaphore().available_permits();
    drop(leases);
    let mut closes = Vec::new();
    for region in regions {
        closes.push(fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            region.close(),
        )));
    }
    fixture.finish();
    assert!(
        all_eligible_and_empty,
        "idle leases carry eligible identity but create no group"
    );
    assert_eq!(
        (live_while_idle, metadata_while_idle, admission_while_idle),
        (0, 0, 1)
    );
    assert!(
        closes
            .into_iter()
            .all(|result| matches!(result, Ok(Ok(()))))
    );
}

#[test]
fn peer_lease_capacity_fallback_retries_and_reuses_after_slot_returns() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let mut regions = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..MAX_CACHED_GROUPS {
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
    let peer_region = fixture
        .block_on(
            fixture
                .cleanup
                .open_child_region(ChildRegionSpec::inherit()),
        )
        .unwrap();
    let peer = fixture.block_on(spawner.lease(peer_region.cx())).unwrap();
    let invoked = Arc::new(AtomicUsize::new(0));
    let first_invoked = invoked.clone();
    let first = fixture.block_on(peer.run(peer_region.cx(), move || {
        first_invoked.fetch_add(1, Ordering::AcqRel);
        Cx::current().unwrap().task_id()
    }));
    let first_uncached = peer.group.get().is_none();
    let count_while_full = spawner.owners.live_groups.load(Ordering::Acquire);
    drop(leases.remove(0));
    peer_lease_wait_for(&fixture, || {
        spawner.owners.live_groups.load(Ordering::Acquire) == MAX_CACHED_GROUPS - 1
    });
    let second_invoked = invoked.clone();
    let second = fixture.block_on(peer.run(peer_region.cx(), move || {
        second_invoked.fetch_add(1, Ordering::AcqRel);
        Cx::current().unwrap().task_id()
    }));
    let recovered = peer.group.get().is_some();
    let third = fixture.block_on(peer.run(peer_region.cx(), || Cx::current().unwrap().task_id()));
    let after_recovery = spawner.owners.live_groups.load(Ordering::Acquire);
    drop(peer);
    drop(leases);
    let mut closes = Vec::new();
    for region in regions.into_iter().chain([peer_region]) {
        closes.push(fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            region.close(),
        )));
    }
    fixture.finish();
    assert!(first.is_ok() && first_uncached);
    assert_eq!(count_while_full, MAX_CACHED_GROUPS);
    assert!(recovered);
    assert_eq!(
        second.unwrap(),
        third.unwrap(),
        "same lease recovers and then reuses one owner"
    );
    assert_eq!(after_recovery, MAX_CACHED_GROUPS);
    assert_eq!(
        invoked.load(Ordering::Acquire),
        2,
        "neither fallback nor recovery retries accepted F"
    );
    assert!(
        closes
            .into_iter()
            .all(|result| matches!(result, Ok(Ok(()))))
    );
}

#[test]
fn peer_lease_sender_eof_preserves_accepted_sibling_waiting_global_admission() {
    use std::io::Write;
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(2);
    let admission = spawner.semaphore();
    let (held_tx, held_rx) = mpsc::channel();
    let (registered_tx, registered_rx) = mpsc::channel();
    let (both_held_tx, both_held_rx) = mpsc::channel();
    let (register, mut register_rx) = rqbit_runtime::sync::oneshot::channel();
    let (release, mut release_rx) = rqbit_runtime::sync::oneshot::channel();
    let holder_admission = admission.clone();
    // A real native owner holds one permit, then queues for the remaining
    // permit ahead of the second accepted storage job. FIFO admission keeps
    // that storage job parked even after the first physical write completes.
    let holder = rqbit_runtime::task::spawn_with(&fixture.cleanup, move |owner| async move {
        let first = OwnedSemaphorePermit::acquire(holder_admission.clone(), &owner, 1).await?;
        held_tx.send(()).unwrap();
        register_rx.recv(&owner).await?;
        let mut next = Box::pin(OwnedSemaphorePermit::acquire(holder_admission, &owner, 1));
        let first_poll = poll_fn(|task| Poll::Ready(next.as_mut().poll(task))).await;
        registered_tx.send(first_poll.is_pending()).unwrap();
        let second = match first_poll {
            Poll::Pending => next.await?,
            Poll::Ready(result) => result?,
        };
        both_held_tx.send(()).unwrap();
        release_rx.recv(&owner).await?;
        drop(second);
        drop(first);
        Ok::<_, anyhow::Error>(())
    });
    held_rx.recv_timeout(BOUND).unwrap();
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("first");
    let second_path = directory.path().join("second");
    let first_file = first_path.clone();
    let second_file = second_path.clone();
    let gate = fixture.gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut first = Box::pin(lease.run(&fixture.cx, move || {
        let mut file = std::fs::File::create(first_file)?;
        file.write_all(b"begun:")?;
        file.flush()?;
        entered_tx.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed")?;
        file.sync_data()?;
        finished_tx.send(()).unwrap();
        Ok::<_, std::io::Error>(())
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = first.as_mut().poll(task);
        Poll::Ready(())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&first_path).unwrap();
    register.send(&fixture.cleanup, ()).unwrap();
    let reserve_was_pending = registered_rx.recv_timeout(BOUND).unwrap();
    let invoked = Arc::new(AtomicUsize::new(0));
    let second_invoked = invoked.clone();
    let (second_done_tx, second_done_rx) = mpsc::channel();
    let mut second = Box::pin(lease.run(&fixture.cx, move || {
        std::fs::write(second_file, b"accepted sibling")?;
        second_invoked.fetch_add(1, Ordering::AcqRel);
        second_done_tx.send(()).unwrap();
        Ok::<_, std::io::Error>(())
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = second.as_mut().poll(task);
        Poll::Ready(())
    }));
    let shared = lease.group.get().unwrap().shared.clone();
    peer_lease_wait_for(&fixture, || {
        admission.telemetry_snapshot(0).waiter_count == 2
    });
    let accepted = shared.requests.load(Ordering::Acquire);
    let owners_before_departure = shared.alive.load(Ordering::Acquire);
    let inbox_len = fixture.block_on(async {
        OwnedMutexGuard::lock(shared.receiver.upgrade().unwrap(), &fixture.cx)
            .await
            .unwrap()
            .len()
    });
    drop(first);
    drop(second);
    drop(lease);
    fixture.gate.release();
    let first_finished = finished_rx.recv_timeout(BOUND);
    let both_held = both_held_rx.recv_timeout(BOUND);
    // The first worker exits on sender EOF while the accepted sibling is
    // demonstrably still awaiting admission, not merely queued in the inbox.
    let eof_observed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        async {
            while shared.alive.load(Ordering::Acquire) > 1 {
                rqbit_runtime::task::yield_now().await;
            }
        },
    ));
    let owners_at_eof = shared.alive.load(Ordering::Acquire);
    let group_stopped_at_eof = shared.stopping.is_cancelled();
    let slots_while_sibling_parked = spawner.owners.live_groups.load(Ordering::Acquire);
    let permits_while_sibling_parked = admission.available_permits();
    let waiting_at_eof = admission.telemetry_snapshot(0).waiter_count;
    let second_before_release = second_done_rx.recv_timeout(Duration::from_millis(50));
    release.send(&fixture.cleanup, ()).unwrap();
    let holder_join = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        holder,
    ));
    let second_finished = second_done_rx.recv_timeout(BOUND);
    peer_lease_wait_for(&fixture, || {
        spawner.owners.live_groups.load(Ordering::Acquire) == 0
    });
    let first_bytes = std::fs::read(first_path);
    let second_bytes = std::fs::read(second_path);
    let count = invoked.load(Ordering::Acquire);
    let restored = admission.available_permits();
    eprintln!(
        "lease EOF receipt: accepted={accepted}, owners_before={owners_before_departure}, inbox={inbox_len}, reserve_pending={reserve_was_pending}, stopped_at_eof={group_stopped_at_eof}, parked_waiters={waiting_at_eof}, held_slots={slots_while_sibling_parked}, held_permits={permits_while_sibling_parked}, executions={count}"
    );
    fixture.finish();
    assert_eq!(prefix, b"begun:");
    assert!(reserve_was_pending);
    assert_eq!((accepted, owners_before_departure, inbox_len), (2, 2, 0));
    first_finished.expect("first real write completes before EOF observation");
    both_held.expect("native control owner acquires the freed permit first");
    assert!(eof_observed.is_ok(), "EOF retires its idle worker within the original bound");
    assert_eq!(owners_at_eof, 1, "accepted sibling remains a live physical owner");
    assert!(
        !group_stopped_at_eof,
        "graceful EOF must not stop accepted sibling work"
    );
    assert_eq!(
        (
            slots_while_sibling_parked,
            permits_while_sibling_parked,
            waiting_at_eof
        ),
        (1, 0, 1)
    );
    assert!(matches!(
        second_before_release,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(
        matches!(holder_join, Ok(Ok(Ok(())))),
        "holder join: {holder_join:?}"
    );
    second_finished.expect("accepted admission-parked work executes after release");
    assert_eq!(first_bytes.unwrap(), b"begun:completed");
    assert_eq!(second_bytes.unwrap(), b"accepted sibling");
    assert_eq!(
        count, 1,
        "accepted work executes once after its waiter and lease depart"
    );
    assert_eq!(restored, 2);
}

#[test]
fn peer_lease_pool_discard_drop_panic_reaches_pending_native_child_after_resources_drop() {
    use std::io::Write;
    struct ReleaseGate(Arc<Gate>);
    impl Drop for ReleaseGate {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    let fixture = Fixture::new();
    let pool = rqbit_runtime::runtime::BlockingPool::new(0, 1);
    let _release_on_failure = ReleaseGate(fixture.gate.clone());
    let cx = fixture
        .cx
        .clone()
        .with_blocking_pool_handle(Some(pool.handle()));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("held-pool-worker");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let _physical = pool.handle().spawn(move || {
        let mut file = std::fs::File::create(physical_path).unwrap();
        file.write_all(b"begun:").unwrap();
        file.flush().unwrap();
        entered_tx.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed").unwrap();
        file.sync_data().unwrap();
    });
    entered_rx.recv_timeout(BOUND).unwrap();
    let admission = Arc::new(Semaphore::new(1));
    let child_admission = admission.clone();
    let child_pool = pool.handle();
    let (disposed_tx, disposed_rx) = mpsc::channel();
    let captured = PanicOnDrop {
        disposed: disposed_tx,
        message: "discarded callback destructor panic",
    };
    let invoked = Arc::new(AtomicUsize::new(0));
    let child_invoked = invoked.clone();
    let (parked_tx, parked_rx) = mpsc::channel();
    let child = rqbit_runtime::task::spawn_with(&cx, move |owner| async move {
        let permit = OwnedSemaphorePermit::acquire(child_admission, &owner, 1)
            .await
            .unwrap();
        let (done, completed) = oneshot::channel();
        let work = PoolWork {
            work: Some(move || {
                let _permit = permit;
                let _captured = captured;
                child_invoked.fetch_add(1, Ordering::AcqRel);
            }),
            done: Some(done),
        };
        let handle = child_pool.spawn(move || work.run());
        let mut completed = Box::pin(completed);
        let first_poll = poll_fn(|task| Poll::Ready(completed.as_mut().poll(task))).await;
        parked_tx
            .send((owner, handle, first_poll.is_pending()))
            .unwrap();
        let result = match first_poll {
            Poll::Pending => completed.await,
            Poll::Ready(result) => result,
        }
        .expect("discard guard sends its original panic payload");
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    });
    let (owner, handle, actually_pending) = parked_rx.recv_timeout(BOUND).unwrap();
    let held = admission.available_permits();
    handle.cancel(); // Real queued native-pool discard, on its pool worker thread.
    owner.set_cancel_reason(rqbit_runtime::CancelReason::user(
        "simultaneous discard cancellation",
    ));
    fixture.gate.release();
    let disposed = disposed_rx.recv_timeout(BOUND);
    let joined = fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, child));
    let restored = admission.available_permits();
    let pool_joined = pool.shutdown_and_wait(BOUND);
    let full = std::fs::read(path);
    fixture.finish();
    assert!(
        actually_pending,
        "native child was really awaiting the queued pool callback"
    );
    assert_eq!(held, 0);
    disposed.expect("discarded callback captures are destroyed");
    assert!(
        matches!(&joined, Ok(Err(rqbit_runtime::task::JoinError::Panicked(payload))) if payload.message() == "discarded callback destructor panic"),
        "original panic outranks cancellation: {joined:?}"
    );
    assert_eq!(
        restored, 1,
        "physical admission releases before terminal panic publication"
    );
    assert_eq!(
        invoked.load(Ordering::Acquire),
        0,
        "discarded F never executes"
    );
    assert!(pool_joined);
    assert_eq!(full.unwrap(), b"begun:completed");
}

#[test]
fn peer_lease_last_spawn_reservation_rollback_releases_slot_with_group_retained() {
    use std::io::Write;
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(2);
    let mut regions = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..MAX_CACHED_GROUPS {
        let region = fixture
            .block_on(fixture.cleanup.open_child_region(ChildRegionSpec::inherit()))
            .unwrap();
        let lease = fixture.block_on(spawner.lease(region.cx())).unwrap();
        fixture.block_on(lease.run(region.cx(), || ())).unwrap();
        regions.push(region);
        leases.push(lease);
    }
    let victim_region = regions.remove(0);
    let victim_cx = victim_region.cx().clone();
    let victim_lease = leases.remove(0);
    let retained_group = victim_lease.group.get().unwrap().clone();
    let shared = retained_group.shared.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retirement-held-write");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let mut physical = Box::pin(victim_lease.run(&victim_cx, move || {
        let mut file = std::fs::File::create(physical_path)?;
        file.write_all(b"begun:")?;
        file.flush()?;
        entered_tx.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed")?;
        file.sync_data()?;
        Ok::<_, std::io::Error>(())
    }));
    fixture.block_on(poll_fn(|task| {
        let _ = physical.as_mut().poll(task);
        Poll::Ready(())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&path).unwrap();
    // Only scheduling is controlled: the actual Group::run CAS reserves the
    // second worker. No owner count or callback execution is synthesized.
    let (resume, pause) = oneshot::channel();
    *shared.spawn_reserved_pause.lock().unwrap() = Some(pause);
    let invoked = Arc::new(AtomicUsize::new(0));
    let callback_invoked = invoked.clone();
    let mut reservation = Box::pin(victim_lease.run(&victim_cx, move || {
        callback_invoked.fetch_add(1, Ordering::AcqRel);
    }));
    let reservation_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(reservation.as_mut().poll(task).is_pending())
    }));
    let reserved_owners = shared.alive.load(Ordering::Acquire);
    drop(physical);
    let mut closing = Box::pin(victim_region.close());
    let close_while_held = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        Duration::from_millis(50),
        closing.as_mut(),
    ));
    let held_slots = spawner.owners.live_groups.load(Ordering::Acquire);
    fixture.gate.release();
    let closed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        closing,
    ));
    let before_rollback = (
        shared.alive.load(Ordering::Acquire),
        shared.receiver.upgrade().is_none(),
        spawner.owners.live_groups.load(Ordering::Acquire),
    );
    resume.send(()).unwrap();
    let rolled_back = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        reservation.as_mut(),
    ));
    let after_rollback = (
        shared.alive.load(Ordering::Acquire),
        shared.live_slot.lock().unwrap().is_none(),
        spawner.owners.live_groups.load(Ordering::Acquire),
    );
    let replacement_region = fixture
        .block_on(fixture.cleanup.open_child_region(ChildRegionSpec::inherit()))
        .unwrap();
    let replacement = fixture.block_on(spawner.lease(replacement_region.cx())).unwrap();
    let replacement_path = directory.path().join("replacement-write");
    let replacement_file = replacement_path.clone();
    let replacement_result = fixture.block_on(replacement.run(replacement_region.cx(), move || {
        std::fs::write(replacement_file, b"quota recovered")
    }));
    let replacement_cached = replacement.group.get().is_some();
    let replacement_slots = spawner.owners.live_groups.load(Ordering::Acquire);
    let retained_through_recovery = Arc::strong_count(&retained_group) >= 2;
    let full = std::fs::read(path);
    let replacement_bytes = std::fs::read(replacement_path);
    eprintln!("retirement rollback receipt: reserved={reserved_owners}, before={before_rollback:?}, after={after_rollback:?}, cached={replacement_cached}, slots={replacement_slots}, retained={retained_through_recovery}, executions={}", invoked.load(Ordering::Acquire));
    drop(reservation);
    drop(retained_group);
    drop(victim_lease);
    drop(shared);
    drop(replacement);
    drop(leases);
    let mut closes = Vec::new();
    for region in regions.into_iter().chain([replacement_region]) {
        closes.push(fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            region.close(),
        )));
    }
    fixture.finish();
    assert_eq!(prefix, b"begun:");
    assert!(reservation_pending);
    assert_eq!(reserved_owners, 2);
    assert!(close_while_held.is_err(), "real physical write keeps close pending for 50 ms");
    assert_eq!(held_slots, MAX_CACHED_GROUPS);
    assert!(matches!(closed, Ok(Ok(()))), "actual last worker close: {closed:?}");
    assert_eq!(before_rollback, (1, true, MAX_CACHED_GROUPS));
    assert!(matches!(rolled_back, Ok(Err(_))), "unaccepted run rejects after receiver loss: {rolled_back:?}");
    assert_eq!(after_rollback, (0, true, MAX_CACHED_GROUPS - 1));
    assert_eq!(invoked.load(Ordering::Acquire), 0, "reserved run never accepts or executes F");
    assert!(matches!(replacement_result, Ok(Ok(()))));
    assert!(replacement_cached && retained_through_recovery);
    assert_eq!(replacement_slots, MAX_CACHED_GROUPS);
    assert_eq!(full.unwrap(), b"begun:completed");
    assert_eq!(replacement_bytes.unwrap(), b"quota recovered");
    assert!(closes.into_iter().all(|close| matches!(close, Ok(Ok(())))));
}
