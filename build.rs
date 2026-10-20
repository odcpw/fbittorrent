use std::{env, fmt::Write, path::Path, process::Command};

fn plugin_files(root: &Path, directory: &Path, output: &mut String) {
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut entries = std::fs::read_dir(directory).expect("reading plugin assets")
        .collect::<std::io::Result<Vec<_>>>().expect("listing plugin assets");
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().expect("plugin asset type");
        assert!(!kind.is_symlink(), "plugin asset must not be a symlink: {}", path.display());
        if kind.is_dir() {
            plugin_files(root, &path, output);
        } else if kind.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            let relative = path.strip_prefix(root).unwrap().to_str().expect("UTF-8 plugin path");
            writeln!(output, "({relative:?}, include_bytes!({:?})),", path.to_str().unwrap()).unwrap();
        }
    }
}

fn embed_plugins() {
    let manifest = std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut output = String::new();
    for (name, id) in [("TORRENTS", "oliver.torrents"), ("QUICK", "oliver.torrents-quick")] {
        writeln!(output, "pub(super) static {name}: Bundle = Bundle {{ id: {id:?}, files: &[").unwrap();
        let root = manifest.join("plugin").join(id);
        plugin_files(&root, &root, &mut output);
        output.push_str("] };\n");
    }
    std::fs::write(std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("plugins.rs"), output)
        .expect("writing embedded plugin table");
}
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn main() {
    // These two production trees contain only the shipping QML/JS/assets;
    // plugin/tests and its fixtures are deliberately outside the bundles.
    embed_plugins();
    // Pack PIE's relative relocation offsets instead of faulting a large RELA
    // table at startup. The Omarchy GNU loader supports RELR; PIE and RELRO stay.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        println!("cargo:rustc-link-arg-bin=fbittorrent=-Wl,-z,pack-relative-relocs");
    }
    println!("cargo:rerun-if-env-changed=FBT_BUILD_ID_OVERRIDE");
    for path in ["src", "vendor", "Cargo.toml", "Cargo.lock", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &branch]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let build = env::var("FBT_BUILD_ID_OVERRIDE").ok().filter(|value|
        !value.is_empty() && value.len() <= 80 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)))
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]).map(|hash| {
            let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
            format!("{hash}{}", if dirty { "-dirty" } else { "" })
        })).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=FBT_BUILD_ID={build}");
}
