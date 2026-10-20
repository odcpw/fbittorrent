//! Actual network counters, accumulated across engine epochs; only three durable totals.
use crate::api::{AllTimeTotals, SessionTotals};
use anyhow::{Context, Result};
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    sync::atomic::AtomicBool,
};

pub(crate) fn path() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| crate::config::data_dir())
        .join("fbittorrent/stats.json")
}
pub(crate) fn read(path: &Path) -> Result<AllTimeTotals> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("reading statistics totals"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AllTimeTotals::default()),
        Err(error) => Err(error).context("reading statistics totals"),
    }
}
/// The last fully synced checkpoint stays valid when a stop interrupts the
/// next temporary-file sync. A cancelled sync never reaches rename.
pub(crate) fn checkpoint_deferred(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error|
        matches!(error.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::TimedOut))
}
pub(crate) struct Stats {
    pub session: SessionTotals,
    pub all_time: AllTimeTotals,
    previous: Option<(u64, (u64, u64, u64))>,
    path: PathBuf,
    dirty: bool,
    saved: Instant,
}
impl Stats {
    pub fn open(path: PathBuf, now: Instant) -> Result<Self> {
        Ok(Self {
            all_time: read(&path)?,
            session: SessionTotals {
                downloaded: 0,
                uploaded: 0,
                started_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            },
            previous: None,
            path,
            dirty: false,
            saved: now,
        })
    }
    pub fn observe(&mut self, epoch: u64, counters: (u64, u64, u64)) {
        let before = self
            .previous
            .filter(|(old, _)| *old == epoch)
            .map_or((0, 0, 0), |(_, v)| v);
        let delta = (
            counters.0.saturating_sub(before.0),
            counters.1.saturating_sub(before.1),
            counters.2.saturating_sub(before.2),
        );
        self.previous = Some((epoch, counters));
        self.session.downloaded = self.session.downloaded.saturating_add(delta.0);
        self.session.uploaded = self.session.uploaded.saturating_add(delta.1);
        self.all_time.downloaded = self.all_time.downloaded.saturating_add(delta.0);
        self.all_time.uploaded = self.all_time.uploaded.saturating_add(delta.1);
        self.all_time.completed = self.all_time.completed.saturating_add(delta.2);
        self.dirty |= delta != (0, 0, 0);
    }
    pub fn save(&mut self, now: Instant, drain: bool) -> Result<bool> {
        self.save_cancellable(now, drain, None)
    }
    pub fn save_cancellable(&mut self, now: Instant, drain: bool, stop: Option<&AtomicBool>) -> Result<bool> {
        if !self.dirty
            || (!drain && now.saturating_duration_since(self.saved) < Duration::from_secs(30))
        {
            return Ok(false);
        }
        let parent = self.path.parent().context("statistics directory")?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temporary = parent.join(format!(".stats-{}-{suffix}.tmp", std::process::id()));
        let deadline = drain.then(|| Instant::now() + Duration::from_secs(1));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            serde_json::to_writer(&mut file, &self.all_time)?;
            file.write_all(b"\n")?;
            crate::durable_sync::sync_all_until(&file, stop, deadline)?;
            fs::rename(&temporary, &self.path)?;
            crate::durable_sync::sync_all_until(&File::open(parent)?, stop, deadline)?;
            Ok(())
        })();
        if result.as_ref().err().is_some_and(|error| !checkpoint_deferred(error)) {
            let _ = fs::remove_file(&temporary);
        }
        result.context("saving statistics totals")?;
        self.dirty = false;
        self.saved = now;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn statistics_survive_ring_setup_failure() {
        crate::durable_sync::fault::isolated(&[libc::ENOMEM, libc::EPERM], |errno| {
            crate::durable_sync::fault::deny_setup(errno);
            save_throttle_30s();
        });
    }
    #[test]
    fn save_throttle_30s() {
        let root = std::env::temp_dir().join(format!(
            "fbt-stats-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("stats.json");
        let start = Instant::now();
        let mut stats = Stats::open(path.clone(), start).unwrap();
        let mut saves = Vec::new();
        for second in 0..=61 {
            stats.observe(1, (second * 100, second * 20, second / 10));
            if stats
                .save(start + Duration::from_secs(second), false)
                .unwrap()
            {
                saves.push(second);
            }
        }
        assert_eq!(saves, vec![30, 60]);
        assert!(stats.save(start + Duration::from_secs(61), true).unwrap());
        saves.push(61);
        println!("observed statistics save timestamps (seconds): {saves:?}");
        let bytes = fs::read(&path).unwrap();
        let totals = read(&path).unwrap();
        assert_eq!(totals.downloaded, 6100);
        assert_eq!(totals.uploaded, 1220);
        assert_eq!(totals.completed, 6);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut restored = Stats::open(path.clone(), start).unwrap();
        assert_eq!(restored.all_time, totals);
        assert_eq!(restored.session.downloaded, 0);
        restored.observe(2, (10, 5, 1));
        restored.observe(2, (10, 5, 1));
        restored.observe(3, (20, 7, 1));
        assert_eq!(restored.session.downloaded, 30);
        assert_eq!(restored.all_time.completed, 8);
        let stopping = AtomicBool::new(true);
        let cancelled = restored.save_cancellable(start + Duration::from_secs(62), false, Some(&stopping)).unwrap_err();
        assert!(checkpoint_deferred(&cancelled));
        assert_eq!(fs::read(&path).unwrap(), bytes, "cancelled checkpoint cannot replace acknowledged totals");
        // Failed atomic replacement leaves the previous file byte-for-byte intact.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(
            restored
                .save(start + Duration::from_secs(62), true)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
