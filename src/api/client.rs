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

fn manager_query_error(unit: &str, detail: impl std::fmt::Display) -> anyhow::Error {
    AppError::new(Code::Backend, format!("cannot verify manager state for {unit}: {detail}; shutdown is unconfirmed"))
        .hint(format!("systemctl --user stop {unit}")).into()
}

fn parse_manager_properties(unit: &str, output: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut state = std::collections::HashMap::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(manager_query_error(unit, "malformed property line"));
        };
        if !["LoadState", "ActiveState", "SubState", "MainPID", "Job"].contains(&key)
            || state.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(manager_query_error(unit, "unexpected or duplicate manager property"));
        }
    }
    let value = |key: &str| state.get(key).map(String::as_str).unwrap_or("");
    let decimal = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) && text.parse::<u32>().is_ok();
    if state.len() != 5
        || !matches!(value("LoadState"), "loaded" | "not-found")
        || !matches!(value("ActiveState"), "active" | "reloading" | "inactive" | "failed" | "activating" | "deactivating" | "maintenance" | "refreshing")
        || value("SubState").is_empty()
        || !value("SubState").bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        || !decimal(value("MainPID"))
        || !(value("Job").is_empty() || decimal(value("Job")))
        || (value("LoadState") == "not-found" &&
            !(value("ActiveState") == "inactive" && value("SubState") == "dead" && value("MainPID") == "0" && value("Job").is_empty())) {
        return Err(manager_query_error(unit, "missing, invalid or inconsistent manager properties"));
    }
    Ok(state)
}

/// A failed query is uncertainty, never evidence that a managed owner is absent.
/// Replies must include every requested property before shutdown can use them.
pub fn manager_properties(unit: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut child = manager_command()
        .args(["show", unit, "--property=LoadState,ActiveState,SubState,MainPID,Job"])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|error| manager_query_error(unit, error))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {},
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(manager_query_error(unit, error));
            },
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new(Code::Timeout, format!("systemctl show {unit} timed out; shutdown is unconfirmed"))
                .hint(format!("systemctl --user stop {unit}")).into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() { return Err(manager_query_error(unit, format!("systemctl show exited {status}"))); }
    let mut output = String::new();
    child.stdout.take().unwrap().take(4097).read_to_string(&mut output)
        .map_err(|error| manager_query_error(unit, error))?;
    if output.len() > 4096 { return Err(manager_query_error(unit, "manager reply exceeds 4096 bytes")); }
    parse_manager_properties(unit, &output)
}

/// No local manager endpoints and no remote bus configuration establish the
/// non-systemd case. Transport/query failures with a possible manager do not.
fn local_manager_absent(unit: &str) -> Result<bool> {
    let runtime = PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }));
    let default_bus = format!("unix:path={}/bus", runtime.display());
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|address| !address.is_empty() && address != default_bus.as_str()) {
        return Ok(false);
    }
    for endpoint in [runtime.join("systemd"), runtime.join("bus")] {
        match std::fs::symlink_metadata(endpoint) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(manager_query_error(unit, error)),
        }
    }
    Ok(true)
}

/// Cancel scheduled restarts as well as a running manager-owned daemon. Even a
/// removed unit file can leave a loaded unit with a pending restart in systemd.
pub fn stop_managed_daemon() -> Result<bool> {
    let unit = unit_name()?;
    if local_manager_absent(&unit)? { return Ok(false); }
    let before = manager_properties(&unit)?;
    if before["LoadState"] == "not-found" { return Ok(false); }
    let was_running = !manager_stopped(&before);
    if !manager_status(&["stop", &unit], Duration::from_secs(25))? {
        return Err(backend_error(format!("systemctl --user stop {unit} failed"), "fbittorrent status --json"));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let after = manager_properties(&unit)?;
        if manager_stopped(&after) { return Ok(was_running); }
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
    fn manager_replies_require_complete_consistent_properties() {
        let absent = "LoadState=not-found\nActiveState=inactive\nSubState=dead\nMainPID=0\nJob=\n";
        assert!(manager_stopped(&parse_manager_properties("fixture.service", absent).unwrap()));
        let restarting = "LoadState=loaded\nActiveState=activating\nSubState=auto-restart\nMainPID=0\nJob=\n";
        assert!(!manager_stopped(&parse_manager_properties("fixture.service", restarting).unwrap()));
        for malformed in [String::new(), absent.replace("SubState=dead\n", ""),
            absent.replace("MainPID=0", "MainPID=unknown"), absent.replace("Job=", "Job=bad"),
            absent.replace("LoadState=not-found", "LoadState=error"),
            absent.replace("MainPID=0", "MainPID=42"), format!("{absent}MainPID=0\n")] {
            let error = parse_manager_properties("fixture.service", &malformed).unwrap_err();
            let (code, message, hint) = crate::agent::classify(&error);
            assert_eq!(code, Code::Backend);
            assert!(message.contains("shutdown is unconfirmed"));
            assert_eq!(hint.as_deref(), Some("systemctl --user stop fixture.service"));
        }
        println!("absent/restarting properties distinguished; malformed replies fail with backend/4 and selected-unit stop hint");
    }

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
