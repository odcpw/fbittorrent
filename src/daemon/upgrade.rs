//! Executable replacement is an owner transition, never an engine restart.
use anyhow::{Context, Result};
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf, time::{Duration, Instant}};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity { dev: u64, inode: u64, seconds: i64, nanos: i64 }
fn identity(path: &std::path::Path) -> Result<Identity> {
    let meta = fs::metadata(path)?;
    Ok(Identity { dev: meta.dev(), inode: meta.ino(), seconds: meta.mtime(), nanos: meta.mtime_nsec() })
}
pub(super) struct Upgrade {
    path: PathBuf,
    original: Identity,
    next: Instant,
    interval: Duration,
    changed: bool,
}
impl Upgrade {
    pub fn new() -> Result<Self> {
        let path = std::env::current_exe().context("locating daemon executable")?;
        let interval = std::env::var("FBT_TEST_UPGRADE_CHECK_MS").ok().and_then(|v| v.parse::<u64>().ok())
            .map(|v| Duration::from_millis(v.max(50))).unwrap_or(Duration::from_secs(30));
        let original = identity(&path).context("recording daemon executable identity")?;
        Ok(Self { path, original, next: Instant::now() + interval, interval, changed: false })
    }
    pub fn wait(&self, now: Instant) -> Duration { self.next.saturating_duration_since(now) }
    /// Once observed, replacement stays pending until the busy table permits it.
    pub fn check(&mut self, now: Instant, busy: bool) -> bool {
        if now >= self.next {
            self.next = now + self.interval;
            match identity(&self.path) {
                Ok(current) => self.changed |= current != self.original,
                Err(error) => eprintln!("WARN daemon upgrade check failed error={:?}", error.to_string()),
            }
        }
        self.changed && !busy
    }
    pub fn pending(&self) -> bool { self.changed }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_replacement_waits_for_busy_table() {
        let root = std::env::temp_dir().join(format!("fbt-upgrade-{}",std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("binary"); fs::write(&path,b"old").unwrap();
        let at = Instant::now();
        let mut upgrade = Upgrade { original: identity(&path).unwrap(), path: path.clone(),
            interval: Duration::from_secs(30), next: at + Duration::from_secs(30), changed: false };
        fs::write(root.join("next"),b"new").unwrap(); fs::rename(root.join("next"),&path).unwrap();
        assert!(!upgrade.check(at,false)); assert!(!upgrade.pending());
        assert!(!upgrade.check(at+Duration::from_secs(30),true)); assert!(upgrade.pending());
        assert!(upgrade.check(at+Duration::from_secs(31),false));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unchanged_executable_never_requests_upgrade() {
        let mut upgrade = Upgrade::new().unwrap();
        assert!(!upgrade.check(upgrade.next,false)); assert!(!upgrade.pending());
    }
}
