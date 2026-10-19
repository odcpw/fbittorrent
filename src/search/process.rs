//! Bounded subprocess capture and whole-process-group lifetime.
use super::*;

pub(super) fn python(directory: &Path) -> Command {
    let mut command = Command::new("python3");
    command
        .current_dir(directory)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null());
    let parent = unsafe { libc::getpid() };
    // Every child gets its own process group. kill(-pid) reaches Nova's pool,
    // its resource tracker, and any provider subprocesses together.
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Nova handles TERM by killing its group, including subprocesses
            // created by providers. Killing only Nova would orphan those children.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::raise(libc::SIGKILL);
            }
            Ok(())
        });
    }
    command
}

pub(super) struct ProcessGroup {
    pub(super) child: Child,
    pub(super) pid: i32,
}
impl ProcessGroup {
    pub(super) fn spawn(mut command: Command) -> Result<Self> {
        let mut registry = jobs()
            .lock()
            .map_err(|_| anyhow!("search lifecycle lock failed"))?;
        if registry.closing {
            bail!("the app is closing; search providers cannot start");
        }
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("search requires a working Python 3 installation (python3)")?;
        let pid = child.id() as i32;
        // Track metadata/compile runners too, before shutdown can acquire the
        // mutex. Search-job registration may happen after the worker is created.
        registry.processes.insert(pid);
        Ok(Self { child, pid })
    }
    pub(super) fn kill(&self) {
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
        let _ = self.child.wait();
        if let Ok(mut registry) = jobs().lock() {
            registry.processes.remove(&self.pid);
        }
    }
}

pub(super) fn capture(command: Command, budget: Duration) -> Result<Vec<u8>> {
    let mut process = ProcessGroup::spawn(command)?;
    let mut stdout = process.child.stdout.take().unwrap();
    let mut stderr = process.child.stderr.take().unwrap();
    let output = thread::spawn(move || drain_bounded(&mut stdout, MAX_CAPTURE_BYTES));
    let errors = thread::spawn(move || drain_bounded(&mut stderr, 16 * 1024));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = process.child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() > budget {
            process.kill();
            let _ = process.child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    process.kill(); // clear any children that outlived the runner
    let (output, exceeded) = output
        .join()
        .map_err(|_| anyhow!("search output reader failed"))??;
    let (errors, _) = errors
        .join()
        .map_err(|_| anyhow!("search error reader failed"))??;
    if exceeded {
        bail!("search provider output exceeded 2 MiB");
    }
    match status {
        Some(status) if status.success() => Ok(output),
        Some(_) => bail!(
            "search provider runtime failed: {}",
            String::from_utf8_lossy(&errors).trim()
        ),
        None => bail!("search provider runtime timed out"),
    }
}

pub(super) fn drain_bounded(reader: &mut impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0; 8192];
    loop {
        let size = match reader.read(&mut buffer) {
            Ok(0) => return Ok((bytes, exceeded)),
            Ok(size) => size,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let retained = size.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        exceeded |= retained < size;
    }
}

