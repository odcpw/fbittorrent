use super::*;
use std::{future::Future, io::Write, os::unix::net::UnixStream};
fn directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fbt-uring-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn batch_sync_survives_ring_allocation_failure() {
    super::fault::isolated(&[libc::ENOMEM, libc::EPERM], |errno| {
        let root = directory("batch");
        std::fs::create_dir(&root).unwrap();
        let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
        let batch = SyncBatch::new().unwrap();
        assert!(
            batch.0.is_some(),
            "reserve the completion batch before denying setup"
        );
        super::fault::deny_setup(errno);
        runtime.block_on(async {
            let control = File::create(root.join("control")).unwrap();
            sync_all(runtime.cx(), &control).await.unwrap();
            let mut batch = batch;
            for index in 0i32..1000 {
                let mut file = File::create(root.join(index.to_string())).unwrap();
                file.write_all(&index.to_le_bytes()).unwrap();
                batch = batch.file(runtime.cx(), &file).await.unwrap();
            }
            batch = batch
                .file(runtime.cx(), &File::open(&root).unwrap())
                .await
                .unwrap();
            assert!(
                batch.0.is_some(),
                "reserved batch continues using the same ring"
            );
        });
        runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
        for index in 0i32..1000 {
            assert_eq!(
                std::fs::read(root.join(index.to_string())).unwrap(),
                index.to_le_bytes()
            );
        }
        std::fs::remove_dir_all(root).unwrap();
        println!(
            "reserved batch: 1000 files and directory synced; per-file fallback also succeeded"
        );
    });
}

#[test]
fn fallback_syncs_files_directories_and_propagates_real_io_errors() {
    super::fault::isolated(
        &[libc::ENOMEM, libc::EPERM, libc::ENOSYS, libc::EAGAIN],
        |errno| {
            super::fault::deny_setup(errno);
            let root = super::fault::root();
            let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
            runtime.block_on(async {
                let mut batch = SyncBatch::new().unwrap();
                assert!(batch.0.is_none());
                for index in 0..8 {
                    let mut file = File::create(root.join(format!("file-{index}"))).unwrap();
                    file.write_all(format!("record {index}").as_bytes())
                        .unwrap();
                    batch = batch.file(runtime.cx(), &file).await.unwrap();
                    assert_eq!(
                        std::fs::read(root.join(format!("file-{index}"))).unwrap(),
                        format!("record {index}").as_bytes()
                    );
                }
                batch
                    .file(runtime.cx(), &File::open(&root).unwrap())
                    .await
                    .unwrap();
                let (socket, _) = UnixStream::pair().unwrap();
                let file = File::from(std::os::fd::OwnedFd::from(socket));
                assert_eq!(
                    sync_all(runtime.cx(), &file)
                        .await
                        .unwrap_err()
                        .raw_os_error(),
                    Some(libc::EINVAL),
                    "the fallback must propagate a real fsync failure"
                );
            });
            let start = Instant::now();
            runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
            assert!(start.elapsed() < Duration::from_secs(1));
        },
    );
}
#[test]
fn cancelled_fallback_never_publishes_and_drains_the_blocking_pool() {
    super::fault::isolated(&[libc::ENOMEM, libc::EPERM], |errno| {
        super::fault::deny_setup(errno);
        let root = super::fault::root();
        let target = root.join("record");
        let temporary = root.join("record.tmp");
        std::fs::write(&target, b"previous durable record").unwrap();
        let mut file = File::create(&temporary).unwrap();
        file.write_all(b"unpublished replacement").unwrap();
        let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
        let pool = runtime.cx().blocking_pool_handle().unwrap();
        let mut workers = Vec::new();
        // Occupy the configured pool so cancellation exercises a genuinely queued
        // fsync, rather than racing a completed one. Every blocker has a fuse.
        for _ in 0..crate::native_runtime::MAX_BLOCKING_THREADS {
            let (entered, ready) = std::sync::mpsc::channel();
            let (release, held) = std::sync::mpsc::channel();
            let worker = pool.spawn(move || {
                entered.send(()).unwrap();
                let _ = held.recv_timeout(Duration::from_secs(5));
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            workers.push((release, worker));
        }
        runtime.block_on(async {
            let mut transaction = std::pin::pin!(async {
                sync_all(runtime.cx(), &file).await.unwrap();
                std::fs::rename(&temporary, &target).unwrap();
            });
            poll_fn(|task| {
                assert!(transaction.as_mut().poll(task).is_pending());
                assert_eq!(pool.pending_count(), 1, "fsync queued on runtime pool");
                Poll::Ready(())
            })
            .await;
            // Drop the pending transaction at the same select cancellation
            // boundary used by ledger writes. It must never publish later.
        });
        {
            // The std statistics owner has a separate stop/deadline interface.
            // Keep its fsync queued too, proving these are real wait bounds.
            let _current = Cx::set_current(Some(runtime.cx().clone()));
            let stopping = AtomicBool::new(true);
            assert_eq!(
                sync_all_until(&file, Some(&stopping), None)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Interrupted
            );
            let began = Instant::now();
            assert_eq!(
                sync_all_until(&file, None, Some(began + Duration::from_millis(20)))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::TimedOut
            );
            assert!(began.elapsed() < Duration::from_millis(200));
        }
        for (release, worker) in workers {
            release.send(()).unwrap();
            assert!(worker.wait_timeout(Duration::from_secs(1)));
        }
        let began = Instant::now();
        runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
        assert!(began.elapsed() < Duration::from_secs(1));
        assert_eq!(std::fs::read(target).unwrap(), b"previous durable record");
        assert!(temporary.exists(), "cancelled transaction did not rename");
        println!(
            "cancelled queued fallback: clean shutdown in {:?}",
            began.elapsed()
        );
    });
}
#[test]
fn real_file_sync_completes_on_the_native_reactor() {
    let directory = directory("sync");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record");
    let mut file = File::create(&path).unwrap();
    file.write_all(b"durable record").unwrap();
    let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
    let sync = FileSync::start(&file).unwrap();
    drop(file);
    runtime.block_on(sync.wait(runtime.cx())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"durable record");
    runtime.shutdown_timeout(Duration::from_secs(1)).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn cancellation_drops_a_pending_kernel_sync_without_a_blocking_worker() {
    let directory = directory("cancel");
    std::fs::create_dir_all(&directory).unwrap();
    let file = File::create(directory.join("record")).unwrap();
    let (gate, _held) = UnixStream::pair().unwrap();
    let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
    let mut ring = IoUring::new(4).unwrap();
    ring.submitter()
        .register_files(&[file.as_raw_fd(), gate.as_raw_fd()])
        .unwrap();
    // A real kernel poll gates a linked fsync. The gate remains unreadable
    // throughout cancellation: this is not a fast already-completed sync.
    let pending = opcode::PollAdd::new(types::Fixed(1), libc::POLLIN as u32)
        .build()
        .user_data(2)
        .flags(squeue::Flags::IO_LINK);
    let flush = opcode::Fsync::new(types::Fixed(0))
        .build()
        .user_data(1)
        .flags(squeue::Flags::ASYNC);
    unsafe {
        ring.submission().push(&pending).unwrap();
        ring.submission().push(&flush).unwrap();
    }
    ring.submit().unwrap();
    let began = Instant::now();
    runtime.block_on(async {
        let mut waiting = std::pin::pin!(FileSync::Ring(ring).wait(runtime.cx()));
        poll_fn(|task| {
            assert!(waiting.as_mut().poll(task).is_pending());
            Poll::Ready(())
        })
        .await;
        // Drop the still-pending operation, just as a cancelled engine
        // select does; no path rename follows that cancelled future.
    });
    runtime
        .shutdown_timeout(Duration::from_millis(100))
        .unwrap();
    assert!(began.elapsed() < Duration::from_secs(1));
    std::fs::remove_dir_all(directory).unwrap();
}
