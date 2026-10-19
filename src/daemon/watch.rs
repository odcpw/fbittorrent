//! Once-per-second verified video readiness. Ownership survives engine-slot swaps.
mod duration;
#[cfg(test)]
mod tests;
use crate::api::{self, Playable};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MIB: u64 = 1024 * 1024;
pub(crate) struct Sample {
    pub hash: String,
    pub name: String,
    pub file: u64,
    pub path: PathBuf,
    pub size: u64,
    pub prefix: u64,
    pub last: bool,
    pub downloading: bool,
    pub dl_bps: Option<u64>,
}
pub(crate) fn video(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mkv" | "mp4" | "webm" | "avi" | "mov" | "m4v" | "ts"
        )
    })
}
/// A shared boundary piece contributes only bytes within this file.
pub(crate) fn largest_video<'a>(
    files: impl Iterator<Item = (usize, &'a Path, u64)>,
) -> Option<usize> {
    files
        .filter(|(_, p, n)| *n > 0 && video(p))
        .max_by(|a, b| a.2.cmp(&b.2).then_with(|| b.0.cmp(&a.0)))
        .map(|v| v.0)
}
pub(crate) fn prefix(offset: u64, size: u64, piece: u64, have: impl Fn(usize) -> bool) -> u64 {
    if piece == 0 || size == 0 {
        return 0;
    }
    let mut end = offset;
    let limit = offset.saturating_add(size);
    let mut index = offset / piece;
    while end < limit && have(index as usize) {
        end = ((index + 1).saturating_mul(piece)).min(limit);
        index += 1;
    }
    end.saturating_sub(offset)
}
pub(crate) fn ready(prefix: u64, size: u64, last: bool) -> bool {
    let percent = ((u128::from(size) * 3 + 99) / 100) as u64;
    last && prefix >= (32 * MIB).max(percent)
}
pub(crate) fn estimate_rate(size: u64, dl_bps: u64) -> bool {
    u128::from(dl_bps) * 7200 >= u128::from(size)
}
fn value(s: &Sample, seconds: Option<f64>, acked: bool) -> Playable {
    let buffered_s = seconds.map(|d| ((s.prefix as f64 / s.size.max(1) as f64) * d).floor() as u64);
    let finishes_in_s = s
        .dl_bps
        .filter(|v| *v > 0)
        .map(|v| s.size.saturating_sub(s.prefix).div_ceil(v));
    let stall_free = seconds
        .zip(s.dl_bps.filter(|v| *v > 0))
        .filter(|_| s.downloading)
        .map(|(d, rate)| {
            s.prefix as f64 * d / s.size.max(1) as f64
                >= s.size.saturating_sub(s.prefix) as f64 / rate as f64
        });
    Playable {
        file: s.file,
        ready: ready(s.prefix, s.size, s.last),
        prefix_bytes: s.prefix,
        acked,
        buffered_s,
        finishes_in_s,
        stall_free,
    }
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Saved {
    ready: bool,
    acked: bool,
}
struct Row {
    value: Playable,
    duration: Option<f64>,
    inspected: u64,
    path: PathBuf,
    size: u64,
}
pub(crate) struct Watch {
    path: PathBuf,
    saved: BTreeMap<String, Saved>,
    rows: BTreeMap<String, Row>,
    next: Instant,
    fired: BTreeSet<String>,
}
impl Watch {
    pub fn open(path: PathBuf) -> Self {
        let load = || -> anyhow::Result<BTreeMap<String, Saved>> {
            let mut bytes = Vec::new();
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path)?
                .take(2 * MIB + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() <= 2 * MIB as usize,
                "watch history exceeds 2 MiB"
            );
            let saved: BTreeMap<String, Saved> = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                saved.len() <= 32768 && saved.keys().all(|s| api::Hash::new(s).is_ok()),
                "invalid watch history"
            );
            Ok(saved)
        };
        let saved = if path.exists() {
            load().unwrap_or_else(|e| {
                eprintln!("WARN watch history unavailable: {e}");
                BTreeMap::new()
            })
        } else {
            BTreeMap::new()
        };
        Self {
            path,
            saved,
            rows: BTreeMap::new(),
            next: Instant::now(),
            fired: BTreeSet::new(),
        }
    }
    pub fn due(&self) -> bool {
        Instant::now() >= self.next
    }
    fn save(&self) {
        let result = || -> anyhow::Result<()> {
            let parent = self.path.parent().unwrap();
            std::fs::create_dir_all(parent)?;
            let tmp = self
                .path
                .with_extension(format!("{}.tmp", std::process::id()));
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&tmp)?;
            serde_json::to_writer(&mut f, &self.saved)?;
            f.flush()?;
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        };
        if let Err(e) = result() {
            eprintln!("WARN saving watch history: {e}");
        }
    }
    pub fn sample(&mut self, samples: Vec<Sample>) -> Vec<Value> {
        self.next = Instant::now() + Duration::from_secs(1);
        let mut dirty = false;
        let mut events = Vec::new();
        for s in samples {
            if !self.saved.contains_key(&s.hash) && self.saved.len() >= 32768 {
                continue;
            }
            let cached = self
                .rows
                .get(&s.hash)
                .filter(|r| r.path == s.path && r.size == s.size);
            let inspected = s.prefix.min(MIB);
            let seconds = if let Some(cached) =
                cached.filter(|r| r.duration.is_some() || r.inspected >= inspected)
            {
                cached.duration
            } else if inspected > 0 {
                duration::read(&s.path, inspected)
            } else {
                None
            };
            let previous = self.saved.get(&s.hash).cloned().unwrap_or_default();
            let mut p = value(&s, seconds, previous.acked);
            if !p.ready {
                p.acked = false;
            }
            let saved = Saved {
                ready: p.ready,
                acked: p.acked,
            };
            if saved != previous {
                self.saved.insert(s.hash.clone(), saved);
                dirty = true;
            }
            if p.ready && !previous.ready && self.fired.insert(s.hash.clone()) {
                let mut event = json!({"ev":"torrent.playable","hash":s.hash,"name":s.name,"file":s.file,"open_target":s.path.to_string_lossy()});
                if let Some(seconds) = p.buffered_s {
                    event["buffered_s"] = json!(seconds);
                }
                events.push(event);
            }
            self.rows.insert(
                s.hash,
                Row {
                    value: p,
                    duration: seconds,
                    inspected,
                    path: s.path,
                    size: s.size,
                },
            );
        }
        if dirty {
            self.save();
        }
        events
    }
    pub fn decorate(&mut self, rows: &mut [api::Torrent]) {
        let mut dirty = false;
        for row in rows {
            if row.staged
                || matches!(
                    row.state,
                    api::TorrentState::Checking
                        | api::TorrentState::Metadata
                        | api::TorrentState::Error
                )
            {
                continue;
            }
            if let Some(p) = self.rows.get_mut(row.hash.as_str()) {
                if row.completed_at.is_some() && !p.value.acked {
                    p.value.acked = true;
                    if let Some(s) = self.saved.get_mut(row.hash.as_str()) {
                        s.acked = true;
                        dirty = true;
                    }
                }
                let mut value = p.value.clone();
                if row.state != api::TorrentState::Downloading {
                    value.stall_free = None;
                }
                value.finishes_in_s = row
                    .dl_bps
                    .filter(|v| *v > 0)
                    .map(|rate| p.size.saturating_sub(value.prefix_bytes).div_ceil(rate));
                if row.completed_at.is_some() {
                    value.prefix_bytes = p.size;
                    value.buffered_s = p.duration.map(|s| s.floor() as u64);
                    value.finishes_in_s = row.dl_bps.filter(|v| *v > 0).map(|_| 0);
                }
                row.playable = Some(value);
            }
        }
        if dirty {
            self.save();
        }
    }
    pub fn target(&self, hash: &str) -> Option<PathBuf> {
        self.rows
            .get(hash)
            .filter(|r| r.value.ready)
            .map(|r| r.path.clone())
    }
    pub fn file_ready(&self, hash: &str, index: u64) -> Option<bool> {
        self.rows
            .get(hash)
            .filter(|r| r.value.file == index)
            .map(|r| r.value.ready)
    }
    pub fn acknowledge(&mut self, hash: Option<&str>) {
        let mut dirty = false;
        for (key, saved) in &mut self.saved {
            if hash.is_none_or(|h| h == key) && saved.ready && !saved.acked {
                saved.acked = true;
                dirty = true;
                if let Some(row) = self.rows.get_mut(key) {
                    row.value.acked = true;
                }
            }
        }
        if dirty {
            self.save();
        }
    }
}
