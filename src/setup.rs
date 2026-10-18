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
        eprintln!("the native engine runs while the fbittorrent app is open");
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

/// Register fbittorrent as the magnet: link handler via a .desktop file + xdg-mime.
pub fn register_magnet_handler() -> Result<()> {
    let exe = std::env::current_exe()?;
    let command = desktop_executable(&exe.to_string_lossy())?;
    let home = dirs::home_dir().context("no home dir")?;
    let apps = home.join(".local/share/applications");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(apps.join("fbittorrent.desktop"), format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open the torrent app\nExec={command} up\nIcon=qbittorrent\nTerminal=false\nCategories=Network;P2P;\n"))?;
    let desktop = apps.join("fbittorrent-magnet.desktop");
    std::fs::write(&desktop, format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open fbittorrent and add a torrent\nExec={command} add %u\nIcon=qbittorrent\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/magnet;application/x-bittorrent;\nCategories=Network;P2P;\n"))?;
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
