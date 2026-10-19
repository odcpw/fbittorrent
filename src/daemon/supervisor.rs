//! Disk policy shared by the owner supervisor and the engine state observer.
use crate::{
    api::{Disk, DiskSpace},
    engine::view::{Intent, TorrentState, TorrentView},
};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::Path,
    time::{Duration, Instant},
};
const MIB: u64 = 1024 * 1024;
const WARNING_MARGIN: u64 = 512 * MIB;
const RETRY_RISE: u64 = 16 * MIB;

#[derive(Clone, Copy)]
struct Space {
    free: u64,
    fs: u64,
}
fn space(path: &Path) -> Option<Space> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut out = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL-terminated and out points to a full writable statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), out.as_mut_ptr()) } != 0 {
        return None;
    }
    let out = unsafe { out.assume_init() };
    if out.f_frsize == 0 { return None; }
    let blocks = out.f_bavail as u64;
    if blocks > i64::MAX as u64 {
        return None;
    }
    Some(Space {
        free: blocks.checked_mul(out.f_frsize as u64)?,
        fs: out.f_fsid as u64,
    })
}
fn active(intent: Intent, state: TorrentState) -> bool {
    intent == Intent::Run
        && !matches!(
            state,
            TorrentState::Paused
                | TorrentState::Done
                | TorrentState::Seeding
                | TorrentState::WaitingVpn
        )
}
fn needed_bytes(rows: impl Iterator<Item = (Option<u64>, Option<u64>)>) -> u64 {
    rows.filter_map(|(size, completed)| {
        size.map(|size| size.saturating_sub(completed.unwrap_or(0)))
    })
    .fold(0, u64::saturating_add)
}
struct Baseline {
    free: u64,
    rise: u64,
}
pub(crate) struct DiskMonitor {
    pub summary: Disk,
    baselines: HashMap<String, Baseline>,
    last_sample: Option<Instant>,
    notified: bool,
}
impl Default for DiskMonitor {
    fn default() -> Self {
        Self {
            summary: Disk {
                needed_bytes: 0,
                low: false,
                free_bytes: None,
            },
            baselines: HashMap::new(),
            last_sample: None,
            notified: false,
        }
    }
}
impl DiskMonitor {
    /// The single engine observer calls this on state samples. Only a new
    /// ENOSPC performs a syscall; ordinary samples never stat the filesystem.
    pub fn observe(&mut self, rows: &[TorrentView]) {
        self.baselines.retain(|hash, _| {
            rows.iter().any(|r| {
                &r.hash == hash
                    && r.intent == Intent::Run
                    && r.reason.as_deref() == Some("disk_full")
            })
        });
        for row in rows.iter().filter(|r| {
            r.intent == Intent::Run
                && r.state == TorrentState::Error
                && r.reason.as_deref() == Some("disk_full")
        }) {
            if self.baselines.contains_key(&row.hash) {
                continue;
            }
            if let Some(space) = space(Path::new(&row.save_path)) {
                let rise = RETRY_RISE.max(row.piece_length.unwrap_or(0));
                self.baselines.insert(
                    row.hash.clone(),
                    Baseline {
                        free: space.free,
                        rise,
                    },
                );
                eprintln!(
                    "WARN engine disk_full hash={} free_bytes={} retry_rise_bytes={rise}",
                    row.hash, space.free
                );
            }
        }
    }
    pub fn restarted(&mut self, hash: &str) {
        self.baselines.remove(hash);
    }
    pub fn sample(&mut self, rows: &[TorrentView], now: Instant) -> (Vec<(String, u64)>, bool) {
        if self
            .last_sample
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(5))
        {
            return (Vec::new(), false);
        }
        self.last_sample = Some(now);
        let mut paths = BTreeMap::new();
        for row in rows.iter().filter(|r| active(r.intent, r.state)) {
            paths.entry(row.save_path.clone()).or_insert(None);
        }
        for (path, value) in &mut paths {
            *value = space(Path::new(path));
        }
        let needed = needed_bytes(
            rows.iter()
                .filter(|r| active(r.intent, r.state))
                .map(|r| (r.size, r.completed_bytes)),
        );
        let mut filesystems = BTreeMap::<u64, (u64, u64)>::new();
        let mut known = true;
        for (path, space) in &paths {
            let Some(space) = space else {
                known = false;
                continue;
            };
            let required = needed_bytes(
                rows.iter()
                    .filter(|r| active(r.intent, r.state) && &r.save_path == path)
                    .map(|r| (r.size, r.completed_bytes)),
            );
            let entry = filesystems.entry(space.fs).or_insert((space.free, 0));
            entry.0 = entry.0.min(space.free);
            entry.1 = entry.1.saturating_add(required);
        }
        let low = filesystems
            .values()
            .any(|(free, needed)| *free < needed.saturating_add(WARNING_MARGIN));
        self.summary = Disk {
            needed_bytes: needed,
            low,
            free_bytes: (known && !paths.is_empty()).then(|| {
                filesystems
                    .values()
                    .map(|v| v.0)
                    .fold(0, u64::saturating_add)
            }),
        };
        let notify = low && !self.notified;
        self.notified |= low;
        let retries = rows
            .iter()
            .filter_map(|row| {
                if row.intent != Intent::Run || row.reason.as_deref() != Some("disk_full") {
                    return None;
                }
                let current = paths.get(&row.save_path)?.as_ref()?.free;
                let baseline = self.baselines.get(&row.hash)?;
                (current >= baseline.free.saturating_add(baseline.rise))
                    .then(|| (row.hash.clone(), current))
            })
            .collect();
        (retries, notify)
    }
}
pub(crate) fn add_warning(
    path: &Path,
    bytes: &[u8],
    selected: Option<&[usize]>,
) -> Option<DiskSpace> {
    let torrent = librqbit::torrent_from_bytes(bytes).ok()?;
    let info = torrent.info.data.validate().ok()?;
    let needed = info
        .iter_file_details()
        .enumerate()
        .filter(|(i, _)| selected.is_none_or(|s| s.contains(i)))
        .map(|(_, f)| f.len)
        .fold(0, u64::saturating_add);
    let free = space(path)?.free;
    (needed > free).then_some(DiskSpace {
        free_bytes: free,
        needed_bytes: needed,
    })
}
pub(crate) fn warning_message(disk: &DiskSpace) -> String {
    fn size(n: u64) -> String {
        if n >= 1024 * MIB {
            format!("{:.1} GiB", n as f64 / (1024 * MIB) as f64)
        } else {
            format!("{:.1} MiB", n as f64 / MIB as f64)
        }
    }
    format!(
        "Needs {} · {} free",
        size(disk.needed_bytes),
        size(disk.free_bytes)
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn needed_bytes_sum() {
        assert_eq!(
            needed_bytes(
                [
                    (Some(100), Some(30)),
                    (Some(10), Some(20)),
                    (None, None),
                    (Some(40), None)
                ]
                .into_iter()
            ),
            110
        );
        assert!(!active(Intent::Pause, TorrentState::Downloading));
        assert!(!active(Intent::Run, TorrentState::Done));
        assert!(active(Intent::Run, TorrentState::Error));
        assert!(active(Intent::Run, TorrentState::Checking));
        assert_eq!(
            needed_bytes([(Some(u64::MAX), Some(0)), (Some(1), Some(0))].into_iter()),
            u64::MAX
        );
    }
    #[test]
    fn recovery_is_per_free_space_rise_not_warning_margin() {
        let b = Baseline {
            free: 1024,
            rise: RETRY_RISE.max(32 * MIB),
        };
        assert!(1024 + 31 * MIB < b.free + b.rise);
        assert_eq!(1024 + 32 * MIB, b.free + b.rise);
        let mut policy = DiskMonitor::default();
        policy.baselines.insert("fixture".into(), b);
        policy.restarted("fixture");
        assert!(policy.baselines.is_empty());
        assert!(RETRY_RISE < 64 * MIB && WARNING_MARGIN > 64 * MIB);
    }
    #[test]
    fn failed_stat_is_unknown_and_binary_units_are_used() {
        assert!(space(Path::new("/not/a/fbittorrent/fixture/path")).is_none());
        assert_eq!(
            warning_message(&DiskSpace {
                free_bytes: 64 * MIB,
                needed_bytes: 100 * MIB
            }),
            "Needs 100.0 MiB · 64.0 MiB free"
        );
    }
}
