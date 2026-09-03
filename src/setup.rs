//! `fbittorrent setup`: configure a local qbittorrent-nox as a systemd user service.

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
            "[LegalNotice]\nAccepted=true\n\n[BitTorrent]\nSession\\DefaultSavePath={save}\nSession\\Port=6881\n\n[Preferences]\nWebUI\\Enabled=true\nWebUI\\Address=127.0.0.1\nWebUI\\Port={port}\nWebUI\\LocalHostAuth=false\nWebUI\\CSRFProtection=false\nWebUI\\HostHeaderValidation=false\nGeneral\\Locale=en\n"
        );
        std::fs::write(&qcfg, body)?;
        println!("wrote {}", qcfg.display());
    } else {
        println!("kept existing {}", qcfg.display());
    }

    let unit_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit = unit_dir.join("qbittorrent-nox.service");
    std::fs::write(&unit, UNIT)?;
    println!("wrote {}", unit.display());

    let fcfg = crate::config::Config::write_default_if_missing()?;
    println!("config at {}", fcfg.display());

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
        let _ = client.set_preferences(&serde_json::json!({ "save_path": save }));
    }
    println!("qbittorrent-nox is running on http://127.0.0.1:{port} (on demand; `fbittorrent down` stops it)");
    Ok(())
}

/// Register fbittorrent as the magnet: link handler via a .desktop file + xdg-mime.
pub fn register_magnet_handler() -> Result<()> {
    let exe = std::env::current_exe()?;
    let home = dirs::home_dir().context("no home dir")?;
    let apps = home.join(".local/share/applications");
    std::fs::create_dir_all(&apps)?;
    let desktop = apps.join("fbittorrent-magnet.desktop");
    std::fs::write(&desktop, format!(
        "[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Add magnet link to qBittorrent\nExec={} add %u\nIcon=qbittorrent\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/magnet;application/x-bittorrent;\nCategories=Network;P2P;\n",
        exe.display()))?;
    for mime in ["x-scheme-handler/magnet", "application/x-bittorrent"] {
        let st = Command::new("xdg-mime").args(["default", "fbittorrent-magnet.desktop", mime]).status()?;
        if !st.success() { bail!("xdg-mime default failed for {mime}"); }
    }
    let _ = Command::new("update-desktop-database").arg(&apps).status();
    println!("magnet: and .torrent links now open with {}", exe.display());
    Ok(())
}
