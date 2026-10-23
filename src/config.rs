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

fn executable_on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| {
        std::fs::metadata(dir.join(program)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }))
}

fn launch_open(args: &[String], target: &std::ffi::OsStr) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    let mut child = std::process::Command::new(&args[0]).args(&args[1..]).arg(target)
        // Players outlive the CLI/TUI without retaining JSON pipes or its group.
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).process_group(0).spawn()
        .with_context(|| format!("could not launch {}", args[0]))?;
    // Catch bad executables, flags and immediate desktop-opener failures before
    // reporting a successful handoff. Playback after startup remains the player's.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "{} failed to open the target ({status})", args[0]);
            return Ok(child);
        }
        if std::time::Instant::now() >= deadline { return Ok(child); }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

impl Config {
    /// The player command accepts quoted arguments, never shell programs.
    /// Generic desktop openers must not send HTTP media streams to a browser.
    pub fn open_target(&self, target: impl AsRef<std::ffi::OsStr>) -> Result<std::process::Child> {
        self.validate()?;
        let target = target.as_ref();
        let path = std::path::Path::new(target);
        if path.is_dir() { return self.open_folder(path); }
        let stream = target.to_str().is_some_and(|s| s.starts_with("http://") || s.starts_with("https://"));
        if !stream {
            crate::public_text::openable_payload(path)?;
            anyhow::ensure!(path.is_file(), "downloaded file is missing: {}", path.display());
        }
        let media = stream || crate::daemon::watch::video(path) || path.extension().is_some_and(|ext| {
            ["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"].iter().any(|v| ext.eq_ignore_ascii_case(v))
        });
        let args = self.open_arguments(media, executable_on_path)?;
        // Absolute paths cannot be interpreted as player command-line options.
        let target = if !stream && !path.is_absolute() { std::env::current_dir()?.join(path).into_os_string() } else { target.to_owned() };
        launch_open(&args, &target)
    }

    /// Folders always go to the file manager, regardless of the player setting.
    pub fn open_folder(&self, path: impl AsRef<std::path::Path>) -> Result<std::process::Child> {
        let path = path.as_ref();
        anyhow::ensure!(path.is_dir(), "download folder is missing: {}", path.display());
        let path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        launch_open(&["xdg-open".into()], path.as_os_str())
    }

    fn open_arguments(&self, media: bool, available: impl Fn(&str) -> bool) -> Result<Vec<String>> {
        let args = shlex::split(&self.open_command).context("open_command: unmatched quote or invalid escape")?;
        let program = args.first().filter(|p| !p.is_empty()).context("open_command: must name a program")?;
        let generic = std::path::Path::new(program).file_name().is_some_and(|p| p == "xdg-open") || program == "auto";
        if generic && media {
            let player = ["mpv", "ffplay", "vlc"].into_iter().find(|p| available(p))
                .context("no media player found; install mpv or ffplay, or set open_command to a player")?;
            return Ok(vec![player.into()]);
        }
        if program == "auto" { return Ok(vec!["xdg-open".into()]); }
        Ok(args)
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
        anyhow::ensure!(shlex::split(&self.open_command).is_some_and(|args| args.first().is_some_and(|p| !p.is_empty())), "open_command: must name a program with valid quoted arguments");
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
        let root = std::env::temp_dir().join(format!("fbt settings open {}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("player.py");
        let output = root.join("arguments.json");
        std::fs::write(&script, format!("import json,sys\nopen({:?}, 'w').write(json.dumps(sys.argv[1:]))\n", output.to_string_lossy())).unwrap();
        let cfg = Config { open_command: format!("python3 '{}' --keep-open=yes --title 'a title ; $(touch escaped)'",script.display()), ..Config::default() };
        let target = root.join("video ; $(touch escaped).mkv");
        std::fs::write(&target, b"fixture").unwrap();
        assert!(cfg.open_target(&target).unwrap().wait().unwrap().success());
        let observed: Vec<String> = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(observed, ["--keep-open=yes".to_owned(), "--title".into(), "a title ; $(touch escaped)".into(), target.to_string_lossy().into_owned()]);
        println!("configured player receives flags and one literal target argument: {observed:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media_uses_a_player_for_generic_desktop_settings() {
        for command in ["xdg-open", "/usr/bin/xdg-open", "auto"] {
            let cfg = Config { open_command: command.into(), ..Config::default() };
            assert_eq!(cfg.open_arguments(true, |p| p == "mpv" || p == "ffplay").unwrap(), ["mpv"]);
            assert_eq!(cfg.open_arguments(true, |p| p == "ffplay").unwrap(), ["ffplay"]);
            assert!(cfg.open_arguments(true, |_| false).unwrap_err().to_string().contains("no media player"));
        }
        let cfg = Config { open_command: "'/path with spaces/player' --title 'literal $HOME'".into(), ..Config::default() };
        assert_eq!(cfg.open_arguments(true, |_| false).unwrap(), ["/path with spaces/player", "--title", "literal $HOME"]);
        assert_eq!(Config::default().open_arguments(false, |_| false).unwrap(), ["xdg-open"]);
    }

    #[test]
    fn launch_errors_and_unsafe_payloads_are_not_reported_as_success() {
        let root = std::env::temp_dir().join(format!("fbt-open-errors-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("movie.mkv");
        std::fs::write(&target, b"fixture").unwrap();
        let mut cfg = Config { open_command: root.join("missing-player").to_string_lossy().into(), ..Config::default() };
        assert!(cfg.open_target(&target).is_err());
        cfg.open_command = "python3 -c 'import sys; sys.exit(7)'".into();
        assert!(cfg.open_target(&target).unwrap_err().to_string().contains("failed to open"));
        assert!(cfg.open_target(root.join("missing.mkv")).is_err());
        let launcher = root.join("malicious.desktop");
        std::fs::write(&launcher, "[Desktop Entry]\nExec=touch escaped\n").unwrap();
        assert!(cfg.open_target(&launcher).unwrap_err().to_string().contains("refusing"));
        cfg.open_command = "mpv 'unterminated".into();
        assert!(cfg.validate().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folders_always_use_file_manager_with_one_literal_path() {
        // Run the launcher in a child test process: PATH/XDG changes never race
        // other tests or reach the user's desktop session.
        const CHILD: &str = "FBT_TEST_FOLDER_OPEN_ROOT";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let cfg = Config { open_command: "missing-player --invalid".into(), ..Config::default() };
            assert!(cfg.open_folder(root.join("folder ; $(touch escaped)")).unwrap().wait().unwrap().success());
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("fbt-folder-open-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder ; $(touch escaped)")).unwrap();
        let output = root.join("arguments.json");
        let opener = root.join("xdg-open");
        std::fs::write(&opener, format!("#!/usr/bin/python3\nimport json,sys\nopen({:?}, 'w').write(json.dumps(sys.argv[1:]))\n", output.to_string_lossy())).unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "config::tests::folders_always_use_file_manager_with_one_literal_path", "--nocapture"])
            .env(CHILD, &root).env("PATH", &root).env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config")).env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state")).env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime")).status().unwrap();
        assert!(status.success());
        let observed: Vec<String> = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(observed, [root.join("folder ; $(touch escaped)").to_string_lossy()]);
        assert!(!root.join("escaped").exists());
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
