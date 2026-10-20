//! Shipping assets live in the binary; development links use a runtime checkout.
use super::*;

pub(super) struct Bundle {
    pub id: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
}
include!(concat!(env!("OUT_DIR"), "/plugins.rs"));

pub(super) enum Source {
    Embedded(&'static Bundle),
    Checkout(PathBuf),
}

fn checkout_source(id: &str, starts: &[PathBuf]) -> Option<PathBuf> {
    for root in starts.iter().flat_map(|start| start.ancestors()) {
        if !root.join("Cargo.toml").is_file() || !root.join("src/main.rs").is_file() {
            continue;
        }
        let source = root.join("plugin").join(id);
        let manifest = std::fs::read(source.join("manifest.json")).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if manifest.as_ref().is_some_and(|manifest| manifest["id"] == id) {
            if let Ok(source) = source.canonicalize() { return Some(source); }
        }
    }
    None
}

pub(super) fn install(actions: &mut Vec<Action>, config: &Path, bundle: &'static Bundle, mode: &str) -> Result<Source> {
    anyhow::ensure!(matches!(mode, "copy" | "link"), "unknown plugin mode");
    let target = config.join("omarchy/plugins").join(bundle.id);
    if mode == "link" {
        // Never use CARGO_MANIFEST_DIR here: it belongs to the build machine.
        let starts = std::env::current_dir().into_iter()
            .chain(std::env::current_exe().into_iter().filter_map(|exe| exe.parent().map(Path::to_path_buf)))
            .collect::<Vec<_>>();
        if let Some(source) = checkout_source(bundle.id, &starts) {
            if std::fs::read_link(&target).ok().as_ref() != Some(&source) {
                remove(actions, target.clone());
                let mut item = action("link", target);
                item.args.push(source.display().to_string());
                actions.push(item);
            }
            return Ok(Source::Checkout(source));
        }
        crate::agent::warning("plugin_link_copy", format!(
            "{}: no source checkout found at runtime; installing the embedded plugin as a copy instead of a link",
            bundle.id), true);
    }
    let linked = target.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink());
    if linked { remove(actions, target.clone()); }
    for (relative, bytes) in bundle.files {
        let path = target.join(relative);
        if linked {
            // Reads through the old link may match, but that link is being removed.
            let mut item = action("write", path);
            item.bytes = bytes.to_vec();
            actions.push(item);
        } else {
            write(actions, path, bytes.to_vec());
        }
    }
    Ok(Source::Embedded(bundle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bundles_have_valid_manifests_and_only_shipping_files() {
        for bundle in [&TORRENTS, &QUICK] {
            let manifest = bundle.files.iter().find(|(path, _)| *path == "manifest.json").unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(manifest.1).unwrap();
            assert_eq!(manifest["id"], bundle.id);
            let mut names = std::collections::HashSet::new();
            for (path, _) in bundle.files {
                assert!(names.insert(*path), "duplicate asset: {path}");
                assert!(!Path::new(path).is_absolute());
                assert!(Path::new(path).components().all(|c| matches!(c, std::path::Component::Normal(_))));
                assert!(!path.contains("fixtures") && !path.starts_with("tests/"));
            }
            assert!(bundle.files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() < 1_048_576);
        }
    }

    #[test]
    fn link_lookup_requires_a_checkout_and_matching_plugin_identity() {
        let root = std::env::temp_dir().join(format!("fbt-link-source-{}", std::process::id()));
        let source = root.join("plugin/oliver.torrents");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("manifest.json"), r#"{"id":"oliver.torrents"}"#).unwrap();
        let starts = [source.clone()];
        assert!(checkout_source("oliver.torrents", &starts).is_none());
        std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"fbittorrent\"\n").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        assert_eq!(checkout_source("oliver.torrents", &starts), Some(source.canonicalize().unwrap()));
        assert!(checkout_source("oliver.torrents-quick", &starts).is_none());
        std::fs::write(source.join("manifest.json"), r#"{"id":"another.plugin"}"#).unwrap();
        assert!(checkout_source("oliver.torrents", &starts).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
