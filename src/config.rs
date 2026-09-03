//! User configuration: ~/.config/fbittorrent/config.toml

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Base URL of the qBittorrent WebUI.
    pub url: String,
    /// Optional WebUI credentials. Not needed when LocalHostAuth is disabled.
    pub username: Option<String>,
    pub password: Option<String>,
    /// Default add-time flags.
    pub sequential: bool,
    pub first_last: bool,
    /// Allow uploading/seeding for new torrents. Off = 1 B/s upload cap and stop when complete.
    pub seed_by_default: bool,
    /// Optional save path override for new torrents.
    pub save_path: Option<String>,
    /// Optional category for new torrents.
    pub category: Option<String>,
    /// Send a desktop notification after an add.
    pub notify: bool,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Start the backend user service on demand when a command needs it.
    pub autostart: bool,
    /// systemd user unit that runs qbittorrent-nox.
    pub backend_unit: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8080".into(),
            username: None,
            password: None,
            sequential: true,
            first_last: true,
            seed_by_default: false,
            save_path: None,
            category: None,
            notify: true,
            timeout_secs: 10,
            autostart: true,
            backend_unit: "qbittorrent-nox.service".into(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("fbittorrent")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write_default_if_missing() -> Result<PathBuf> {
        let path = config_path();
        if !path.exists() {
            std::fs::create_dir_all(config_dir())?;
            let body = toml::to_string_pretty(&Config::default())?;
            std::fs::write(&path, body)?;
        }
        Ok(path)
    }
}
