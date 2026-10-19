//! Configure native app storage and desktop handlers, with explicit legacy setup.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const UNIT: &str = "[Unit]
Description=qBittorrent-nox (fbittorrent backend)
After=network.target

[Service]
Type=exec
ExecStart=/usr/bin/qbittorrent-nox
Restart=on-failure
RestartSec=3
# The WebUI is bound to 127.0.0.1 in ~/.config/qBittorrent/qBittorrent.conf

[Install]
WantedBy=default.target
";

pub fn run(port: u16, save_path: Option<String>) -> Result<()> {
    let mut cfg = crate::config::Config::load()?;
    if cfg.native() {
        let save = save_path.or(cfg.save_path.clone()).unwrap_or_else(|| {
            dirs::download_dir().unwrap_or_else(|| crate::config::data_dir().join("downloads"))
                .display().to_string()
        });
        std::fs::create_dir_all(&save).context("creating the app download folder")?;
        cfg.save_path = Some(save.clone());
        let path = crate::config::config_path();
        cfg.save()?;
        eprintln!("config at {}; downloads in {save}", path.display());
        install_unit()?;
        eprintln!("the native daemon starts on demand; nothing is enabled at login");
        return Ok(());
    }
    let nox = PathBuf::from("/usr/bin/qbittorrent-nox");
    if !nox.exists() {
        bail!("qbittorrent-nox is not installed. Run:  omarchy pkg add qbittorrent-nox");
    }
    let home = dirs::home_dir().context("no home dir")?;
    let save = save_path.unwrap_or_else(|| home.join("Downloads").display().to_string());
    std::fs::create_dir_all(&save)?;

    // qBittorrent config: only written when absent so an existing setup is never clobbered.
    let qcfg_dir = home.join(".config/qBittorrent");
    let qcfg = qcfg_dir.join("qBittorrent.conf");
    if !qcfg.exists() {
        std::fs::create_dir_all(&qcfg_dir)?;
        let body = format!(
            "[LegalNotice]\nAccepted=true\n\n[BitTorrent]\nSession\\DefaultSavePath={save}\nSession\\Port=6881\n\n[Preferences]\nWebUI\\Enabled=true\nWebUI\\Address=127.0.0.1\nWebUI\\Port={port}\nWebUI\\LocalHostAuth=false\nWebUI\\CSRFProtection=true\nWebUI\\HostHeaderValidation=true\nGeneral\\Locale=en\n"
        );
        std::fs::write(&qcfg, body)?;
        eprintln!("wrote {}", qcfg.display());
    } else {
        eprintln!("kept existing {}", qcfg.display());
    }

    let unit_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit = unit_dir.join("qbittorrent-nox.service");
    std::fs::write(&unit, UNIT)?;
    eprintln!("wrote {}", unit.display());

    let fcfg = crate::config::Config::write_default_if_missing()?;
    eprintln!("config at {}", fcfg.display());

    // On-demand only: never enabled at login. `fbittorrent` starts it when needed.
    for args in [
        vec!["--user", "daemon-reload"],
        vec!["--user", "disable", "qbittorrent-nox.service"],
        vec!["--user", "start", "qbittorrent-nox.service"],
    ] {
        let st = Command::new("systemctl").args(&args).status().context("running systemctl")?;
        if !st.success() { bail!("systemctl {} failed", args.join(" ")); }
    }
    // Apply the download folder on an existing install too (the conf file above is only written once).
    let cfg = crate::config::Config::load()?;
    let client = crate::qbt::Client::from_config(&cfg);
    if crate::backend::ensure_running(&cfg, &client).is_ok() {
        let _ = client.login();
        // Local auth stays off (the WebUI only listens on 127.0.0.1), but CSRF and
        // host-header checks must stay ON: without them any web page you visit could
        // POST to localhost and add or delete torrents. fbittorrent sends no Origin
        // header and uses the loopback host, so both checks pass for it.
        let _ = client.set_preferences(&serde_json::json!({
            "save_path": save,
            "web_ui_csrf_protection_enabled": true,
            "web_ui_host_header_validation_enabled": true,
        }));
    }
    eprintln!("qbittorrent-nox is running on http://127.0.0.1:{port} (on demand; `fbittorrent down` stops it)");
    Ok(())
}


/// Static on-demand unit: deliberately no Install section or login enablement.
pub fn daemon_unit(exe: &std::path::Path) -> Result<String> {
    let path = exe.to_str().context("executable path is not UTF-8")?;
    if !exe.is_absolute() || path.chars().any(char::is_control) { bail!("invalid daemon executable path"); }
    // Specifiers expand in the executable word; environment variables do not.
    // Escape percent while retaining a literal dollar in current_exe's path.
    let path = path.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    Ok(format!(r#"[Unit]
Description=fbittorrent torrent engine (started on demand)
Documentation=https://github.com/odcpw/fbittorrent
# On-demand units start often; avoid wedging after normal idle/upgrade cycles.
StartLimitIntervalSec=60
StartLimitBurst=20

[Service]
Type=notify
NotifyAccess=main
ExecStart="{path}" serve
KillSignal=SIGTERM
# Signal the owner first; kill any remaining children only at the deadline.
KillMode=mixed
TimeoutStopSec=20
Restart=on-failure
RestartSec=2
RestartForceExitStatus=75
SuccessExitStatus=75
NoNewPrivileges=yes
# Static by design: no login enablement links.
"#))
}

fn reload_manager() -> Result<()> {
    if !crate::api::client::manager_status(&["daemon-reload"], std::time::Duration::from_secs(10))? {
        bail!("systemctl --user daemon-reload failed");
    }
    Ok(())
}

fn install_unit() -> Result<()> {
    let unit = crate::api::client::unit_path()?;
    std::fs::create_dir_all(unit.parent().unwrap())?;
    replace_unit(&unit, daemon_unit(&std::env::current_exe()?)?.as_bytes(), reload_manager)?;
    eprintln!("wrote {} (static, not enabled)", unit.display());
    Ok(())
}

struct UnitTemporary(PathBuf);

impl Drop for UnitTemporary {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

fn unit_temporary_path(unit: &std::path::Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    unit.with_file_name(format!(".{}.{}-{}.tmp", unit.file_name().unwrap().to_string_lossy(),
        std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)))
}

/// Same-directory creation checks directory writability, unlike truncating an
/// existing writable inode. Keep the old inode until reload succeeds so any
/// subsequent failure can restore it by another atomic rename.
fn replace_unit(unit: &std::path::Path, body: &[u8], reload: impl FnOnce() -> Result<()>) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let (temporary, mut file) = loop {
        let path = unit_temporary_path(unit);
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o644).open(&path) {
            Ok(file) => break (UnitTemporary(path), file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("creating same-directory unit temporary"),
        }
    };
    file.write_all(body)?;
    file.sync_all()?;
    let mut backup = match std::fs::symlink_metadata(unit) {
        Ok(metadata) => {
            if !metadata.is_file() { bail!("existing unit is not a regular file"); }
            Some(loop {
                let path = unit_temporary_path(unit);
                match std::fs::hard_link(unit, &path) {
                    Ok(()) => break UnitTemporary(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error).context("preserving previous unit"),
                }
            })
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let directory = std::fs::File::open(unit.parent().unwrap())?;
    std::fs::rename(&temporary.0, unit)?;
    if let Err(error) = directory.sync_all().map_err(anyhow::Error::from).and_then(|_| reload()) {
        let rollback = match &backup {
            Some(backup) => std::fs::rename(&backup.0, unit),
            None => std::fs::remove_file(unit),
        }.and_then(|_| directory.sync_all());
        if let Err(rollback_error) = rollback {
            let recovery = backup.take().map(|backup| {
                let path = backup.0.clone();
                std::mem::forget(backup);
                path
            });
            bail!("{error:#}; restoring previous unit failed: {rollback_error}; previous inode retained at {recovery:?}");
        }
        return Err(error);
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let unit = crate::api::client::unit_path()?;
    match std::fs::remove_file(&unit) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error.into()),
    }
    reload_manager()?;
    eprintln!("removed {}; saved data is kept", unit.display());
    Ok(())
}

/// Register fbittorrent as the magnet: link handler via a .desktop file + xdg-mime.
pub fn register_magnet_handler() -> Result<()> {
    let exe = std::env::current_exe()?;
    let command = desktop_executable(&exe.to_string_lossy())?;
    let apps = dirs::data_local_dir().context("no data directory")?.join("applications");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(apps.join("fbittorrent.desktop"), format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open the torrent app\nExec={command} dashboard\nIcon=qbittorrent\nTerminal=false\nCategories=Network;P2P;\n"))?;
    let desktop = apps.join("fbittorrent-magnet.desktop");
    std::fs::write(&desktop, format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open fbittorrent and add a torrent\nExec={command} add --origin handler %u\nIcon=qbittorrent\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/magnet;application/x-bittorrent;\nCategories=Network;P2P;\n"))?;
    for mime in ["x-scheme-handler/magnet", "application/x-bittorrent"] {
        let st = Command::new("xdg-mime").args(["default", "fbittorrent-magnet.desktop", mime]).status()?;
        if !st.success() { bail!("xdg-mime default failed for {mime}"); }
    }
    let _ = Command::new("update-desktop-database").arg(&apps).status();
    eprintln!("magnet: and .torrent links now open with {}", exe.display());
    Ok(())
}

fn desktop_executable(path: &str) -> Result<String> {
    if path.contains('=') || path.chars().any(char::is_control) {
        bail!("executable path contains unsupported desktop-entry characters");
    }
    // xdg-open's generic launcher does not unquote its first word. Ordinary
    // install paths need no quotes; reserve them for paths that require them.
    if path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)) {
        return Ok(path.to_owned());
    }
    let escaped = path.replace('\\', "\\\\\\\\").replace('"', "\\\\\"")
        .replace('`', "\\\\`").replace('$', "\\\\$").replace('%', "%%");
    Ok(format!("\"{escaped}\""))
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_replacement_is_atomic_and_preserves_previous_on_failure() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let directory = std::env::temp_dir().join(format!("fbt-unit-atomic-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let unit = directory.join("fixture.service");
        std::fs::write(&unit, "previous").unwrap();
        let inode = std::fs::metadata(&unit).unwrap().ino();
        let failure = super::replace_unit(&unit, b"replacement", || {
            assert_eq!(std::fs::read(&unit).unwrap(), b"replacement");
            anyhow::bail!("reload failed")
        });
        assert!(failure.unwrap_err().to_string().contains("reload failed"));
        assert_eq!(std::fs::metadata(&unit).unwrap().ino(), inode);
        assert_eq!(std::fs::read(&unit).unwrap(), b"previous");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        let refused = super::replace_unit(&unit, b"forbidden", || panic!("must not reload"));
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(refused.is_err(), "same-directory temporary must require directory write permission");
        assert_eq!(std::fs::metadata(&unit).unwrap().ino(), inode);
        assert_eq!(std::fs::read(&unit).unwrap(), b"previous");
        super::replace_unit(&unit, b"installed", || Ok(())).unwrap();
        assert_ne!(std::fs::metadata(&unit).unwrap().ino(), inode);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
        println!("atomic unit rename, failed reload rollback and 0500 directory refusal verified");
    }

    #[test]
    fn static_daemon_unit_policy_and_exec_escaping() {
        let unit = super::daemon_unit(std::path::Path::new("/tmp/a path/%/$/fbittorrent")).unwrap();
        for line in ["Type=notify", "NotifyAccess=main", "KillMode=mixed", "TimeoutStopSec=20", "Restart=on-failure", "RestartSec=2", "RestartForceExitStatus=75", "SuccessExitStatus=75", "StartLimitIntervalSec=60", "StartLimitBurst=20", "NoNewPrivileges=yes"] {
            assert!(unit.lines().any(|actual| actual == line), "missing {line}");
        }
        assert!(unit.contains("ExecStart=\"/tmp/a path/%%/$/fbittorrent\" serve"));
        assert!(!unit.contains("[Install]"));
        assert!(super::daemon_unit(std::path::Path::new("relative")).is_err());
        println!("static notify unit: policy and escaped absolute executable verified");
    }

    #[test]
    fn desktop_handler_command_preserves_paths_and_generic_launcher_compatibility() {
        assert_eq!(super::desktop_executable("/home/oliver/.local/bin/fbittorrent").unwrap(),
            "/home/oliver/.local/bin/fbittorrent");
        assert_eq!(super::desktop_executable("/tmp/with space/fbittorrent").unwrap(),
            "\"/tmp/with space/fbittorrent\"");
        assert!(super::desktop_executable("/tmp/line\nbreak").is_err());
        assert!(super::desktop_executable("/tmp/equal=sign").is_err());
        assert!(super::desktop_executable("/tmp/tab\tname").is_err());
        assert_eq!(super::desktop_executable("/tmp/percent%/fbittorrent").unwrap(),
            "\"/tmp/percent%%/fbittorrent\"");
    }
}
