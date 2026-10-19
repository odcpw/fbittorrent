//! Native setup as a deterministic plan; dry-run and execution share the same actions.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[path="quick.rs"]
mod quick;

#[derive(Default)]
pub struct Options {
    pub save: Option<String>,
    pub magnet: bool,
    pub plugin: Option<String>,
    pub quick: Option<String>,
    pub dry_run: bool,
    pub uninstall: bool,
    pub purge: bool,
}

#[derive(Serialize)]
pub struct Action {
    kind: &'static str,
    path: PathBuf,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip)]
    bytes: Vec<u8>,
}
fn action(kind: &'static str, path: impl Into<PathBuf>) -> Action {
    Action {
        kind,
        path: path.into(),
        args: Vec::new(),
        bytes: Vec::new(),
    }
}
fn write(actions: &mut Vec<Action>, path: PathBuf, bytes: Vec<u8>) {
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let mut item = action("write", path);
        item.bytes = bytes;
        actions.push(item);
    }
}
// Validate installation permissions even when the installed bytes already match.
// A healthy idempotent run still has an empty plan and writes no probe file.
fn plan_unit(actions: &mut Vec<Action>, path: PathBuf, bytes: Vec<u8>) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut parent = path.parent().context("no unit parent")?;
    while !parent.exists() {
        parent = parent.parent().context("no existing unit ancestor")?;
    }
    let name = std::ffi::CString::new(parent.as_os_str().as_bytes())?;
    if !parent.is_dir() || unsafe {
        libc::faccessat(libc::AT_FDCWD, name.as_ptr(), libc::W_OK | libc::X_OK, libc::AT_EACCESS)
    } != 0 {
        bail!("unit installation directory is not writable: {}", parent.display());
    }
    write(actions, path, bytes);
    Ok(())
}
fn command(actions: &mut Vec<Action>, name: &str, args: &[&str]) {
    let mut item = action("command", name);
    item.args = args.iter().map(|s| s.to_string()).collect();
    actions.push(item);
}
fn remove(actions: &mut Vec<Action>, path: PathBuf) {
    if path.symlink_metadata().is_ok() {
        actions.push(action("remove", path));
    }
}
fn purge(actions: &mut Vec<Action>, path: PathBuf, protected: Option<&Path>) -> Result<()> {
    if let Some(keep) = protected {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
        if resolved == keep || resolved.starts_with(keep) {
            return Ok(());
        }
        if keep.starts_with(&resolved)
            && path.is_dir()
            && !path.symlink_metadata()?.file_type().is_symlink()
        {
            let mut entries = std::fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                purge(actions, entry.path(), protected)?;
            }
            return Ok(());
        }
    }
    remove(actions, path);
    Ok(())
}
fn existing(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("{}\n".into()),
        Err(e) => Err(e.into()),
    }
}
fn copy_plan(actions: &mut Vec<Action>, source: &Path, target: &Path, force: bool) -> Result<()> {
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let dest = target.join(entry.file_name());
        if kind.is_symlink() {
            bail!(
                "plugin source contains a symlink: {}",
                entry.path().display()
            );
        }
        if kind.is_dir() {
            copy_plan(actions, &entry.path(), &dest, force)?;
        } else if kind.is_file() {
            let bytes = std::fs::read(entry.path())?;
            if force {
                let mut item = action("write", dest);
                item.bytes = bytes;
                actions.push(item);
            } else {
                write(actions, dest, bytes);
            }
        }
    }
    Ok(())
}

pub fn run(cfg: &crate::config::Config, options: Options) -> Result<Vec<Action>> {
    let config = dirs::config_dir().context("no config directory")?;
    let data = dirs::data_local_dir().context("no data directory")?;
    let mut actions = Vec::new();
    let unit = crate::api::client::unit_path()?;
    let plugin = config.join("omarchy/plugins/oliver.torrents");
    let menu = config.join("omarchy/extensions/omarchy-menu.jsonc");
    let shell = config.join("omarchy/shell.json");
    let icon = data.join("icons/hicolor/scalable/apps/fbittorrent.svg");
    let apps = data.join("applications");
    let uninstall = options.uninstall || options.purge;
    if uninstall {
        remove(&mut actions, unit.clone());
        if actions.iter().any(|a| a.path == unit) {
            command(
                &mut actions,
                "systemctl",
                &["--user", "--no-ask-password", "daemon-reload"],
            );
        }
        for name in [
            "fbittorrent.desktop",
            "fbittorrent-magnet.desktop",
            "fbittorrent-tui.desktop",
        ] {
            remove(&mut actions, apps.join(name));
        }
        remove(&mut actions, icon);
        remove(&mut actions, plugin);
        if menu.exists() {
            write(
                &mut actions,
                menu.clone(),
                super::menu::edit(&existing(&menu)?, true)?.into_bytes(),
            );
        }
        if shell.exists() {
            write(
                &mut actions,
                shell.clone(),
                super::menu::bar(&existing(&shell)?, true)?.into_bytes(),
            );
        }
        quick::uninstall(&mut actions,&config)?;
        // MIME defaults are ours only while they name our handler. Never change
        // another application's chosen association during uninstall.
        let mime = config.join("mimeapps.list");
        if mime.exists() {
            let old = std::fs::read_to_string(&mime)?;
            let new = old
                .lines()
                .filter_map(|line| {
                    let Some((key, value)) = line.split_once('=') else {
                        return Some(line.to_owned());
                    };
                    if !["x-scheme-handler/magnet", "application/x-bittorrent"]
                        .contains(&key.trim())
                    {
                        return Some(line.to_owned());
                    }
                    let entries = value
                        .split(';')
                        .filter(|name| !name.is_empty() && *name != "fbittorrent-magnet.desktop")
                        .collect::<Vec<_>>();
                    if !value
                        .split(';')
                        .any(|name| name == "fbittorrent-magnet.desktop")
                    {
                        Some(line.to_owned())
                    } else if entries.is_empty() {
                        None
                    } else {
                        Some(format!("{key}={};", entries.join(";")))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            write(&mut actions, mime, new.into_bytes());
        }
        if options.purge {
            let protected = cfg
                .save_path
                .as_ref()
                .map(PathBuf::from)
                .map(|path| path.canonicalize().unwrap_or(path));
            for path in [
                crate::config::data_dir(),
                config.join("fbittorrent"),
                dirs::state_dir()
                    .unwrap_or_else(|| data.join("state"))
                    .join("fbittorrent"),
                dirs::cache_dir()
                    .unwrap_or_else(|| data.join("cache"))
                    .join("fbittorrent"),
            ] {
                purge(&mut actions, path, protected.as_deref())?;
            }
        }
    } else {
        let mut saved = cfg.clone();
        let save = options
            .save
            .or(saved.save_path.clone())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::download_dir().unwrap_or_else(|| crate::config::data_dir().join("downloads"))
            });
        if !save.is_dir() {
            actions.push(action("mkdir", save.clone()));
        }
        saved.save_path = Some(save.display().to_string());
        saved.validate()?;
        if !crate::config::config_path().exists() || cfg.save_path != saved.save_path {
            write(
                &mut actions,
                crate::config::config_path(),
                toml::to_string_pretty(&saved)?.into_bytes(),
            );
        }
        let executable = std::env::current_exe()?;
        plan_unit(
            &mut actions,
            unit.clone(),
            super::daemon_unit(&executable)?.into_bytes(),
        )?;
        if let Some(item) = actions.iter_mut().find(|a| a.path == unit) {
            item.kind = "unit";
            item.args = vec![
                "systemctl".into(),
                "--user".into(),
                "--no-ask-password".into(),
                "daemon-reload".into(),
            ];
        }
        if options.magnet {
            let exe = super::desktop_executable(&executable.to_string_lossy())?;
            let entries=[
                ("fbittorrent.desktop",format!("[Desktop Entry]\nType=Application\nName=fbittorrent\nComment=Open the torrent app\nExec={exe} dashboard\nIcon=fbittorrent\nTerminal=false\nCategories=Network;P2P;\n")),
                ("fbittorrent-magnet.desktop",format!("[Desktop Entry]\nType=Application\nName=fbittorrent\nExec={exe} add --origin handler %u\nIcon=fbittorrent\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/magnet;application/x-bittorrent;\nCategories=Network;P2P;\n")),
                ("fbittorrent-tui.desktop", "[Desktop Entry]\nType=Application\nName=fbittorrent Terminal\nExec=omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui\nIcon=fbittorrent\nTerminal=false\nCategories=Network;P2P;\n".into())];
            let changed = entries.iter().any(|(name, text)| {
                std::fs::read(apps.join(name)).ok().as_deref() != Some(text.as_bytes())
            });
            for (name, text) in entries {
                write(&mut actions, apps.join(name), text.into_bytes());
            }
            write(
                &mut actions,
                icon,
                include_bytes!("../../assets/fbittorrent.svg").to_vec(),
            );
            let mime = config.join("mimeapps.list");
            let old = match std::fs::read_to_string(&mime) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(e.into()),
            };
            for kind in ["x-scheme-handler/magnet", "application/x-bittorrent"] {
                if changed
                    || !old.lines().any(|line| {
                        line.trim() == format!("{kind}=fbittorrent-magnet.desktop")
                            || line.trim() == format!("{kind}=fbittorrent-magnet.desktop;")
                    })
                {
                    command(
                        &mut actions,
                        "xdg-mime",
                        &["default", "fbittorrent-magnet.desktop", kind],
                    );
                }
            }
            if changed {
                command(
                    &mut actions,
                    "update-desktop-database",
                    &[apps.to_str().context("applications path not UTF-8")?],
                );
            }
        }
        if let Some(mode) = &options.plugin {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("plugin/oliver.torrents")
                .canonicalize()
                .context("plugin source unavailable; run setup from an installed checkout")?;
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(source.join("manifest.json"))?)?;
            if manifest["id"] != "oliver.torrents" {
                bail!("unexpected plugin source identity");
            }
            let metadata = plugin.symlink_metadata().ok();
            if mode == "link" {
                if std::fs::read_link(&plugin).ok().as_ref() != Some(&source) {
                    if metadata.is_some() {
                        remove(&mut actions, plugin.clone());
                    }
                    let mut item = action("link", plugin.clone());
                    item.args.push(source.display().to_string());
                    actions.push(item);
                }
            } else if mode == "copy" {
                let linked = metadata.is_some_and(|m| m.file_type().is_symlink());
                if linked {
                    remove(&mut actions, plugin.clone());
                }
                copy_plan(&mut actions, &source, &plugin, linked)?;
            } else {
                bail!("unknown plugin mode");
            }
            let old = existing(&shell)?;
            let new = super::menu::bar(&old, false)?;
            if old != new {
                write(&mut actions, shell.clone(), new.into_bytes());
                command(
                    &mut actions,
                    "omarchy",
                    &["plugin", "enable", "oliver.torrents"],
                );
            }
            write(
                &mut actions,
                menu.clone(),
                super::menu::edit(&existing(&menu)?, false)?.into_bytes(),
            );
        }
        if let Some(chord)=options.quick {
            anyhow::ensure!(options.plugin.is_some() || plugin.join("manifest.json").is_file(),"install the shared service first: fbittorrent setup --plugin copy --quick");
            quick::install(&mut actions,&config,options.plugin.as_deref().unwrap_or("copy"),&chord)?;
        }
    }
    if !options.dry_run {
        for item in &actions {
            apply(item)?;
        }
    }
    Ok(actions)
}
fn apply(item: &Action) -> Result<()> {
    match item.kind {
        "mkdir" => std::fs::create_dir_all(&item.path)?,
        "remove" => {
            let meta = std::fs::symlink_metadata(&item.path)?;
            if meta.is_dir() && !meta.file_type().is_symlink() {
                std::fs::remove_dir_all(&item.path)?;
            } else {
                std::fs::remove_file(&item.path)?;
            }
        }
        "unit" => {
            std::fs::create_dir_all(item.path.parent().context("no unit parent")?)?;
            super::replace_unit(&item.path, &item.bytes, super::reload_manager)?;
        }
        "write" => {
            std::fs::create_dir_all(item.path.parent().context("no parent")?)?;
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let staged = super::unit_temporary_path(&item.path);
            let cleanup = super::UnitTemporary(staged.clone());
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&staged)?;
            file.write_all(&item.bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&staged, &item.path)?;
            std::fs::File::open(item.path.parent().unwrap())?.sync_all()?;
            drop(cleanup);
        }
        "link" => {
            std::fs::create_dir_all(item.path.parent().unwrap())?;
            std::os::unix::fs::symlink(&item.args[0], &item.path)?;
        }
        "command" => {
            let status = std::process::Command::new(&item.path)
                .args(&item.args)
                .stdout(std::process::Stdio::null())
                .status()
                .with_context(|| format!("running {}", item.path.display()))?;
            if !status.success() {
                bail!("{} {:?} failed", item.path.display(), item.args);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_plan_requires_writable_directory_before_and_after_install() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = std::env::temp_dir().join(format!("fbt-unit-plan-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let unit = root.join("fixture.service");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
        let missing = super::plan_unit(&mut Vec::new(), unit.clone(), b"static unit".to_vec());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(missing.is_err(), "uninstalled unit requires a writable destination");
        assert!(!unit.exists());
        std::fs::write(&unit, b"static unit").unwrap();
        let before = std::fs::metadata(&unit).unwrap();
        let mut actions = Vec::new();
        super::plan_unit(&mut actions, unit.clone(), b"static unit".to_vec()).unwrap();
        assert!(actions.is_empty(), "healthy setup stays idempotent");
        assert_eq!(std::fs::metadata(&unit).unwrap().ino(), before.ino());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
        let installed = super::plan_unit(&mut actions, unit.clone(), b"static unit".to_vec());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(installed.is_err(), "equal unit bytes cannot bypass installation permissions");
        assert!(actions.is_empty());
        assert_eq!(std::fs::metadata(&unit).unwrap().ino(), before.ino());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
        println!("uninstalled/equal unit permission refusal and healthy no-write idempotence verified");
    }

    #[test]
    fn purge_protects_nested_payload_directory() {
        let root = std::env::temp_dir().join(format!("fbt-purge-control-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let payload = root.join("downloads");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("movie.mkv"), b"payload").unwrap();
        std::fs::write(root.join("rss.json"), b"records").unwrap();
        let mut actions = Vec::new();
        super::purge(&mut actions, root.clone(), Some(&payload)).unwrap();
        assert_eq!(actions.len(), 1);
        for action in &actions {
            super::apply(action).unwrap();
        }
        assert_eq!(
            std::fs::read(payload.join("movie.mkv")).unwrap(),
            b"payload"
        );
        assert!(!root.join("rss.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
