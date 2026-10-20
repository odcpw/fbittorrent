//! User configuration: ~/.config/fbittorrent/config.toml

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Incoming peer port for the embedded engine.
    pub peer_port: u16,
    /// Maximum simultaneously downloading torrents; additional downloads wait.
    pub max_active_downloads: usize,
    /// Peer encryption policy: off, prefer, or require.
    pub peer_encryption: String,
    /// Require this interface, or "proton" for automatic ProtonVPN discovery.
    pub network_interface: Option<String>,
    /// Payload upload permission, independent of speed limits.
    pub upload: String,
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
    /// Stop an otherwise idle daemon after this delay; 0 disables the idle net.
    pub idle_exit_secs: u64,
    /// Preferred release quality for followed shows ("2160p", "1080p", "720p", "480p").
    pub show_quality: String,
    /// Disable hover-name motion; selected rows are always static.
    pub reduce_motion: bool,
    pub handler: String,
    pub open_command: String,
    pub clipboard_watch: bool,
    pub download_limit_bps: Option<u32>,
    pub upload_limit_bps: Option<u32>,
    pub alt_download_limit_bps: Option<u32>,
    pub alt_upload_limit_bps: Option<u32>,
    pub alt_limits_on: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            peer_port: 6882,
            max_active_downloads: 3,
            peer_encryption: "prefer".into(),
            network_interface: None,
            upload: "off".into(),
            save_path: None,
            category: None,
            notify: true,
            timeout_secs: 10,
            autostart: true,
            idle_exit_secs: 300,
            show_quality: "1080p".into(),
            reduce_motion: false,
            handler: "confirm".into(),
            open_command: "xdg-open".into(),
            clipboard_watch: false,
            download_limit_bps: None,
            upload_limit_bps: None,
            alt_download_limit_bps: None,
            alt_upload_limit_bps: None,
            alt_limits_on: false,
        }
    }
}

pub fn upload_off() -> String { "off".into() }

pub fn config_dir() -> PathBuf {
    require_home(dirs::config_dir(), "configuration").join("fbittorrent")
}

/// Without a home directory there is no private place for configuration,
/// passkeys or caches. A literal "~/.config" (relative to the working
/// directory) or a shared "/tmp" fallback would put them somewhere other
/// users or processes control, so refuse to run instead.
pub fn require_home(directory: Option<PathBuf>, what: &str) -> PathBuf {
    match directory.filter(|path| path.is_absolute()) {
        Some(path) => path,
        None => {
            ::std::eprintln!("fbittorrent: error: no home directory for {what} (set HOME); refusing a relative or shared fallback");
            std::process::exit(78)
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl Config {
    /// The command is an explicit local user preference. Pass the target as a
    /// separate, quoted positional argument so filenames never become code.
    pub fn open_target(&self, target: impl AsRef<std::ffi::OsStr>) -> Result<std::process::Child> {
        self.validate()?;
        Ok(std::process::Command::new("sh")
            .arg("-c").arg(format!("exec {} \"$1\"", self.open_command))
            .arg("fbittorrent-open").arg(target)
            // A player outlives the CLI/TUI. Inheriting their pipes delays EOF
            // for JSON callers and lets player output corrupt the TUI frame.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).spawn()?)
    }

    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: toml::Value=toml::from_str(&raw).map_err(|error:toml::de::Error| {
            let line=error.span().map(|span|raw.as_bytes()[..span.start.min(raw.len())].iter().filter(|b|**b==b'\n').count()+1).unwrap_or(1);
            crate::agent::AppError::new(crate::agent::Code::Usage,format!("config.toml has a syntax error near line {line}. Fix the file, then retry.")).hint(format!("Edit {}",path.display()))
        })?;
        if let Some(table)=parsed.as_table() {
            let known=serde_json::to_value(Self::default()).unwrap();
            for key in table.keys().filter(|key|known.get(key.as_str()).is_none()) {
                if let Some(correct)=known.as_object().unwrap().keys().find(|correct|near_key(key,correct)) {
                    return Err(crate::agent::AppError::new(crate::agent::Code::Usage,format!("Unknown setting {key:?} in config.toml. Use {correct:?}, then retry.")).into());
                }
            }
        }
        let cfg: Self = parsed.try_into().map_err(|_|crate::agent::AppError::new(crate::agent::Code::Usage,"config.toml has a setting with the wrong type. Check the setting values, then retry."))?;
        cfg.validate().map_err(|error|crate::agent::AppError::new(crate::agent::Code::Usage,format!("Invalid config.toml setting: {error}. Fix the file, then retry.")))?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.idle_exit_secs == 0 || self.idle_exit_secs >= 30,
                        "idle_exit_secs must be 0 or at least 30 seconds");
        anyhow::ensure!(self.max_active_downloads > 0, "max_active_downloads: must be at least 1");
        anyhow::ensure!(matches!(self.peer_encryption.as_str(), "off" | "prefer" | "require"), "peer_encryption: must be off, prefer, or require");
        if let Some(name) = &self.network_interface {
            anyhow::ensure!(!name.is_empty() && name.len() < libc::IFNAMSIZ && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')), "network_interface: choose proton or a valid network interface name");
        }
        anyhow::ensure!(matches!(self.upload.as_str(), "off" | "on"), "upload: must be off or on");
        anyhow::ensure!(self.timeout_secs > 0, "timeout_secs: must be positive");
        anyhow::ensure!(matches!(self.handler.as_str(), "add" | "confirm"), "handler: must be add or confirm");
        anyhow::ensure!(!self.open_command.trim().is_empty(), "open_command: must not be empty");
        anyhow::ensure!(!self.show_quality.trim().is_empty(), "show_quality: must not be empty");
        anyhow::ensure!(self.save_path.as_ref().is_none_or(|p| !p.trim().is_empty()), "save_path: must not be empty");
        for (key, limit) in [
            ("download_limit_bps", self.download_limit_bps), ("upload_limit_bps", self.upload_limit_bps),
            ("alt_download_limit_bps", self.alt_download_limit_bps), ("alt_upload_limit_bps", self.alt_upload_limit_bps),
        ] {
            anyhow::ensure!(limit.is_none_or(|v| v >= 16_384), "{key}: must be at least 16384 or null for unlimited");
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> { self.save_to(&config_path()) }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        self.validate()?;
        let directory = path.parent().context("settings path has no parent")?;
        std::fs::create_dir_all(directory)?;
        let staged = directory.join(format!(".config-{}-{}.tmp", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&staged)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
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
}

#[cfg(test)]
mod idle_tests {
    use super::*;
    #[test]
    fn idle_delay_bounds_and_default() {
        let mut cfg = Config::default();
        assert_eq!(cfg.idle_exit_secs, 300);
        for seconds in [0, 30, 300] { cfg.idle_exit_secs = seconds; cfg.validate().unwrap(); }
        for seconds in [1, 2, 29] { cfg.idle_exit_secs = seconds; assert!(cfg.validate().is_err()); }
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
    fn open_command_keeps_arguments_and_literal_target() {
        let root = std::env::temp_dir().join(format!("fbt-settings-open-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("player.py");
        let output = root.join("arguments.json");
        std::fs::write(&script, format!("import json,sys\nopen({:?}, 'w').write(json.dumps(sys.argv[1:]))\n", output.to_string_lossy())).unwrap();
        let cfg = Config { open_command: format!("python3 {} --keep-open=yes",script.display()), ..Config::default() };
        let target = root.join("video ; $(touch escaped).mkv");
        assert!(cfg.open_target(&target).unwrap().wait().unwrap().success());
        let observed: Vec<String> = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(observed, ["--keep-open=yes".to_owned(), target.to_string_lossy().into_owned()]);
        println!("configured player receives flags and one literal target argument: {observed:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_save_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("fbittorrent-config-private-{}", std::process::id()));
        let path = dir.join("config.toml");
        Config::default().save_to(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut cfg = Config::default();
        cfg.max_active_downloads = 7;
        cfg.save_to(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let saved: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.max_active_downloads, 7);
        println!("{}: {mode:o} after config save", path.display());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn existing_settings_gain_queue_and_encryption_defaults() {
        let cfg: Config = toml::from_str("engine = 'native'\n").unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.max_active_downloads, 3);
        assert_eq!(cfg.upload, "off");
        assert_eq!(cfg.peer_encryption, "prefer");
        assert_eq!(cfg.network_interface, None);
    }
    #[test]
    fn reduce_motion_defaults_false_and_roundtrips_in_private_config() {
        let mut cfg:Config=toml::from_str("engine='native'").unwrap();
        assert!(!cfg.reduce_motion);
        cfg.reduce_motion=true;
        let dir=std::env::temp_dir().join(format!("fbt-p406-motion-{}",std::process::id()));
        let path=dir.join("config.toml");
        cfg.save_to(&path).unwrap();
        let saved:Config=toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.reduce_motion);
        println!("reduce_motion=true persisted to {}",path.display());
        std::fs::remove_dir_all(dir).unwrap();
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

#[cfg(test)]
mod saved_settings_tests {
    use super::*;
    #[test]
    fn current_settings_keep_their_values() {
        let input=r#"engine="native"
peer_port=16882
save_path="/tmp/private-fixture/downloads"
backend_unit="unused.service"
url="http://127.0.0.1:9"
username="unused"
sequential=false
first_last=false
"#;
        let cfg:Config=toml::from_str(input).unwrap();cfg.validate().unwrap();
        assert_eq!(cfg.peer_port,16882);assert_eq!(cfg.save_path.as_deref(),Some("/tmp/private-fixture/downloads"));
        assert!(!toml::to_string(&cfg).unwrap().contains("engine"),"retired keys are dropped on the next save");
    }
}

// Recognize a one-character misspelling without rejecting unrelated keys left
// by other tools. Values never enter the diagnostic (they may contain secrets).
fn near_key(a:&str,b:&str)->bool {
    if !a.is_ascii() || a.len()>128 || a.len().abs_diff(b.len())>1 {return false;}
    let a=a.as_bytes();let b=b.as_bytes();
    let prefix=a.iter().zip(b).take_while(|(a,b)|a==b).count();
    let (a,b)=(&a[prefix..],&b[prefix..]);
    if a.len()==b.len() {a.len()>=2 && a[0]==b[1] && a[1]==b[0] && a[2..]==b[2..] || !a.is_empty() && a[1..]==b[1..]}
    else if a.len()>b.len() {a[1..]==*b}else{*a==b[1..]}
}

#[cfg(test)]
#[test]
fn misspelled_settings_are_recognized_without_values() {
    for key in ["max_active_downlods","max_active_donwloads","max_active_download","max_activx_downloads"] {assert!(near_key(key,"max_active_downloads"),"{key}");}
    assert!(!near_key("arbitrary_other_tool","max_active_downloads"));
    assert!(!near_key("upload","upload"));
}
