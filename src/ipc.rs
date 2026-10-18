//! Private, app-lifetime transport for the TUI, CLI and desktop widget.
//!
//! This is a bounded synchronous boundary because the CLI/widget request API
//! and feature handlers are synchronous. Its listener and at most 16 request
//! threads are owned and joined by Server; torrent work runs on the app-owned
//! Asupersync runtime. Keeping these handlers outside the runtime's blocking
//! pool prevents a handler waiting for torrent disk work from occupying that
//! same pool and deadlocking its own request.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::config::Config;

const MAX_FRAME: usize = 32 * 1024 * 1024;
const MAX_CLIENTS: usize = 16;

#[derive(Debug, Serialize, Deserialize)]
pub struct Upload {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub args: Vec<(String, String)>,
    pub file: Option<Upload>,
}

impl Request {
    pub fn arg(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Serialize, Deserialize)]
struct Response {
    value: Option<String>,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_error: Option<crate::agent::AppError>,
}

pub fn socket_path(_cfg: &Config) -> PathBuf {
    crate::config::runtime_dir().join("engine.sock")
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME {
        bail!(
            "native request is too large (maximum {} MiB)",
            MAX_FRAME / 1024 / 1024
        );
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}

fn read_frame<T: for<'a> Deserialize<'a>>(stream: &mut UnixStream) -> Result<T> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        bail!("native response exceeds size limit");
    }
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn request(
    cfg: &Config,
    method: &str,
    path: &str,
    args: &[(&str, &str)],
    file: Option<(&str, &[u8])>,
) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path(cfg))
        .context("fbittorrent is closed; open the app to run the native torrent engine")?;
    let timeout = Duration::from_secs(if path == "torrents/add" {
        cfg.timeout_secs.max(95)
    } else {
        cfg.timeout_secs.max(1)
    });
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let req = Request {
        method: method.into(),
        path: path.into(),
        args: args
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
        file: file.map(|(name, bytes)| Upload {
            name: name.into(),
            bytes: bytes.to_vec(),
        }),
    };
    write_frame(&mut stream, &req)?;
    let response: Response = read_frame(&mut stream).context("reading native engine response")?;
    if let Some(error) = response.app_error { return Err(error.into()); }
    match (response.value, response.error) {
        (Some(value), None) => Ok(value),
        (_, Some(error)) => bail!("{error}"),
        _ => bail!("invalid native engine response"),
    }
}

pub fn is_running(cfg: &Config) -> bool {
    let mut probe = cfg.clone();
    probe.timeout_secs = 1;
    request(&probe, "GET", "app/version", &[], None).is_ok()
}

/// Held for the whole engine lifetime, before any session or file I/O starts.
pub(crate) struct InstanceLock {
    directory: PathBuf,
    _file: File,
}

impl InstanceLock {
    pub(crate) fn acquire() -> Result<Self> {
        Self::acquire_in(crate::config::runtime_dir())
    }

    pub(crate) fn acquire_in(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        Self::acquire_wait(directory, Duration::ZERO)
    }

    /// The daemon validates the directory before this call. Do not repair or
    /// follow an ambiguous path here; keep the same open file across retries.
    pub(crate) fn acquire_wait(directory: PathBuf, timeout: Duration) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("engine.lock"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            bail!("engine.lock must be a regular private file owned by this uid");
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        // The stable lock file must never be unlinked while another app can open it.
        let began = std::time::Instant::now();
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                if began.elapsed() < timeout {
                    std::thread::sleep(Duration::from_millis(20).min(timeout.saturating_sub(began.elapsed())));
                    continue;
                }
                return Err(crate::agent::AppError::new(crate::agent::Code::Failure,
                    "already_running: fbittorrent is already open; use the existing app")
                    .hint("fbittorrent status --json").into());
            }
            return Err(error).context("locking the native torrent engine");
        }
        Ok(Self {
            directory,
            _file: file,
        })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // flock belongs to the open file description, which fork inherits even
        // with CLOEXEC. A parallel provider spawn may not have reached exec yet
        // when the engine closes and reopens this directory. Release ownership
        // explicitly instead of waiting for every inherited descriptor to close.
        // The stable lock file remains in place for the next owner.
        unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub struct Server {
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    clients: Arc<Mutex<HashMap<u64, (UnixStream, Arc<AtomicBool>)>>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Server {
    pub fn start(
        _cfg: &Config,
        lock: &InstanceLock,
        handler: impl Fn(&Request) -> Result<String> + Send + Sync + 'static,
    ) -> Result<Self> {
        let path = lock.directory.join("engine.sock");
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(&path).context("opening native engine socket")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let handler = Arc::new(handler);
        let active = Arc::new(AtomicUsize::new(0));
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let client_sequence = AtomicU64::new(0);
        let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
        let server_clients = clients.clone();
        let server_workers = workers.clone();
        let thread = std::thread::Builder::new()
            .name("fbittorrent-ipc".into())
            .spawn(move || {
                for incoming in listener.incoming() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut stream) = incoming else { continue };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    if active.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                        active.fetch_sub(1, Ordering::AcqRel);
                        let _ = write_frame(
                            &mut stream,
                            &Response {
                                value: None,
                                error: Some("native engine is busy; retry shortly".into()),
                                app_error: None,
                            },
                        );
                        continue;
                    }
                    let client_id = client_sequence.fetch_add(1, Ordering::Relaxed);
                    let read = Arc::new(AtomicBool::new(false));
                    if let Ok(clone) = stream.try_clone() {
                        server_clients.lock().unwrap().insert(client_id, (clone, read.clone()));
                    }
                    let handler = handler.clone();
                    let active = active.clone();
                    let clients = server_clients.clone();
                    let worker = std::thread::spawn(move || {
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                        let result = read_frame::<Request>(&mut stream)
                            .and_then(|request| {
                                read.store(true, Ordering::Release);
                                handler(&request)
                            });
                        let response = match result {
                            Ok(value) => Response {
                                value: Some(value),
                                error: None,
                                app_error: None,
                            },
                            Err(error) => Response {
                                value: None,
                                error: Some(format!("{error:#}")),
                                app_error: error.downcast_ref::<crate::agent::AppError>().cloned(),
                            },
                        };
                        let _ = write_frame(&mut stream, &response);
                        clients.lock().unwrap().remove(&client_id);
                        active.fetch_sub(1, Ordering::AcqRel);
                    });
                    let mut workers = server_workers.lock().unwrap();
                    // Reap finished requests, so widget polling does not accumulate threads.
                    let mut index = 0;
                    while index < workers.len() {
                        if workers[index].is_finished() {
                            let _ = workers.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                    workers.push(worker);
                }
            })?;
        Ok(Self {
            path,
            stopping,
            thread: Some(thread),
            clients,
            workers,
        })
    }

    pub fn stop_accepting(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Wake incomplete readers, retaining their write half for the error
        // reply. Already-read requests must still receive their handler result.
        for (stream, read) in self.clients.lock().unwrap().values() {
            if !read.load(Ordering::Acquire) {
                let _ = stream.shutdown(std::net::Shutdown::Read);
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop_accepting();
        for worker in self.workers.lock().unwrap().drain(..) {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn quiesce_preserves_an_already_read_reply() {
        let directory = std::env::temp_dir().join(format!("fbt-ipc-quiesce-{}", std::process::id()));
        let lock = InstanceLock::acquire_in(directory.clone()).unwrap();
        let (entered, has_entered) = std::sync::mpsc::channel();
        let (resume, resume_rx) = std::sync::mpsc::channel();
        let resume_rx = Mutex::new(resume_rx);
        let mut server = Server::start(&Config::default(), &lock, move |_| {
            entered.send(()).unwrap();
            resume_rx.lock().unwrap().recv_timeout(Duration::from_secs(2))?;
            Ok("read-before-quiesce".into())
        }).unwrap();
        let path = directory.join("engine.sock");
        let client = std::thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            write_frame(&mut stream, &Request { method: "GET".into(), path: "app/version".into(), args: vec![], file: None }).unwrap();
            read_frame::<Response>(&mut stream).unwrap()
        });
        has_entered.recv_timeout(Duration::from_secs(2)).unwrap();
        server.stop_accepting();
        resume.send(()).unwrap();
        let response = client.join().unwrap();
        println!("quiesced already-read request reply: {:?}", response.value);
        assert_eq!(response.value.as_deref(), Some("read-before-quiesce"));
        drop(server);
        drop(lock);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn instance_lock_waits_for_a_short_offline_owner() {
        let directory = std::env::temp_dir().join(format!("fbt-lock-wait-{}", std::process::id()));
        let first = InstanceLock::acquire_in(directory.clone()).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(first);
        });
        let began = Instant::now();
        let next = InstanceLock::acquire_wait(directory.clone(), Duration::from_secs(1)).unwrap();
        println!("offline lock acquired after {} ms", began.elapsed().as_millis());
        assert!(began.elapsed() >= Duration::from_millis(100));
        release.join().unwrap();
        drop(next);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn instance_lock_repairs_existing_permissions_without_replacing_the_file() {
        use std::os::unix::fs::MetadataExt;
        let directory = std::env::temp_dir().join(format!("fbittorrent-private-lock-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("engine.lock");
        std::fs::write(&path, b"stable lock fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        let lock = InstanceLock::acquire_in(directory.clone()).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.ino(), inode);
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"stable lock fixture");
        assert!(InstanceLock::acquire_in(directory.clone()).is_err());
        println!("{}: 600 after acquiring an existing 644 lock", path.display());
        drop(lock);
        drop(InstanceLock::acquire_in(directory.clone()).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn stopping_socket_interrupts_an_incomplete_frame_and_joins_its_worker() {
        let directory = std::env::temp_dir().join(format!(
            "fbittorrent-ipc-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let lock = InstanceLock::acquire_in(directory.clone()).unwrap();
        let mut server = Server::start(&Config::default(), &lock, |_| Ok("unused".into())).unwrap();
        let mut client = UnixStream::connect(directory.join("engine.sock")).unwrap();
        client.write_all(&[0, 0]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while server.clients.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            server.clients.lock().unwrap().len(),
            1,
            "prove the server has admitted the parked reader"
        );
        let began = Instant::now();
        server.stop_accepting();
        drop(server);
        assert!(began.elapsed() < Duration::from_secs(1));
        assert!(!directory.join("engine.sock").exists());
        drop(client);
        drop(lock);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
