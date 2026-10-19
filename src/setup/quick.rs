//! Opt-in overlay installation; preserve the bar route and unrelated user bindings.
use super::*;
const ID: &str = "oliver.torrents-quick";
const BEGIN: &str = "-- fbittorrent:quick:begin\n";
const END: &str = "-- fbittorrent:quick:end\n";

fn chord(value: &str) -> Result<String> {
    anyhow::ensure!(
        !value.chars().any(char::is_control),
        "shortcut contains controls"
    );
    let tokens = value
        .split('+')
        .map(|s| s.trim().to_ascii_uppercase())
        .collect::<Vec<_>>();
    let (key, mods) = tokens.split_last().context("empty shortcut")?;
    anyhow::ensure!(
        !mods.is_empty()
            && mods
                .iter()
                .all(|m| ["SUPER", "CTRL", "ALT", "SHIFT"].contains(&m.as_str())),
        "shortcut needs modifiers SUPER/CTRL/ALT/SHIFT and a key"
    );
    anyhow::ensure!(
        !key.is_empty()
            && key.len() <= 16
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "invalid shortcut key"
    );
    let mut parts = ["SUPER", "CTRL", "ALT", "SHIFT"]
        .into_iter()
        .filter(|m| mods.iter().any(|v| v == m))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    parts.push(key.clone());
    Ok(parts.join(" + "))
}
fn bindings(text: &str, selected: Option<&str>) -> Result<String> {
    anyhow::ensure!(text.len() <= 1_048_576, "bindings file exceeds 1 MiB");
    let mut base = text.to_owned();
    if let Some((before, rest)) = text.split_once(BEGIN) {
        let (_, after) = rest
            .split_once(END)
            .context("incomplete owned quick-palette binding; kept bindings unchanged")?;
        anyhow::ensure!(
            !after.contains(BEGIN),
            "duplicate quick-palette binding blocks"
        );
        base = format!("{before}{after}");
    } else {
        anyhow::ensure!(
            !text.contains(END),
            "incomplete owned quick-palette binding; kept bindings unchanged"
        );
    }
    if let Some(value) = selected {
        let value = chord(value)?;
        for line in base.lines().filter(|l| !l.trim_start().starts_with("--")) {
            if (line.contains("o.bind(") || line.contains("o.rebind(") || line.contains("hl.bind("))
                && line
                    .split('"')
                    .nth(1)
                    .and_then(|s| chord(s).ok())
                    .as_deref()
                    == Some(&value)
            {
                bail!("shortcut {value} is already bound; choose an unused chord with --quick");
            }
        }
        if !base.is_empty() && !base.ends_with('\n') {
            base.push('\n');
        }
        base.push_str(&format!("{BEGIN}o.bind(\"{value}\", \"Torrent quick add/search\", {{ panel = \"{ID}\" }})\n{END}"));
    }
    Ok(base)
}
fn enabled(text: &str, on: bool) -> Result<String> {
    use serde_json::{json, Map, Value};
    let current = super::super::jsonc::parse(text)?;
    let mut plugins = match current.get("plugins") {
        None => vec![],
        Some(Value::Array(v)) => v.clone(),
        _ => bail!("shell.plugins must be an array"),
    };
    let own = |v: &Value| v.get("id").and_then(Value::as_str) == Some(ID);
    if on {
        if !plugins.iter().any(own) {
            plugins.push(json!({"id":ID}));
        }
    } else {
        plugins.retain(|v| !own(v));
    }
    let mut fields = Map::from_iter([("plugins".into(), json!(plugins))]);
    if let Some(disabled) = current.get("disabledPlugins") {
        let disabled = disabled
            .as_array()
            .context("disabledPlugins must be an array")?
            .iter()
            .filter(|v| v.as_str() != Some(ID))
            .cloned()
            .collect::<Vec<_>>();
        if !disabled.is_empty() {
            fields.insert("disabledPlugins".into(), json!(disabled));
        }
    }
    let mut desired = current
        .as_object()
        .cloned()
        .context("shell config must be an object")?;
    desired.remove("plugins");
    desired.remove("disabledPlugins");
    desired.extend(fields.clone());
    if Value::Object(desired) == current {
        return Ok(text.to_owned());
    }
    super::super::jsonc::edit(
        text,
        |k| ["plugins", "disabledPlugins"].contains(&k),
        &fields,
    )
}
fn shell(actions: &mut Vec<Action>, config: &Path, on: bool) -> Result<()> {
    let path = config.join("omarchy/shell.json");
    let pending = actions
        .iter()
        .position(|a| a.kind == "write" && a.path == path);
    let old = match pending {
        Some(i) => String::from_utf8(actions[i].bytes.clone())?,
        None => existing(&path)?,
    };
    let new = enabled(&old, on)?;
    if old != new {
        if let Some(i) = pending {
            actions[i].bytes = new.into_bytes();
        } else {
            write(actions, path, new.into_bytes());
        }
    }
    Ok(())
}
pub(super) fn install(
    actions: &mut Vec<Action>,
    config: &Path,
    mode: &str,
    selected: &str,
) -> Result<()> {
    let before = actions.len();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugin")
        .join(ID)
        .canonicalize()?;
    let target = config.join("omarchy/plugins").join(ID);
    if mode == "link" {
        if std::fs::read_link(&target).ok().as_ref() != Some(&source) {
            remove(actions, target.clone());
            let mut item = action("link", target);
            item.args.push(source.display().to_string());
            actions.push(item);
        }
    } else {
        let linked = target
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
        if linked {
            remove(actions, target.clone());
        }
        copy_plan(actions, &source, &target, linked)?;
    }
    let changed = actions.len() != before;
    let path = config.join("hypr/bindings.lua");
    let old = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    write(actions, path, bindings(&old, Some(selected))?.into_bytes());
    shell(actions, config, true)?;
    if changed {
        command(actions, "omarchy-shell", &["-q", "shell", "rescanPlugins"]);
    }
    Ok(())
}
pub(super) fn uninstall(actions: &mut Vec<Action>, config: &Path) -> Result<()> {
    remove(actions, config.join("omarchy/plugins").join(ID));
    let path = config.join("hypr/bindings.lua");
    if path.exists() {
        let old = std::fs::read_to_string(&path)?;
        write(actions, path, bindings(&old, None)?.into_bytes());
    }
    if config.join("omarchy/shell.json").exists() {
        shell(actions, config, false)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_binding_is_opt_in_idempotent_and_does_not_replace_another() {
        let before = "-- User bindings\no.bind(\"SUPER + B\", \"Browser\", \"browser\")\n";
        let installed = bindings(before, Some("ctrl+super+alt+t")).unwrap();
        assert!(installed.starts_with(before));
        assert!(installed.contains("SUPER + CTRL + ALT + T"));
        assert_eq!(
            installed,
            bindings(&installed, Some("SUPER + CTRL + ALT + T")).unwrap()
        );
        assert_eq!(before, bindings(&installed, None).unwrap());
        assert!(bindings(before, Some("SUPER+B")).is_err());
        for bad in ["T", "SUPER + quote\"", "SUPER + T\n", "SUPER + $(whoami)"] {
            assert!(bindings(before, Some(bad)).is_err());
        }
        assert!(bindings(BEGIN, None).is_err());
    }
    #[test]
    fn quick_overlay_enable_preserves_bar_routing_and_other_plugins() {
        let old="{\n// keep\n\"bar\":{\"layout\":{\"right\":[\"oliver.torrents\"]}},\"plugins\":[{\"id\":\"other.overlay\"}]\n}";
        let installed = enabled(old, true).unwrap();
        assert!(installed.contains("// keep"));
        assert_eq!(installed, enabled(&installed, true).unwrap());
        for text in [&installed, &enabled(&installed, false).unwrap()] {
            let value = super::super::super::jsonc::parse(text).unwrap();
            assert_eq!(
                value["bar"],
                serde_json::json!({"layout":{"right":["oliver.torrents"]}})
            );
            assert_eq!(value["plugins"][0]["id"], "other.overlay");
        }
        let removed = enabled(&installed, false).unwrap();
        assert!(!removed.contains(ID));
    }
}
