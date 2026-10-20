//! Kernel-owned fsync completion, without a userspace thread stuck in fsync.
//! Dropping an unacknowledged sync never publishes a replacement: callers rename
//! only after its CQE. The kernel owns its file reference; no borrowed buffers
//! or application work can outlive cancellation. Regular payload I/O is separate.
use asupersync::{Cx, runtime::reactor::Interest};
use io_uring::{IoUring, opcode, squeue, types};
use std::{
    fs::File,
    future::poll_fn,
    io,
    os::fd::AsRawFd,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
    time::{Duration, Instant},
};

pub(crate) struct FileSync {
    ring: IoUring,
}
impl FileSync {
    #[cfg(test)]
    pub(crate) fn gated_for_test(
        file: &File,
        gate: &std::os::unix::net::UnixStream,
    ) -> io::Result<Self> {
        let mut ring = IoUring::new(4)?;
        ring.submitter()
            .register_files(&[file.as_raw_fd(), gate.as_raw_fd()])?;
        let pending = opcode::PollAdd::new(types::Fixed(1), libc::POLLIN as u32)
            .build()
            .user_data(2)
            .flags(squeue::Flags::IO_LINK);
        let flush = opcode::Fsync::new(types::Fixed(0))
            .build()
            .user_data(1)
            .flags(squeue::Flags::ASYNC);
        // SAFETY: both entries use owned fixed-file references and neither
        // operation borrows userspace memory. This gates actual kernel fsync.
        unsafe {
            ring.submission().push(&pending).unwrap();
            ring.submission().push(&flush).unwrap();
        }
        ring.submit()?;
        Ok(Self { ring })
    }
    pub(crate) fn start(file: &File) -> io::Result<Self> {
        let mut ring = IoUring::new(2)?;
        ring.submitter().register_files(&[file.as_raw_fd()])?;
        let entry = opcode::Fsync::new(types::Fixed(0))
            .build()
            .user_data(1)
            .flags(squeue::Flags::ASYNC);
        // SAFETY: fsync has no userspace buffer. The fixed-file table owns
        // the file reference through completion or cancellation, independently
        // of the caller closing/reusing its descriptor. No borrowed data escapes.
        unsafe {
            ring.submission()
                .push(&entry)
                .map_err(|_| io::Error::other("fsync submission queue full"))?;
        }
        ring.submit()?;
        Ok(Self { ring })
    }
    pub(crate) fn completion(&mut self) -> Option<io::Result<()>> {
        for entry in self.ring.completion() {
            if entry.user_data() != 1 {
                continue;
            }
            return Some(if entry.result() < 0 {
                Err(io::Error::from_raw_os_error(-entry.result()))
            } else {
                Ok(())
            });
        }
        None
    }
}

/// The owning runtime parks on the ring's CQE readiness, rather than occupying
/// its blocking pool. Cancellation drops the registration and ring immediately.
pub(crate) async fn sync_all(cx: &Cx, file: &File) -> io::Result<()> {
    cx.checkpoint()
        .map_err(|e| io::Error::new(io::ErrorKind::Interrupted, e))?;
    FileSync::start(file)?.wait(cx).await
}
/// One ring and one owned fixed-file slot for an ordered durability batch.
/// Consuming self across each wait prevents reusing a slot after cancellation.
pub(crate) struct SyncBatch(FileSync);
impl SyncBatch {
    pub(crate) fn new() -> io::Result<Self> {
        let ring = IoUring::new(2)?;
        ring.submitter().register_files(&[-1])?;
        Ok(Self(FileSync { ring }))
    }
    pub(crate) async fn file(mut self, cx: &Cx, file: &File) -> io::Result<Self> {
        cx.checkpoint()
            .map_err(|e| io::Error::new(io::ErrorKind::Interrupted, e))?;
        // The preceding CQE has already retired before this slot is replaced.
        self.0
            .ring
            .submitter()
            .register_files_update(0, &[file.as_raw_fd()])?;
        let entry = opcode::Fsync::new(types::Fixed(0))
            .build()
            .user_data(1)
            .flags(squeue::Flags::ASYNC);
        // SAFETY: the table owns the file reference; fsync borrows no buffer.
        // A cancelled future owns and drops the entire batch, never its next slot.
        unsafe {
            self.0
                .ring
                .submission()
                .push(&entry)
                .map_err(|_| io::Error::other("fsync submission queue full"))?;
        }
        self.0.ring.submit()?;
        self.0.wait_ready(cx).await?;
        Ok(self)
    }
}
impl FileSync {
    async fn wait(mut self, cx: &Cx) -> io::Result<()> {
        self.wait_ready(cx).await
    }
    async fn wait_ready(&mut self, cx: &Cx) -> io::Result<()> {
        let mut registration = cx.register_io(&self.ring, Interest::READABLE)?;
        poll_fn(|task| {
            if let Err(error) = registration.rearm(Interest::READABLE, task.waker()) {
                return Poll::Ready(Err(error));
            }
            match self.completion() {
                Some(result) => Poll::Ready(result),
                None => Poll::Pending,
            }
        })
        .await
    }
}

/// The std-only statistics owner uses poll, sleeping in the kernel. During
/// periodic saves its stop flag interrupts the wait; final saves have a fuse.
pub(crate) fn sync_all_until(
    file: &File,
    stop: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> io::Result<()> {
    if stop.is_some_and(|stop| stop.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "statistics owner stopping",
        ));
    }
    let mut sync = FileSync::start(file)?;
    loop {
        if let Some(result) = sync.completion() {
            return result;
        }
        if stop.is_some_and(|stop| stop.load(Ordering::Acquire)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "statistics owner stopping",
            ));
        }
        if deadline.is_some_and(|end| Instant::now() >= end) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "statistics checkpoint deadline",
            ));
        }
        let wait = deadline.map_or(Duration::from_millis(20), |end| {
            end.saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20))
        });
        let mut poll = libc::pollfd {
            fd: sync.ring.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll points to one initialized live pollfd. No spin: it
        // sleeps until the CQE or the bounded stop/deadline observation.
        if unsafe { libc::poll(&mut poll, 1, wait.as_millis().max(1) as i32) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        const CHILD: &str = "FBT_TEST_SYNC_BATCH_CHILD";
        if std::env::var(CHILD).as_deref() == Ok("1") {
            let root = directory("batch");
            std::fs::create_dir(&root).unwrap();
            let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
            let batch = SyncBatch::new().unwrap();
            // An actual process-local kernel fault budget: after reserving one
            // ring, every further io_uring_setup returns the observed ENOMEM.
            // This affects only this private child, never the test runner.
            let filter = [
                libc::sock_filter {
                    code: 0x20,
                    jt: 0,
                    jf: 0,
                    k: 0,
                },
                libc::sock_filter {
                    code: 0x15,
                    jt: 0,
                    jf: 1,
                    k: libc::SYS_io_uring_setup as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: 0x0005_0000 | libc::ENOMEM as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: 0x7fff_0000,
                },
            ];
            let program = libc::sock_fprog {
                len: filter.len() as u16,
                filter: filter.as_ptr() as *mut _,
            };
            // SAFETY: the kernel copies the initialized BPF array; TSYNC applies
            // the budget to this child's runtime threads as well.
            // Only io_uring_setup is denied; the child owns all its test files.
            unsafe {
                assert_eq!(
                    libc::prctl(
                        libc::PR_SET_NO_NEW_PRIVS,
                        1 as libc::c_ulong,
                        0 as libc::c_ulong,
                        0 as libc::c_ulong,
                        0 as libc::c_ulong
                    ),
                    0
                );
                assert_eq!(
                    libc::syscall(
                        libc::SYS_seccomp,
                        1 as libc::c_ulong,
                        1 as libc::c_ulong,
                        &program
                    ),
                    0
                );
            }
            runtime.block_on(async {
                let control = File::create(root.join("control")).unwrap();
                let error = sync_all(runtime.cx(), &control).await.unwrap_err();
                assert_eq!(
                    error.raw_os_error(),
                    Some(libc::ENOMEM),
                    "old per-file allocation must fail under the fault budget"
                );
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
                drop(batch);
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
                "old per-file ring: ENOMEM; reserved batch: 1000 files and directory synced and verified"
            );
            return;
        }
        use std::os::unix::fs::DirBuilderExt;
        let root = directory("batch-child");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "durable_sync::tests::batch_sync_survives_ring_allocation_failure",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .current_dir(&root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for key in [
            "HOME",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_RUNTIME_DIR",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            command.env(key, &root);
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        std::fs::remove_dir_all(root).unwrap();
        assert!(output.status.success(), "batch fault probe: {output:?}");
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
            let mut waiting = std::pin::pin!(FileSync { ring }.wait(runtime.cx()));
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
}
