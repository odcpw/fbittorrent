//! Connect commands to the app-owned engine, through API v1.

use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::Config;

// Invocation policy is separate from Config: interactive settings can reload
// or save Config without losing --no-start or persisting it as autostart=false.
static NO_START: AtomicBool = AtomicBool::new(false);

pub fn start_prevented()->bool {NO_START.load(Ordering::Relaxed)}

pub fn prevent_start() {
    NO_START.store(true, Ordering::Relaxed);
}

pub fn stop(cfg: &Config) -> Result<()> {
        // A free instance lock during RestartSec does not mean the owner is
        // stopped: cancel the manager's pending restart before probing the lock.
        let managed = crate::api::client::stop_managed_daemon()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut sent = managed;
        let mut request_cfg = cfg.clone();
        request_cfg.timeout_secs = 1;
        loop {
            // Shutdown is an owner control and works while the engine slot is
            // Starting. A version probe would wait for the session unnecessarily.
            if !sent && crate::config::runtime_dir().join("api.sock").exists() {
                sent = crate::api::client::Client::connect(&request_cfg, false)
                    .and_then(|client| client.call("app.shutdown", serde_json::json!({}))).is_ok();
            }
            if engine_idle()? {
                if !sent && NO_START.load(Ordering::Relaxed) {
                    return Err(crate::agent::AppError::new(crate::agent::Code::Backend, "fbittorrent is closed")
                        .hint("fbittorrent up").into());
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(crate::agent::AppError::new(crate::agent::Code::Timeout, "fbittorrent received the close request or is still starting; its owner has not stopped")
                    .hint("fbittorrent status --json").into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
}

/// Focus the app's existing terminal, or launch this binary in a new terminal.
pub fn open_app() -> Result<()> {
    let exe = std::env::current_exe().context("finding the fbittorrent executable")?;
    // Omarchy's launcher forms a shell command internally. Quote the path for
    // that boundary, even when the executable lives in a folder containing spaces.
    let quoted = format!("'{}'", exe.to_string_lossy().replace('\'', "'\\''"));
    let mut launcher = Command::new("omarchy-launch-or-focus-tui")
        .args(["--app-id=org.omarchy.fbittorrent", &quoted, "tui"])
        // The launched terminal outlives this command. Inheriting a CLI pipe
        // would keep its caller waiting for EOF until the app window closes.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("opening the fbittorrent app; omarchy-launch-or-focus-tui is required")?;
    for _ in 0..5 {
        if let Some(status) = launcher.try_wait()? {
            if !status.success() { bail!("the Omarchy launcher could not open or focus fbittorrent"); }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // A terminal launcher may remain alive for its window's lifetime.
    std::thread::spawn(move || { let _ = launcher.wait(); });
    Ok(())
}

fn engine_idle() -> Result<bool> {
    match crate::ipc::InstanceLock::acquire() {
        Ok(_lock) => Ok(true),
        Err(error) if error.to_string().contains("already open") => Ok(false),
        Err(error) => Err(error),
    }
}

/// Open the shell panel when installed, otherwise open an attached TUI terminal.
pub fn dashboard(view: Option<&str>, query: Option<&str>) -> Result<()> {
    dashboard_hash(view,query,None)
}

pub fn dashboard_hash(view: Option<&str>,query: Option<&str>,hash: Option<&str>) -> Result<()> {
    let mut payload=serde_json::json!({"view":view.unwrap_or("torrents"),"query":query.unwrap_or("")});
    if let Some(hash)=hash {payload["hash"]=serde_json::json!(hash);}
    if summon_dashboard(&payload)? {return Ok(())}
    open_app()
}

/// Structured argv only; callers can route a confirmation without adding first.
pub fn summon_dashboard(payload: &serde_json::Value) -> Result<bool> {
    let manifest=dirs::config_dir().context("no config directory")?.join("omarchy/plugins/oliver.torrents/manifest.json");
    let panel=std::fs::read(&manifest).ok().and_then(|bytes|serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value|value["kinds"].as_array().is_some_and(|kinds|kinds.iter().any(|kind|kind=="panel")));
    if !panel {return Ok(false)}
    let payload=serde_json::to_string(payload)?;
    if payload.len()>65536 {return Err(crate::agent::AppError::new(crate::agent::Code::Usage,"dashboard request exceeds 65536 bytes").into())}
    match Command::new("omarchy-shell").args(["shell","summon","oliver.torrents",&payload]).stdout(Stdio::null()).status() {
        Ok(status) if status.success()=>Ok(true),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(false),
        result=>Err(crate::agent::AppError::new(crate::agent::Code::Upstream,format!("could not summon torrent dashboard: {result:?}")).into()),
    }
}
