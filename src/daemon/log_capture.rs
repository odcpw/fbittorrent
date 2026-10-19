//! Drain redirected stderr without changing its journald/file copy. Only the
//! stored interpretation is capped; original diagnostic bytes are copied intact.
use super::log::Ring;
use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};
pub(crate) struct Capture {
    pub ring: Arc<Ring>,
    original: File,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Capture {
    pub fn install() -> Result<Self> {
        let fd = unsafe { libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 3) };
        anyhow::ensure!(
            fd >= 0,
            "copying stderr: {}",
            std::io::Error::last_os_error()
        );
        let original = unsafe { File::from_raw_fd(fd) };
        let mut copy = original.try_clone()?;
        let mut pipe = [-1; 2];
        anyhow::ensure!(
            unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } == 0,
            "creating log pipe: {}",
            std::io::Error::last_os_error()
        );
        let mut reader = unsafe { File::from_raw_fd(pipe[0]) };
        let writer = unsafe { File::from_raw_fd(pipe[1]) };
        anyhow::ensure!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } >= 0,
            "configuring log reader"
        );
        anyhow::ensure!(
            unsafe { libc::dup2(writer.as_raw_fd(), 2) } >= 0,
            "redirecting stderr"
        );
        drop(writer);
        let mut capture = Self {
            ring: Arc::new(Ring::default()),
            original,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        };
        let ring = capture.ring.clone();
        let stop = capture.stop.clone();
        capture.worker = Some(
            thread::Builder::new()
                .name("fbt-log".into())
                .spawn(move || {
                    let mut bytes = [0; 8192];
                    let mut line = Vec::with_capacity(1024);
                    loop {
                        let mut poll = libc::pollfd {
                            fd: reader.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        unsafe {
                            libc::poll(&mut poll, 1, 100);
                        }
                        let mut blocks = 0;
                        loop {
                            match reader.read(&mut bytes) {
                                Ok(0) => return,
                                Ok(count) => {
                                    let _ = copy.write_all(&bytes[..count]);
                                    for byte in &bytes[..count] {
                                        if *byte == b'\n' {
                                            ring.line(&String::from_utf8_lossy(&line));
                                            line.clear();
                                        } else if line.len() < 8192 {
                                            line.push(*byte);
                                        }
                                    }
                                    blocks += 1;
                                    if stop.load(Ordering::Acquire) && blocks >= 256 {
                                        return;
                                    }
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                    continue
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    break
                                }
                                Err(_) => return,
                            }
                        }
                        if stop.load(Ordering::Acquire) {
                            if !line.is_empty() {
                                ring.line(&String::from_utf8_lossy(&line));
                            }
                            return;
                        }
                    }
                })
                .context("starting log reader")?,
        );
        Ok(capture)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.original.as_raw_fd(), 2);
        }
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
