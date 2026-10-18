// Native storage ownership regressions use the actual application runtime:
// one scheduler worker and the existing 1..2 blocking pool. The barrier holds
// a real file write while its async waiter is dropped.
use super::NativeRuntime;
use asupersync::{cx::ChildRegionSpec, types::CancelReason, Cx};
use librqbit::spawn_utils::BlockingSpawner;
use std::{
    future::{poll_fn, Future},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    task::Poll,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const START_AND_FINISH_BOUND: Duration = Duration::from_secs(2);
const HELD_OBSERVATION: Duration = Duration::from_millis(50);
const FIRST_BYTES: &[u8] = b"begun:completed";
const SECOND_BYTES: &[u8] = b"second physical write";

#[derive(Default)]
struct WriteGate {
    released: Mutex<bool>,
    changed: Condvar,
}
impl WriteGate {
    fn wait(&self) {
        let mut released = self.released.lock().unwrap_or_else(|error| error.into_inner());
        while !*released {
            released = self
                .changed
                .wait(released)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
    fn release(&self) {
        *self.released.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.changed.notify_all();
    }
}

struct Fixture {
    runtime: Option<NativeRuntime>,
    gate: Arc<WriteGate>,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-storage-ownership-{}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        Self {
            runtime: Some(NativeRuntime::new().unwrap()),
            gate: Arc::new(WriteGate::default()),
            directory,
        }
    }
    fn runtime(&self) -> &NativeRuntime {
        self.runtime.as_ref().unwrap()
    }
    fn shutdown(&mut self) -> anyhow::Result<()> {
        self.gate.release();
        self.runtime.take().unwrap().shutdown_timeout(START_AND_FINISH_BOUND)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // A setup error or failed assertion must release the actual file
        // operation before NativeRuntime attempts its bounded physical drain.
        self.gate.release();
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.shutdown_timeout(START_AND_FINISH_BOUND);
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn held_write(
    path: &Path,
    gate: Arc<WriteGate>,
    entered: mpsc::Sender<()>,
    finished: mpsc::Sender<()>,
    physically_finished: Arc<AtomicBool>,
) -> impl FnOnce() -> io::Result<()> + Send + 'static {
    let path = path.to_owned();
    move || {
        let mut file = std::fs::File::create(path)?;
        file.write_all(b"begun:")?;
        file.flush()?;
        // Publish after a real file mutation, not merely after polling a
        // future or entering the closure. The remainder is still outstanding.
        entered.send(()).unwrap();
        gate.wait();
        file.write_all(b"completed")?;
        file.sync_data()?;
        physically_finished.store(true, Ordering::Release);
        finished.send(()).unwrap();
        Ok(())
    }
}

fn spawn_droppable_waiter<F>(
    owner: &Cx,
    spawner: BlockingSpawner,
    write: F,
) -> (
    rqbit_runtime::task::JoinHandle<()>,
    rqbit_runtime::sync::oneshot::Sender<()>,
    mpsc::Receiver<()>,
)
where
    F: FnOnce() -> io::Result<()> + Send + 'static,
{
    let (stop_tx, mut stop_rx) = rqbit_runtime::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let waiter = rqbit_runtime::task::spawn_with(owner, move |cx| async move {
        let mut operation = Box::pin(spawner.run(&cx, write));
        let mut stop = Box::pin(stop_rx.recv(&cx));
        poll_fn(|task| {
            if let Poll::Ready(result) = stop.as_mut().poll(task) {
                result.unwrap();
                return Poll::Ready(());
            }
            if operation.as_mut().poll(task).is_ready() {
                panic!("held write returned before the explicit waiter-drop command");
            }
            Poll::Pending
        })
        .await;
        drop(stop);
        drop(operation);
        // The caller has actually dropped run's future and its stack locals.
        dropped_tx.send(()).unwrap();
    });
    (waiter, stop_tx, dropped_rx)
}

fn reserved_obligations(runtime: &NativeRuntime) -> usize {
    // This is the runtime's documented heuristic snapshot. Structured region
    // close and successful runtime teardown remain separate required evidence.
    let shared = runtime.handle.shared.read().unwrap();
    shared.as_ref().unwrap().diagnostics().find_leaked_obligations().len()
}

#[test]
fn native_storage_waiter_drop_keeps_admission_until_physical_write_finishes() {
    let mut fixture = Fixture::new();
    let runtime = fixture.runtime();
    let region = runtime
        .block_on(runtime.cx().open_child_region(ChildRegionSpec::inherit()))
        .unwrap();
    let owner = region.cx().clone();
    let spawner = BlockingSpawner::new(1);
    let admission = spawner.semaphore();
    let first_path = fixture.directory.join("first.bin");
    let second_path = fixture.directory.join("second.bin");
    let physically_finished = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let (waiter, stop, dropped) = spawn_droppable_waiter(
        &owner,
        spawner.clone(),
        held_write(
            &first_path,
            fixture.gate.clone(),
            entered_tx,
            finished_tx,
            physically_finished.clone(),
        ),
    );
    entered_rx.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    assert_eq!(std::fs::read(&first_path).unwrap(), b"begun:");
    assert_eq!(admission.available_permits(), 0, "prove initial admission is held");
    stop.send(runtime.cx(), ()).unwrap();
    dropped.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    let permits_after_waiter_drop = admission.available_permits();

    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let (second_polled_tx, second_polled_rx) = mpsc::channel();
    let second_file = second_path.clone();
    let second = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
        let mut write = Box::pin(spawner.run(&cx, move || {
            let mut file = std::fs::File::create(second_file)?;
            file.write_all(SECOND_BYTES)?;
            second_entered_tx.send(()).unwrap();
            file.sync_data()?;
            Ok::<(), io::Error>(())
        }));
        let mut first_poll = Some(second_polled_tx);
        poll_fn(|task| {
            let result = write.as_mut().poll(task);
            if let Some(polled) = first_poll.take() {
                polled.send(result.is_pending()).unwrap();
            }
            result
        })
        .await
    });
    let second_started_pending = second_polled_rx.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    let second_before_release = second_entered_rx.recv_timeout(HELD_OBSERVATION);
    let first_still_physical = !physically_finished.load(Ordering::Acquire);

    // Collect the negative evidence first; report failures only after release,
    // joins, region close and runtime shutdown, including the expected RED.
    fixture.gate.release();
    let first_finished = finished_rx.recv_timeout(START_AND_FINISH_BOUND);
    let second_after_release = if second_before_release.is_ok() {
        Ok(())
    } else {
        second_entered_rx.recv_timeout(START_AND_FINISH_BOUND)
    };
    let waiter_join = runtime.block_on(rqbit_runtime::time::timeout(
        runtime.cx(), START_AND_FINISH_BOUND, waiter,
    ));
    let second_join = runtime.block_on(rqbit_runtime::time::timeout(
        runtime.cx(), START_AND_FINISH_BOUND, second,
    ));
    let region_close = runtime.block_on(rqbit_runtime::time::timeout(
        runtime.cx(), START_AND_FINISH_BOUND, region.close(),
    ));
    let leaks_after_close = reserved_obligations(runtime);
    let restored_permits = admission.available_permits();
    let first_bytes = std::fs::read(first_path);
    let second_bytes = std::fs::read(second_path);
    let shutdown = fixture.shutdown();

    first_finished.expect("the released physical write must finish");
    second_after_release.expect("second physical write must run after admission is released");
    assert!(matches!(waiter_join, Ok(Ok(()))), "waiter join: {waiter_join:?}");
    assert!(matches!(second_join, Ok(Ok(Ok(Ok(()))))), "second join: {second_join:?}");
    assert!(matches!(region_close, Ok(Ok(()))), "region close: {region_close:?}");
    assert_eq!(leaks_after_close, 0, "no reserved obligations after structured close");
    assert_eq!(restored_permits, 1, "physical completion must release admission");
    assert_eq!(first_bytes.unwrap(), FIRST_BYTES);
    assert_eq!(second_bytes.unwrap(), SECOND_BYTES);
    shutdown.expect("native runtime must physically drain and join its workers");
    assert!(second_started_pending, "prove the second caller was actually polled");
    assert!(first_still_physical, "negative probe must occur while the first write is held");
    assert_eq!(
        permits_after_waiter_drop, 0,
        "dropping the async waiter must not release admission owned by a running write",
    );
    assert!(
        matches!(second_before_release, Err(mpsc::RecvTimeoutError::Timeout)),
        "second physical operation entered before release: {second_before_release:?}",
    );
}

#[test]
fn native_storage_region_close_receipt_follows_physical_write_completion() {
    let mut fixture = Fixture::new();
    let runtime = fixture.runtime();
    let region = runtime
        .block_on(runtime.cx().open_child_region(ChildRegionSpec::inherit()))
        .unwrap();
    let owner = region.cx().clone();
    let spawner = BlockingSpawner::new(1);
    let admission = spawner.semaphore();
    let path = fixture.directory.join("close-held.bin");
    let physically_finished = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let (waiter, stop, dropped) = spawn_droppable_waiter(
        &owner,
        spawner,
        held_write(
            &path,
            fixture.gate.clone(),
            entered_tx,
            finished_tx,
            physically_finished.clone(),
        ),
    );
    entered_rx.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"begun:");
    stop.send(runtime.cx(), ()).unwrap();
    dropped.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    region.cancel(CancelReason::user("storage close fixture")).unwrap();

    let (close_polled_tx, close_polled_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let physical_witness = physically_finished.clone();
    let close = rqbit_runtime::task::spawn_with(runtime.cx(), move |_| async move {
        let mut close = Box::pin(region.close());
        let mut first_poll = Some(close_polled_tx);
        let result = poll_fn(|task| {
            let result = close.as_mut().poll(task);
            if let Some(polled) = first_poll.take() {
                polled.send(result.is_pending()).unwrap();
            }
            result
        })
        .await;
        closed_tx
            .send((result.is_ok(), physical_witness.load(Ordering::Acquire)))
            .unwrap();
        result
    });
    let close_started_pending = close_polled_rx.recv_timeout(START_AND_FINISH_BOUND).unwrap();
    let close_before_release = closed_rx.recv_timeout(HELD_OBSERVATION);
    let first_still_physical = !physically_finished.load(Ordering::Acquire);
    fixture.gate.release();
    let first_finished = finished_rx.recv_timeout(START_AND_FINISH_BOUND);
    let close_receipt = match close_before_release.as_ref() {
        Ok(receipt) => Ok(*receipt),
        Err(_) => closed_rx.recv_timeout(START_AND_FINISH_BOUND),
    };
    let waiter_join = runtime.block_on(rqbit_runtime::time::timeout(
        runtime.cx(), START_AND_FINISH_BOUND, waiter,
    ));
    let close_join = runtime.block_on(rqbit_runtime::time::timeout(
        runtime.cx(), START_AND_FINISH_BOUND, close,
    ));
    let leaks_after_close = reserved_obligations(runtime);
    let restored_permits = admission.available_permits();
    let bytes = std::fs::read(path);
    let shutdown = fixture.shutdown();

    first_finished.expect("physical write must finish after release");
    assert!(matches!(waiter_join, Ok(Ok(()))), "waiter join: {waiter_join:?}");
    assert!(matches!(close_join, Ok(Ok(Ok(())))), "close join: {close_join:?}");
    assert_eq!(close_receipt.unwrap(), (true, true), "close must follow the physical end marker");
    assert_eq!(leaks_after_close, 0, "no reserved obligations after structured close");
    assert_eq!(restored_permits, 1);
    assert_eq!(bytes.unwrap(), FIRST_BYTES);
    shutdown.expect("native runtime must physically drain and join its workers");
    assert!(close_started_pending, "prove region.close was polled while storage remained active");
    assert!(first_still_physical, "probe must occur while the physical operation is held");
    assert!(
        matches!(close_before_release, Err(mpsc::RecvTimeoutError::Timeout)),
        "region close produced a receipt before the physical operation finished: {close_before_release:?}",
    );
}
