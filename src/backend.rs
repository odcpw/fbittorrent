//! On-demand qbittorrent-nox: started when a command needs it, stopped on request.

use anyhow::{bail, Context, Result};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::qbt::Client;

pub fn is_active(unit: &str) -> bool {
    Command::new("systemctl").args(["--user", "is-active", "--quiet", unit]).status()
        .map(|s| s.success()).unwrap_or(false)
}

/// Start the unit if it is not running and wait until the WebUI answers.
pub fn ensure_running(cfg: &Config, client: &Client) -> Result<bool> {
    if client.version().is_ok() { return Ok(false); }
    if !cfg.autostart { bail!("qBittorrent is not running (autostart is off). Run: fbittorrent up"); }
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
    let st = Command::new("systemctl").args(["--user", "stop", &cfg.backend_unit]).status()
        .context("running systemctl")?;
    if !st.success() { bail!("systemctl --user stop {} failed", cfg.backend_unit); }
    Ok(())
}
