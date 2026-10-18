use super::{read_owned_upload, Error, MAX_MSG_LEN};
use rqbit_runtime::{sync::Semaphore, Budget, RuntimeBuilder};
use std::{
    io::Read,
    sync::{mpsc, Arc, Condvar, Mutex},
    time::Duration,
};

#[derive(Default)]
struct ReadGate {
    released: Mutex<bool>,
    changed: Condvar,
}

impl ReadGate {
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

// A failed assertion must also release the real blocking operation.
struct ReleaseReadOnDrop(Arc<ReadGate>);
impl Drop for ReleaseReadOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn runtime() -> rqbit_runtime::runtime::Runtime {
    RuntimeBuilder::new()
        .worker_threads(1)
        .blocking_threads(1, 2)
        .enable_time()
        .build()
        .unwrap()
}

#[test]
fn native_upload_read_reuses_buffer_and_allows_timer_progress() {
    let runtime = runtime();
    let owner = runtime.request_cx_with_budget(Budget::INFINITE);
    let admission = Arc::new(Semaphore::new(2));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upload.bin");
    let expected: Vec<u8> = (0..16 * 1024).map(|index| (index % 251) as u8).collect();
    std::fs::write(&path, &expected).unwrap();
    let mut file = std::fs::File::open(&path).unwrap();
    let gate = Arc::new(ReadGate::default());
    let _release_on_failure = ReleaseReadOnDrop(gate.clone());
    let held_gate = gate.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let buffer = Box::new([0xa5; MAX_MSG_LEN]);
    let original_address = buffer.as_ptr() as usize;
    let payload_end = 13 + expected.len();
    let read = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
        read_owned_upload(&cx, admission, buffer, 13..payload_end, move |payload| {
            started_tx.send(()).unwrap();
            held_gate.wait();
            file.read_exact(payload)?;
            Ok(())
        })
        .await
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let (progress_tx, progress_rx) = mpsc::channel();
    let timer = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
        rqbit_runtime::time::sleep(&cx, Duration::from_millis(10)).await;
        progress_tx.send(()).unwrap();
    });
    let timer_progress = progress_rx.recv_timeout(Duration::from_millis(500));
    gate.release();
    let buffer = runtime.block_on(read).unwrap().unwrap();
    runtime.block_on(timer).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));

    timer_progress.expect("the sole async worker must run timers during a held upload read");
    assert_eq!(buffer.as_ptr() as usize, original_address, "reuse the same allocation");
    assert_eq!(&buffer[13..payload_end], expected.as_slice());
    assert!(buffer[..13].iter().all(|byte| *byte == 0xa5));
    assert!(buffer[payload_end..].iter().all(|byte| *byte == 0xa5));
}

#[test]
fn native_upload_read_drop_keeps_admission_until_actual_read_finishes() {
    let runtime = runtime();
    let owner = runtime.request_cx_with_budget(Budget::INFINITE);
    let admission = Arc::new(Semaphore::new(2));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upload.bin");
    std::fs::write(&path, b"data").unwrap();
    let gate = Arc::new(ReadGate::default());
    let _release_on_failure = ReleaseReadOnDrop(gate.clone());
    let (started_tx, started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let mut reads = Vec::new();
    let mut stops = Vec::new();
    for _ in 0..2 {
        let mut file = std::fs::File::open(&path).unwrap();
        let held_gate = gate.clone();
        let admission = admission.clone();
        let started_tx = started_tx.clone();
        let dropped_tx = dropped_tx.clone();
        let (stop_tx, mut stop_rx) = rqbit_runtime::sync::oneshot::channel();
        stops.push(stop_tx);
        reads.push(rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
            let read = Box::pin(read_owned_upload(
                &cx,
                admission,
                Box::new([0; MAX_MSG_LEN]),
                13..17,
                move |payload| {
                    started_tx.send(()).unwrap();
                    held_gate.wait();
                    file.read_exact(payload)?;
                    Ok(())
                },
            ));
            match futures::future::select(read, Box::pin(stop_rx.recv(&cx))).await {
                futures::future::Either::Right((stopped, read)) => {
                    stopped.unwrap();
                    drop(read);
                    dropped_tx.send(()).unwrap();
                }
                futures::future::Either::Left(_) => panic!("held read finished before release"),
            }
        }));
    }
    for _ in 0..2 {
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    for stop in stops {
        stop.send(&owner, ()).unwrap();
    }
    for _ in 0..2 {
        dropped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    assert_eq!(admission.available_permits(), 0, "dropped waiters must not release active reads");

    let mut file = std::fs::File::open(&path).unwrap();
    let (third_started_tx, third_started_rx) = mpsc::channel();
    let third_admission = admission.clone();
    let third = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
        read_owned_upload(
            &cx,
            third_admission,
            Box::new([0; MAX_MSG_LEN]),
            13..17,
            move |payload| {
                third_started_tx.send(()).unwrap();
                file.read_exact(payload)?;
                Ok(())
            },
        )
        .await
    });
    assert!(matches!(
        third_started_rx.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    gate.release();
    third_started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let buffer = runtime.block_on(third).unwrap().unwrap();
    for read in reads {
        runtime.block_on(read).unwrap();
    }
    assert_eq!(&buffer[13..17], b"data");
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    assert_eq!(admission.available_permits(), 2);
}

#[test]
fn native_upload_read_keeps_read_admission_and_cancellation_errors_distinct() {
    let runtime = runtime();
    let owner = runtime.request_cx_with_budget(Budget::INFINITE);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("short-upload.bin");
    std::fs::write(&path, b"x").unwrap();
    let mut short_file = std::fs::File::open(&path).unwrap();
    let read_error = runtime.block_on(read_owned_upload(
        &owner,
        Arc::new(Semaphore::new(2)),
        Box::new([0; MAX_MSG_LEN]),
        13..17,
        move |payload| {
            short_file.read_exact(payload)?;
            Ok(())
        },
    ));
    assert!(matches!(read_error, Err(Error::ReadChunk(_))));

    let closed = Arc::new(Semaphore::new(2));
    closed.close();
    let admission_error = runtime.block_on(read_owned_upload(
        &owner,
        closed,
        Box::new([0; MAX_MSG_LEN]),
        13..17,
        |_| panic!("closed admission must not run the read"),
    ));
    assert!(matches!(admission_error, Err(Error::SemaphoreAcquireError(_))));

    owner.cancel_fast(rqbit_runtime::types::CancelKind::User);
    let cancelled = runtime.block_on(read_owned_upload(
        &owner,
        Arc::new(Semaphore::new(2)),
        Box::new([0; MAX_MSG_LEN]),
        13..17,
        |_| panic!("cancelled owner must not run the read"),
    ));
    assert!(matches!(cancelled, Err(Error::Anyhow(_))));
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}
