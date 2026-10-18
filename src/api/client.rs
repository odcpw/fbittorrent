//! On-demand owner startup. P1-11 replaces only the legacy probe with API hello.
use crate::{
    agent::{AppError, Code},
    config::Config,
};
use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::os::{
    fd::FromRawFd,
    unix::{ffi::OsStrExt, net::UnixStream},
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn unit_name() -> Result<String> {
    let name = match std::env::var("FBITTORRENT_UNIT") {
        Ok(name) => name,
        Err(std::env::VarError::NotPresent) => "fbittorrent.service".into(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(AppError::new(
                Code::Usage,
                "FBITTORRENT_UNIT must be a UTF-8 .service unit name",
            )
            .into());
        }
    };
    if !name.ends_with(".service")
        || name.starts_with('-')
        || name.len() > 255
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.@:".contains(&c))
    {
        return Err(AppError::new(
            Code::Usage,
            "FBITTORRENT_UNIT must be a single .service unit name",
        )
        .into());
    }
    Ok(name)
}

pub fn unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("no config directory")?
        .join("systemd/user")
        .join(unit_name()?))
}

pub fn manager_command() -> Command {
    let mut command = Command::new("systemctl");
    command.arg("--user").arg("--no-ask-password");
    // systemctl uses the manager's private local transport on this systemd
    // version, even with DBUS_SESSION_BUS_ADDRESS set. Scope this override to
    // the control command: the daemon keeps its configured sandbox runtime.
    command.env(
        "XDG_RUNTIME_DIR",
        format!("/run/user/{}", unsafe { libc::geteuid() }),
    );
    // A test/client can have a private XDG_RUNTIME_DIR while the user manager
    // still lives on its real bus. Do not change the sandboxed daemon's XDG dirs.
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        command.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{}/bus", unsafe { libc::geteuid() }),
        );
    }
    command
}

pub fn manager_status(args: &[&str], bound: Duration) -> Result<bool> {
    let mut child = manager_command()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running systemctl --user")?;
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if began.elapsed() >= bound {
            let _ = child.kill();
            let _ = child.wait();
            bail!("systemctl --user {} timed out", args.join(" "));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Small property replies only; keep manager control bounded even on a lost bus.
pub fn manager_properties(unit: &str) -> Result<Option<std::collections::HashMap<String, String>>> {
    let mut child = manager_command()
        .args(["show", unit, "--property=LoadState,ActiveState,SubState,MainPID,Job"])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait()? { break status; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("systemctl show {unit} timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() { return Ok(None); }
    let mut output = String::new();
    child.stdout.take().unwrap().take(4096).read_to_string(&mut output)?;
    Ok(Some(output.lines().filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned())).collect()))
}

/// Cancel scheduled restarts as well as a currently running manager-owned daemon.
/// No installed unit means a foreground owner; its socket shutdown remains valid.
pub fn stop_managed_daemon() -> Result<bool> {
    if !unit_path()?.is_file() { return Ok(false); }
    let unit = unit_name()?;
    let Some(before) = manager_properties(&unit)? else { return Ok(false); };
    if before.get("LoadState").map(String::as_str) != Some("loaded") { return Ok(false); }
    let was_running = !manager_stopped(&before);
    if !manager_status(&["stop", &unit], Duration::from_secs(25))? {
        return Err(backend_error(format!("systemctl --user stop {unit} failed"), "fbittorrent status --json"));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let after = manager_properties(&unit)?;
        if after.as_ref().is_some_and(manager_stopped) { return Ok(was_running); }
        if Instant::now() >= deadline {
            return Err(AppError::new(Code::Timeout, format!("{unit} has not stopped; observed {after:?}"))
                .hint("fbittorrent status --json").into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn manager_stopped(state: &std::collections::HashMap<String, String>) -> bool {
    matches!(state.get("ActiveState").map(String::as_str), Some("inactive" | "failed"))
        && state.get("MainPID").map(String::as_str) == Some("0")
        && state.get("Job").is_some_and(|job| job.is_empty() || job == "0")
        && state.get("SubState").map(String::as_str) != Some("auto-restart")
}

fn deactivating(unit: &str) -> Result<bool> {
    let mut child = manager_command()
        .args(["is-active", unit])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let began = Instant::now();
    while child.try_wait()?.is_none() {
        if began.elapsed() >= Duration::from_secs(1) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("systemctl is-active timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut state = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(4096)
        .read_to_string(&mut state)?;
    Ok(state.trim() == "deactivating")
}

fn backend_error(message: impl Into<String>, hint: &str) -> anyhow::Error {
    AppError::new(Code::Backend, message).hint(hint).into()
}

// AF_UNIX has no std connect_timeout. Use a nonblocking connect + poll, then
// one deadline for the complete framed probe (including a dribbling peer).
fn connect_probe(path: &std::path::Path, deadline: Instant) -> Result<UnixStream> {
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        bail!("engine socket path is too long");
    }
    for (out, byte) in address.sun_path.iter_mut().zip(bytes) {
        *out = *byte as _;
    }
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let result = unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EAGAIN)) {
            return Err(error.into());
        }
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("engine connect probe timed out");
            }
            let mut event = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut event, 1, remaining.as_millis().max(1) as _) };
            if ready < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            if ready <= 0 {
                bail!("engine connect probe timed out");
            }
            let mut error = 0i32;
            let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&mut error as *mut i32).cast(),
                    &mut size,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if error != 0 {
                return Err(std::io::Error::from_raw_os_error(error).into());
            }
            break;
        }
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

pub fn probe(cfg: &Config) -> Result<()> {
    probe_path(&crate::ipc::socket_path(cfg))
}

fn probe_path(path: &std::path::Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut stream = connect_probe(path, deadline)?;
    stream.set_write_timeout(Some(
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1)),
    ))?;
    let bytes = serde_json::to_vec(&crate::ipc::Request {
        method: "GET".into(),
        path: "app/version".into(),
        args: vec![],
        file: None,
    })?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    fn read(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
        while !bytes.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("engine probe timed out");
            }
            stream.set_read_timeout(Some(remaining))?;
            let count = stream.read(bytes)?;
            if count == 0 {
                bail!("engine probe closed before its reply");
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    let mut header = [0; 4];
    read(&mut stream, &mut header, deadline)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > 32 * 1024 * 1024 {
        bail!("engine probe response exceeds size limit");
    }
    let mut bytes = vec![0; size];
    read(&mut stream, &mut bytes, deadline)?;
    let reply: serde_json::Value = serde_json::from_slice(&bytes)?;
    if reply["error"].is_string() || reply["app_error"].is_object() {
        bail!(
            "engine probe: {}",
            reply["error"].as_str().unwrap_or("engine unavailable")
        );
    }
    if !reply["value"].is_string() {
        bail!("invalid engine probe response");
    }
    Ok(())
}

/// Returns whether startup was needed. Reads pass no_start=true; explicit up
/// overrides Config.autostart at its caller, never the invocation-level policy.
pub fn ensure_daemon(cfg: &Config, no_start: bool) -> Result<bool> {
    if probe(cfg).is_ok() {
        return Ok(false);
    }
    if no_start || std::env::var("FBITTORRENT_NO_START").as_deref() == Ok("1") {
        return Err(backend_error(
            "fbittorrent is closed (fbittorrent is not running; automatic startup is disabled)",
            "fbittorrent up",
        ));
    }
    if !unit_path()?.is_file()
        || !manager_status(&["show-environment"], Duration::from_secs(2)).unwrap_or(false)
    {
        return Err(backend_error("run the daemon in a terminal or a tmux window; this session has no systemd user manager or installed fbittorrent unit", "fbittorrent serve"));
    }
    let unit = unit_name()?;
    let journal = format!(
        "journalctl --user -u {} -n 50",
        unit.trim_end_matches(".service")
    );
    for round in 0..2 {
        if !manager_status(&["start", &unit], Duration::from_secs(30)).unwrap_or(false) {
            return Err(backend_error(
                "could not start the fbittorrent daemon",
                &journal,
            ));
        }
        match probe(cfg) {
            Ok(()) => return Ok(true),
            Err(error) if round == 0 => {
                eprintln!(
                    "INFO daemon startup probe retry error={:?}",
                    error.to_string()
                );
                let deadline = Instant::now() + Duration::from_secs(10);
                while deactivating(&unit).unwrap_or(false) {
                    if Instant::now() >= deadline {
                        return Err(backend_error(
                            "fbittorrent is still deactivating after 10 seconds",
                            &journal,
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            Err(error) => {
                return Err(backend_error(
                    format!("fbittorrent did not answer after two start rounds: {error}"),
                    &journal,
                ))
            }
        }
    }
    unreachable!("the two start rounds return a result")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_connect_refuses_missing_or_overlong_socket() {
        let deadline = Instant::now() + Duration::from_millis(200);
        assert!(connect_probe(
            std::path::Path::new("/tmp/fbt-nonexistent-probe/socket"),
            deadline
        )
        .is_err());
        assert!(connect_probe(std::path::Path::new(&"x".repeat(200)), deadline).is_err());
    }
    #[test]
    fn probe_deadline_bounds_a_silent_peer() {
        let path = std::env::temp_dir().join(format!("fbt-probe-{}", std::process::id()));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let peer = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let began = Instant::now();
        let error = probe_path(&path).unwrap_err();
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "full production probe must not wait for the silent peer to close: {error}"
        );
        peer.join().unwrap();
        std::fs::remove_file(path).unwrap();
        println!("silent local peer bounded to 200ms probe deadline");
    }
}
