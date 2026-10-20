//! Ordered durability, using io_uring when available and ordinary fsync on the
//! runtime blocking path when ring setup is unavailable or resource constrained.
//! Both paths own their file reference. Cancellation can leave an already-started
//! fsync running, but never a rename/publication: those stay with the caller.
use asupersync::{runtime::reactor::Interest, Cx};
use io_uring::{opcode, squeue, types, IoUring};
use std::{
    fs::File,
    future::{poll_fn, Future},
    io,
    os::fd::AsRawFd,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
    time::{Duration, Instant},
};

pub(crate) enum FileSync {
    Ring(IoUring),
    Blocking(Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send>>>),
}

fn unavailable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOMEM | libc::EPERM | libc::ENOSYS | libc::EAGAIN)
    )
}

fn checkpoint(cx: &Cx) -> io::Result<()> {
    cx.checkpoint()
        .map_err(|e| io::Error::new(io::ErrorKind::Interrupted, e))
}

fn completion(ring: &mut IoUring) -> Option<io::Result<()>> {
    for entry in ring.completion() {
        if entry.user_data() == 1 {
            return Some(if entry.result() < 0 {
                Err(io::Error::from_raw_os_error(-entry.result()))
            } else {
                Ok(())
            });
        }
    }
    None
}

async fn wait_ring(cx: &Cx, ring: &mut IoUring) -> io::Result<()> {
    let mut registration = cx.register_io(ring, Interest::READABLE)?;
    poll_fn(|task| {
        if let Err(error) =
            checkpoint(cx).and_then(|()| registration.rearm(Interest::READABLE, task.waker()))
        {
            return Poll::Ready(Err(error));
        }
        completion(ring).map_or(Poll::Pending, Poll::Ready)
    })
    .await
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
        Ok(Self::Ring(ring))
    }
    pub(crate) fn start(file: &File) -> io::Result<Self> {
        match Self::ring(file) {
            Ok(ring) => Ok(Self::Ring(ring)),
            Err(error) if unavailable(&error) => Self::blocking(file),
            Err(error) => Err(error),
        }
    }
    fn blocking(file: &File) -> io::Result<Self> {
        let file = file.try_clone()?;
        Ok(Self::Blocking(Some(Box::pin(async move {
            // Uses the current runtime's bounded blocking pool. Std-only watch
            // and statistics owners use Asupersync's bounded thread fallback.
            // Only fsync escapes cancellation, never a write, rename or publish.
            asupersync::runtime::spawn_blocking_io(move || file.sync_all()).await
        }))))
    }
    fn ring(file: &File) -> io::Result<IoUring> {
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
        Ok(ring)
    }
    pub(crate) fn completion(&mut self) -> Option<io::Result<()>> {
        match self {
            Self::Ring(ring) => completion(ring),
            Self::Blocking(future) => {
                let task = &mut std::task::Context::from_waker(std::task::Waker::noop());
                match future.as_mut()?.as_mut().poll(task) {
                    Poll::Pending => None,
                    Poll::Ready(result) => {
                        *future = None;
                        Some(result)
                    }
                }
            }
        }
    }
}

/// Wait without blocking the async executor. Cancellation drops either the ring
/// registration or blocking completion; the caller must not publish afterward.
pub(crate) async fn sync_all(cx: &Cx, file: &File) -> io::Result<()> {
    checkpoint(cx)?;
    FileSync::start(file)?.wait(cx).await
}
/// One ring and one owned fixed-file slot for an ordered durability batch.
/// Consuming self across each wait prevents reusing a slot after cancellation.
pub(crate) struct SyncBatch(Option<IoUring>);
impl SyncBatch {
    pub(crate) fn new() -> io::Result<Self> {
        let reserve = || {
            let ring = IoUring::new(2)?;
            ring.submitter().register_files(&[-1])?;
            Ok(ring)
        };
        match reserve() {
            Ok(ring) => Ok(Self(Some(ring))),
            Err(error) if unavailable(&error) => Ok(Self(None)),
            Err(error) => Err(error),
        }
    }
    pub(crate) async fn file(mut self, cx: &Cx, file: &File) -> io::Result<Self> {
        checkpoint(cx)?;
        if let Some(ring) = &mut self.0 {
            match Self::submit(ring, file) {
                Ok(()) => {
                    wait_ring(cx, ring).await?;
                    checkpoint(cx)?;
                    return Ok(self);
                }
                Err(error) if unavailable(&error) => self.0 = None,
                Err(error) => return Err(error),
            }
        }
        FileSync::blocking(file)?.wait(cx).await?;
        Ok(self)
    }
    fn submit(ring: &mut IoUring, file: &File) -> io::Result<()> {
        // The preceding CQE has already retired before this slot is replaced.
        ring.submitter()
            .register_files_update(0, &[file.as_raw_fd()])?;
        let entry = opcode::Fsync::new(types::Fixed(0))
            .build()
            .user_data(1)
            .flags(squeue::Flags::ASYNC);
        // SAFETY: the table owns the file reference; fsync borrows no buffer.
        // A cancelled future owns and drops the entire batch, never its next slot.
        unsafe {
            ring.submission()
                .push(&entry)
                .map_err(|_| io::Error::other("fsync submission queue full"))?;
        }
        ring.submit()?;
        Ok(())
    }
}
impl FileSync {
    pub(crate) async fn wait(mut self, cx: &Cx) -> io::Result<()> {
        self.wait_ready(cx).await
    }
    async fn wait_ready(&mut self, cx: &Cx) -> io::Result<()> {
        match self {
            Self::Ring(ring) => wait_ring(cx, ring).await?,
            Self::Blocking(future) => {
                poll_fn(|task| {
                    if let Err(error) = checkpoint(cx) {
                        return Poll::Ready(Err(error));
                    }
                    future
                        .as_mut()
                        .expect("sync already completed")
                        .as_mut()
                        .poll(task)
                })
                .await?;
                *future = None;
            }
        }
        checkpoint(cx)
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
        let FileSync::Ring(ring) = &sync else {
            std::thread::sleep(wait);
            continue;
        };
        let mut poll = libc::pollfd {
            fd: ring.as_raw_fd(),
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
pub(crate) mod fault;
#[cfg(test)]
pub(crate) mod tests;
