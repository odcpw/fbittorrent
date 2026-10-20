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
impl FileSync {
    async fn wait(mut self, cx: &Cx) -> io::Result<()> {
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
