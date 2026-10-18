//! Connect commands to the app-owned engine, with explicit legacy compatibility.

use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::qbt::Client;

// Invocation policy is separate from Config: interactive settings can reload
// or save Config without losing --no-start or persisting it as autostart=false.
static NO_START: AtomicBool = AtomicBool::new(false);

pub fn prevent_start() {
    NO_START.store(true, Ordering::Relaxed);
}

pub fn is_active(unit: &str) -> bool {
    Command::new("systemctl").args(["--user", "is-active", "--quiet", unit])
        .stdout(Stdio::null()).stderr(Stdio::null()).status()
        .map(|s| s.success()).unwrap_or(false)
}

/// Start the unit if it is not running and wait until the WebUI answers.
pub fn ensure_running(cfg: &Config, client: &Client) -> Result<bool> {
    if cfg.native() {
        return crate::api::client::ensure_daemon(cfg, !cfg.autostart || NO_START.load(Ordering::Relaxed));
    }
    if client.version().is_ok() { return Ok(false); }
    if !cfg.autostart || NO_START.load(Ordering::Relaxed) {
        return Err(crate::agent::AppError::new(crate::agent::Code::Backend, "qBittorrent is not running (automatic startup is disabled)")
            .hint("fbittorrent up").into());
    }
    let st = Command::new("systemctl").args(["--user", "start", &cfg.backend_unit]).status()
        .context("running systemctl")?;
    if !st.success() { bail!("systemctl --user start {} failed", cfg.backend_unit); }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
        if client.version().is_ok() { return Ok(true); }
    }
    bail!("started {} but the WebUI did not come up at {}", cfg.backend_unit, cfg.url)
}

pub fn stop(cfg: &Config) -> Result<()> {
    if cfg.native() {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut sent = false;
        let mut request_cfg = cfg.clone();
        request_cfg.timeout_secs = 1;
        loop {
            // Shutdown is an owner control and works while the engine slot is
            // Starting. A version probe would wait for the session unnecessarily.
            if !sent && crate::ipc::socket_path(cfg).exists() {
                sent = crate::ipc::request(&request_cfg, "POST", "app/shutdown", &[], None).is_ok();
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
    stop_legacy(cfg)
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

/// Retire the old service only after the native migration has been saved.
pub fn stop_legacy(cfg: &Config) -> Result<()> {
    let st = Command::new("systemctl").args(["--user", "stop", &cfg.backend_unit]).status()
        .context("running systemctl")?;
    if !st.success() { bail!("systemctl --user stop {} failed", cfg.backend_unit); }
    Ok(())
}

pub fn disable_legacy(cfg: &Config) -> Result<()> {
    let status = Command::new("systemctl").args(["--user", "disable", &cfg.backend_unit])
        .status().context("disabling the imported legacy service")?;
    if !status.success() { bail!("systemctl --user disable {} failed; imported torrents remain paused", cfg.backend_unit); }
    Ok(())
}

/// A stopped installed nox is opened once, inside migration, to export its state.
/// There must be both a known local record and a loaded qBittorrent service.
pub fn start_for_legacy_migration(cfg: &Config, client: &Client) -> Result<bool> {
    if !(cfg.url.starts_with("http://127.0.0.1:") || cfg.url.starts_with("http://localhost:")) {
        return Ok(false);
    }
    let data = dirs::data_local_dir().context("no qBittorrent data directory")?.join("qBittorrent/BT_backup");
    let mut backup_exists = false;
    if data.is_dir() {
        for entry in std::fs::read_dir(&data)? {
            let entry = entry.context("reading legacy torrent backup directory")?;
            if entry.path().extension().is_some_and(|extension| extension == "torrent" || extension == "fastresume") {
                backup_exists = true;
            }
        }
    }
    let feeds = dirs::config_dir().context("no qBittorrent config directory")?.join("qBittorrent/rss/feeds.json");
    let feeds_exist = if feeds.exists() {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&feeds)?)
            .context("reading old RSS feed records; legacy data is preserved")?;
        value.as_object().is_some_and(|feeds| !feeds.is_empty())
    } else { false };
    if !backup_exists && !feeds_exist { return Ok(false); }
    let property = |name: &str| -> Option<String> {
        let output = Command::new("systemctl").args(["--user", "show", "--property", name, "--value", &cfg.backend_unit])
            .stderr(Stdio::null()).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    if property("LoadState").as_deref() != Some("loaded")
        || !property("ExecStart").is_some_and(|command| command.contains("qbittorrent-nox")) {
        bail!("legacy torrent or RSS records exist, but no loaded qBittorrent-nox service can export them; start the legacy app once to migrate safely");
    }
    let status = Command::new("systemctl").args(["--user", "start", &cfg.backend_unit])
        .status().context("starting existing qBittorrent once to capture legacy state")?;
    if !status.success() { bail!("could not start {} for migration; legacy records preserved", cfg.backend_unit); }
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if client.login().is_ok() && client.version().is_ok() { return Ok(true); }
        std::thread::sleep(Duration::from_millis(150));
    }
    bail!("started the existing legacy service for migration, but its WebUI did not come up; source preserved")
}

/// Open the shell panel when installed, otherwise open an attached TUI terminal.
pub fn dashboard(view: Option<&str>, query: Option<&str>) -> Result<()> {
    let manifest = dirs::config_dir().context("no config directory")?.join("omarchy/plugins/oliver.torrents/manifest.json");
    let panel = std::fs::read(&manifest).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value["kinds"].as_array().is_some_and(|kinds| kinds.iter().any(|kind| kind == "panel")));
    if panel {
        let payload = serde_json::json!({"view": view.unwrap_or("torrents"), "query": query.unwrap_or("")}).to_string();
        match Command::new("omarchy-shell").args(["shell", "summon", "oliver.torrents", &payload]).status() {
            Ok(status) if status.success() => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            result => return Err(crate::agent::AppError::new(crate::agent::Code::Upstream, format!("could not summon torrent dashboard: {result:?}")).into()),
        }
    }
    open_app()
}
