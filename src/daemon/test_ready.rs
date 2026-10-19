//! Private integration barrier: install subscriptions before READY and restore.
use super::Daemon;
use anyhow::{ensure, Result};
use std::{
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
pub(super) fn wait(daemon: &Daemon, directory: &Path) -> Result<()> {
    let Some(value) = std::env::var_os("FBT_TEST_READY_GATE") else {
        return Ok(());
    };
    let path = PathBuf::from(value);
    ensure!(
        path.starts_with(directory)
            && path.is_absolute()
            && path
                .components()
                .all(|part| !matches!(part, Component::ParentDir)),
        "test readiness gate must be inside the private runtime directory"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    eprintln!("DEBUG daemon waiting for private readiness gate");
    while !path.is_file() {
        ensure!(
            Instant::now() < deadline,
            "private readiness gate timed out"
        );
        ensure!(
            daemon.lifecycle.wait(Duration::from_millis(5)).is_none(),
            "stopped before private readiness gate"
        );
    }
    Ok(())
}
