//! Configure native app storage and desktop handlers, and desktop handlers.

pub mod desktop;
mod jsonc;
mod menu;

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

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
    install_icon()?;
    let exe = std::env::current_exe()?;
    let command = desktop_executable(&exe.to_string_lossy())?;
    let apps = dirs::data_local_dir().context("no data directory")?.join("applications");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(apps.join("fbittorrent.desktop"), format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open the torrent app\nExec={command} dashboard\nIcon=fbittorrent\nTerminal=false\nCategories=Network;P2P;\n"))?;
    let desktop = apps.join("fbittorrent-magnet.desktop");
    std::fs::write(&desktop, format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open fbittorrent and add a torrent\nExec={command} add --origin handler %u\nIcon=fbittorrent\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/magnet;application/x-bittorrent;\nCategories=Network;P2P;\n"))?;
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
    fn unit_text_matches_plan() {
        let expected = r#"[Unit]
Description=fbittorrent torrent engine (started on demand)
Documentation=https://github.com/odcpw/fbittorrent
# On-demand units start often; avoid wedging after normal idle/upgrade cycles.
StartLimitIntervalSec=60
StartLimitBurst=20

[Service]
Type=notify
NotifyAccess=main
ExecStart="/fixture/fbittorrent" serve
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
"#;
        assert_eq!(super::daemon_unit(std::path::Path::new("/fixture/fbittorrent")).unwrap(), expected);
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

fn install_icon() -> Result<()> {
    let directory = dirs::data_local_dir().context("no data directory")?.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("fbittorrent.svg"), include_str!("../assets/fbittorrent.svg"))?;
    Ok(())
}
