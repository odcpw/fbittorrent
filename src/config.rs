//! User configuration: ~/.config/fbittorrent/config.toml

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Torrent engine: native (embedded Rust, app lifetime) or qbittorrent (legacy).
    pub engine: String,
    /// Incoming peer port for the embedded engine.
    pub peer_port: u16,
    /// Maximum simultaneously downloading torrents; additional downloads wait.
    pub max_active_downloads: usize,
    /// Peer encryption policy: off, prefer, or require.
    pub peer_encryption: String,
    /// Require this interface, or "proton" for automatic ProtonVPN discovery.
    pub network_interface: Option<String>,
    /// Base URL of the qBittorrent WebUI.
    pub url: String,
    /// Optional WebUI credentials. Not needed when LocalHostAuth is disabled.
    pub username: Option<String>,
    pub password: Option<String>,
    /// Default add-time flags.
    pub sequential: bool,
    pub first_last: bool,
    /// Allow uploading/seeding for new torrents. Off = upload cap and stop when complete.
    pub seed_by_default: bool,
    /// Optional save path override for new torrents.
    pub save_path: Option<String>,
    /// Optional category for new torrents.
    pub category: Option<String>,
    /// Send a desktop notification after an add.
    pub notify: bool,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Open the app on demand when a command needs its torrent engine.
    pub autostart: bool,
    /// systemd user unit that runs qbittorrent-nox.
    pub backend_unit: String,
    /// Preferred release quality for followed shows ("2160p", "1080p", "720p", "480p").
    pub show_quality: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: "native".into(),
            peer_port: 6882,
            max_active_downloads: 3,
            peer_encryption: "prefer".into(),
            network_interface: None,
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
            show_quality: "1080p".into(),
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
    pub fn native(&self) -> bool { self.engine == "native" }
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(matches!(self.engine.as_str(), "native" | "qbittorrent"), "engine must be native or qbittorrent");
        anyhow::ensure!(self.max_active_downloads > 0, "simultaneous downloads must be at least 1");
        anyhow::ensure!(matches!(self.peer_encryption.as_str(), "off" | "prefer" | "require"), "peer encryption must be off, prefer, or require");
        if let Some(name) = &self.network_interface {
            anyhow::ensure!(!name.is_empty() && name.len() < libc::IFNAMSIZ && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')), "choose proton or a valid network interface name");
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> { self.save_to(&config_path()) }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        self.validate()?;
        let directory = path.parent().context("settings path has no parent")?;
        std::fs::create_dir_all(directory)?;
        let staged = directory.join(format!(".config-{}-{}.tmp", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&staged)?;
            file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&staged, path)?;
            std::fs::File::open(directory)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() { let _ = std::fs::remove_file(&staged); }
        result.with_context(|| format!("saving {}", path.display()))
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

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir().map(|p| p.join("fbittorrent")).unwrap_or_else(config_dir)
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(|p| PathBuf::from(p).join("fbittorrent"))
        .unwrap_or_else(|| data_dir().join("runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_settings_gain_queue_and_encryption_defaults() {
        let cfg: Config = toml::from_str("engine = 'native'\nseed_by_default = false\n").unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.max_active_downloads, 3);
        assert_eq!(cfg.peer_encryption, "prefer");
        assert_eq!(cfg.network_interface, None);
    }
    #[test]
    fn invalid_settings_leave_saved_file_intact() {
        let dir = std::env::temp_dir().join(format!("fbittorrent-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut cfg = Config::default();
        cfg.network_interface = Some("proton".into());
        cfg.save_to(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        cfg.max_active_downloads = 0;
        assert!(cfg.save_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        cfg.max_active_downloads = 3;
        cfg.network_interface = Some("../eth0".into());
        assert!(cfg.save_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
