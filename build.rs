use std::{env, process::Command};
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn main() {
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
