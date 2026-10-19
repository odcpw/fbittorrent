//! Followed TV shows: a local list that is refreshed from EZTV when the app opens.
//!
//! No service, no timer. `Shows` lives in `~/.local/share/fbittorrent/shows.json` and
//! carries, per show, the episodes found at the last check plus what the user has
//! grabbed or marked seen. The TUI refreshes stale shows on launch; the CLI does it
//! on `shows check`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::imdb::Title;

/// A show is "stale" and refreshed on launch after this long.
pub const STALE_SECS: i64 = 30 * 60;
/// A malformed source must not make us fetch forever. Reaching this guard is an
/// error, rather than silently treating a partial catalogue as the complete list.
const MAX_PAGES: usize = 32;
const CHECK_BUDGET: Duration = Duration::from_secs(30);
static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
pub const QUALITIES: [&str; 4] = ["2160p", "1080p", "720p", "480p"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Episode {
    /// "S05E16", or "S05" for a complete season release.
    pub key: String,
    pub season: u16,
    pub episode: u16,
    /// Release name of the chosen torrent.
    pub title: String,
    pub magnet: String,
    pub seeds: i64,
    pub size: i64,
    /// Unix seconds, 0 = unknown.
    pub released: i64,
    /// "1080p" etc. when the release name says so.
    pub quality: String,
    /// How many torrents the source had for this episode.
    pub candidates: usize,
}

impl Episode {
    pub fn is_season_pack(&self) -> bool {
        self.season > 0 && self.episode == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Show {
    /// "tt0903747"
    pub imdb: String,
    pub title: String,
    pub year: Option<u16>,
    pub added: i64,
    /// Preferred quality token for this show; None = the config default.
    pub quality: Option<String>,
    /// Release keys the user has read/dismissed in the feed. This is independent
    /// of whether a release was downloaded or watched.
    pub seen: BTreeSet<String>,
    /// Episode key -> magnet we added.
    pub grabbed: BTreeMap<String, String>,
    pub last_checked: i64,
    /// Whether a successful check reached the end of the source catalogue.
    /// Incomplete first checks remain visible instead of implying a full history.
    pub catalogue_complete: bool,
    pub episodes: Vec<Episode>,
    pub error: Option<String>,
}

impl Show {
    pub fn from_title(t: &Title, now: i64) -> Self {
        Show {
            imdb: t.id.clone(),
            title: t.name.clone(),
            year: t.year,
            added: now,
            ..Default::default()
        }
    }
    pub fn label(&self) -> String {
        match self.year {
            Some(y) => format!("{} ({y})", self.title),
            None => self.title.clone(),
        }
    }
    pub fn is_new(&self, key: &str) -> bool {
        !self.seen.contains(key)
    }
    pub fn new_count(&self) -> usize {
        self.episodes.iter().filter(|e| self.is_new(&e.key)).count()
    }
    pub fn is_stale(&self, now: i64) -> bool {
        now - self.last_checked > STALE_SECS
    }
    pub fn episode(&self, key: &str) -> Option<&Episode> {
        let k = key.to_ascii_uppercase();
        self.episodes.iter().find(|e| e.key == k)
    }
    /// Toggle feed read state; returns true when read. Download history is kept.
    #[cfg(test)]
    pub fn toggle_seen(&mut self, key: &str) -> bool {
        if self.seen.remove(key) {
            false
        } else {
            self.seen.insert(key.to_string());
            true
        }
    }
    pub fn mark_all_seen(&mut self) -> usize {
        let mut n = 0;
        for e in &self.episodes {
            if self.is_new(&e.key) {
                n += 1;
            }
        }
        for e in &self.episodes {
            self.seen.insert(e.key.clone());
        }
        n
    }
    pub fn mark_grabbed(&mut self, key: &str, magnet: &str) {
        self.grabbed.insert(key.to_string(), magnet.to_string());
        self.seen.insert(key.to_string());
    }
    /// Keep every discovered release, including old unread seasons that have
    /// disappeared from the source. A better/newer candidate updates its row
    /// without changing the user's per-key read or download history.
    pub fn merge_episodes(&mut self, episodes: Vec<Episode>) -> usize {
        let mut by_key: BTreeMap<String, Episode> = self
            .episodes
            .drain(..)
            .map(|e| (e.key.clone(), e))
            .collect();
        let mut fresh = 0;
        for e in episodes {
            if !by_key.contains_key(&e.key) {
                fresh += 1;
            }
            by_key.insert(e.key.clone(), e);
        }
        self.episodes = by_key.into_values().collect();
        self.episodes
            .sort_by(|a, b| (b.season, b.episode).cmp(&(a.season, a.episode)));
        fresh
    }
    pub fn apply_check(&mut self, check: EpisodeCheck) -> usize {
        let fresh = self.merge_episodes(check.episodes);
        self.catalogue_complete = check.catalogue_complete;
        self.last_checked = now();
        self.error = check.warning;
        fresh
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Shows {
    pub shows: Vec<Show>,
    #[serde(skip)]
    baseline: RefCell<Option<Vec<Show>>>,
}

pub fn path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("fbittorrent")
        .join("shows.json")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Shows {
    pub fn load() -> Result<Self> {
        Self::load_from(&path())
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&path())
    }

    pub fn load_from(p: &std::path::Path) -> Result<Self> {
        if !p.exists() {
            return Ok(Self {
                baseline: RefCell::new(Some(Vec::new())),
                ..Default::default()
            });
        }
        let raw = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let all: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}; existing history was preserved", p.display()))?;
        *all.baseline.borrow_mut() = Some(all.shows.clone());
        Ok(all)
    }

    /// Atomic write with a unique temporary file, so readers never see partial
    /// JSON and simultaneous writers cannot rename one another's temp file.
    pub fn save_to(&self, p: &std::path::Path) -> Result<()> {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _lock = store_lock(p)?;
        let output = match self.baseline.borrow().as_ref() {
            Some(before) if p.exists() => {
                let mut latest = Self::load_from(p)?;
                latest.merge_changes(before, &self.shows);
                latest
            }
            _ => self.clone(),
        };
        let tmp = p.with_extension(format!(
            "json.tmp.{}.{}",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| -> Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            f.write_all(serde_json::to_string_pretty(&output)?.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, p).with_context(|| format!("writing {}", p.display()))?;
            if let Some(dir) = p.parent() {
                std::fs::File::open(dir)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        if result.is_ok() {
            *self.baseline.borrow_mut() = Some(self.shows.clone());
        }
        result
    }

    /// Apply only this caller's changes. Another CLI invocation may have changed
    /// a different read flag while the TUI was open; saving must retain that edit.
    fn merge_changes(&mut self, before: &[Show], desired: &[Show]) {
        for old in before {
            if !desired.iter().any(|s| s.imdb == old.imdb) {
                self.unfollow(&old.imdb);
            }
        }
        for wanted in desired {
            let old = before.iter().find(|s| s.imdb == wanted.imdb);
            let Some(current) = self.get_mut(&wanted.imdb) else {
                // Respect an unfollow performed by another process. Only a
                // newly followed show is allowed to create a row here.
                if old.is_none() {
                    self.shows.push(wanted.clone());
                }
                continue;
            };
            let empty = Show::default();
            let old = old.unwrap_or(&empty);
            if wanted.title != old.title {
                current.title = wanted.title.clone();
            }
            if wanted.year != old.year {
                current.year = wanted.year;
            }
            if wanted.added != old.added {
                current.added = wanted.added;
            }
            if wanted.quality != old.quality {
                current.quality = wanted.quality.clone();
            }
            for key in wanted.seen.difference(&old.seen) {
                current.seen.insert(key.clone());
            }
            for key in old.seen.difference(&wanted.seen) {
                current.seen.remove(key);
            }
            for (key, magnet) in &wanted.grabbed {
                if old.grabbed.get(key) != Some(magnet) {
                    current.grabbed.insert(key.clone(), magnet.clone());
                }
            }
            for key in old
                .grabbed
                .keys()
                .filter(|k| !wanted.grabbed.contains_key(*k))
            {
                current.grabbed.remove(key);
            }
            if wanted.episodes != old.episodes {
                current.merge_episodes(wanted.episodes.clone());
            }
            if wanted.last_checked != old.last_checked {
                current.last_checked = current.last_checked.max(wanted.last_checked);
            }
            if wanted.catalogue_complete != old.catalogue_complete {
                current.catalogue_complete = wanted.catalogue_complete;
            }
            if wanted.error != old.error {
                current.error = wanted.error.clone();
            }
        }
        self.shows.sort_by_key(|s| s.title.to_lowercase());
    }

    pub fn get(&self, imdb: &str) -> Option<&Show> {
        self.shows.iter().find(|s| s.imdb == imdb)
    }
    pub fn get_mut(&mut self, imdb: &str) -> Option<&mut Show> {
        self.shows.iter_mut().find(|s| s.imdb == imdb)
    }
    pub fn set_quality(&mut self, imdb: &str, quality: Option<&str>) -> Result<()> {
        anyhow::ensure!(quality.is_none_or(|q|QUALITIES.contains(&q)),"quality must be 2160p, 1080p, 720p, 480p or default");
        self.get_mut(imdb).context("show not followed")?.quality=quality.map(str::to_owned);
        Ok(())
    }

    /// Add a show; false when it is already followed.
    pub fn follow(&mut self, t: &Title) -> bool {
        if self.get(&t.id).is_some() {
            return false;
        }
        self.shows.push(Show::from_title(t, now()));
        self.shows.sort_by_key(|s| s.title.to_lowercase());
        true
    }

    pub fn unfollow(&mut self, imdb: &str) -> Option<Show> {
        let i = self.shows.iter().position(|s| s.imdb == imdb)?;
        Some(self.shows.remove(i))
    }

    pub fn new_total(&self) -> usize {
        self.shows.iter().map(|s| s.new_count()).sum()
    }

}

fn store_lock(p: &std::path::Path) -> Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(p.with_extension("json.lock"))?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    let start = Instant::now();
    loop {
        // flock is advisory and the file descriptor releases it on every exit.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(lock);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock
            || start.elapsed() >= Duration::from_secs(2)
        {
            return Err(error).context("locking show history; no changes were written");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------- EZTV

#[derive(Deserialize)]
struct EztvResp {
    #[serde(default)]
    torrents: Vec<EztvItem>,
    #[serde(default)]
    torrents_count: i64,
}
#[derive(Deserialize)]
struct EztvItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    magnet_url: String,
    #[serde(default)]
    season: serde_json::Value,
    #[serde(default)]
    episode: serde_json::Value,
    #[serde(default)]
    seeds: i64,
    #[serde(default)]
    size_bytes: serde_json::Value,
    #[serde(default)]
    date_released_unix: i64,
}

fn as_i64(v: &serde_json::Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0)
}

pub fn quality_of(name: &str) -> String {
    let low = name.to_ascii_lowercase();
    QUALITIES
        .iter()
        .find(|q| low.contains(*q))
        .map(|q| q.to_string())
        .unwrap_or_default()
}

#[derive(Debug)]
pub struct EpisodeCheck {
    pub episodes: Vec<Episode>,
    pub catalogue_complete: bool,
    pub warning: Option<String>,
}

pub(crate) fn fetch_updates_cancelled(show:&Show, quality:&str, cancelled:&std::sync::atomic::AtomicBool)->Result<EpisodeCheck> {
    let boundary=show.episodes.iter().map(|e|e.released).filter(|&v|v>0).max();
    fetch_eztv_inner(&show.imdb,show.quality.as_deref().unwrap_or(quality),boundary,show.catalogue_complete,Some(cancelled))
}
pub(crate) fn eztv_endpoint(query:&str)->String {
    let base=std::env::var("FBT_EZTV_BASE_URL").unwrap_or_else(|_|"https://eztv.re".into());
    format!("{}/api/get-torrents?{query}",base.trim_end_matches('/'))
}
fn fetch_eztv_inner(imdb:&str,quality:&str,boundary:Option<i64>,complete_before:bool,cancelled:Option<&std::sync::atomic::AtomicBool>)->Result<EpisodeCheck> {
    let digits = imdb.trim_start_matches("tt");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent("fbittorrent/0.1 (show feed)")
        .build()
        .into();
    fetch_pages(quality, boundary, complete_before, |page, remaining| {
        anyhow::ensure!(!cancelled.is_some_and(|c|c.load(Ordering::Acquire)),"show check cancelled");
        let url = eztv_endpoint(&format!("imdb_id={digits}&limit=100&page={page}"));
        let mut response = agent
            .get(&url)
            .config()
            .timeout_global(Some(remaining.min(Duration::from_secs(10))))
            .build()
            .call()
            .with_context(|| format!("checking EZTV page {page}"))?;
        let body = response
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_to_string()?;
        serde_json::from_str(&body).context("parsing EZTV releases")
    })
}

fn fetch_pages<F>(
    quality: &str,
    boundary: Option<i64>,
    complete_before: bool,
    mut fetch: F,
) -> Result<EpisodeCheck>
where
    F: FnMut(usize, Duration) -> Result<EztvResp>,
{
    let started = Instant::now();
    let mut items = Vec::new();
    let mut complete = complete_before;
    let mut overlap_after = None;
    let mut source_ordered = true;
    let mut previous_date = i64::MAX;
    for page in 1..=MAX_PAGES {
        let Some(remaining) = CHECK_BUDGET
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
        else {
            complete = false;
            break;
        };
        let r = fetch(page, remaining)?;
        let n = r.torrents.len();
        for item in &r.torrents {
            if item.date_released_unix <= 0 || item.date_released_unix > previous_date {
                source_ordered = false;
            }
            previous_date = item.date_released_unix;
        }
        if overlap_after.is_none()
            && source_ordered
            && boundary.is_some_and(|b| r.torrents.iter().any(|t| t.date_released_unix < b))
        {
            overlap_after = Some(page);
        }
        items.extend(r.torrents);
        if n < 100 || (r.torrents_count > 0 && items.len() as i64 >= r.torrents_count) {
            complete = true;
            break;
        }
        if source_ordered && overlap_after.is_some_and(|p| page > p) {
            break;
        }
        if page == MAX_PAGES {
            complete = false;
        }
    }
    let episodes = reduce(
        items
            .into_iter()
            .map(|t| Raw {
                title: t.title,
                magnet: t.magnet_url,
                season: as_i64(&t.season),
                episode: as_i64(&t.episode),
                seeds: t.seeds,
                size: as_i64(&t.size_bytes),
                released: t.date_released_unix,
            })
            .collect(),
        quality,
    );
    Ok(EpisodeCheck { episodes, catalogue_complete: complete, warning: (!complete).then(||
        format!("EZTV history is partial (check budget: {MAX_PAGES} pages / {} seconds). Newest and previously discovered releases are kept; older source releases may be missing.", CHECK_BUDGET.as_secs())) })
}

struct Raw {
    title: String,
    magnet: String,
    season: i64,
    episode: i64,
    seeds: i64,
    size: i64,
    released: i64,
}

/// Group by (season, episode) and keep the best release of each: preferred quality
/// first, then seeds. Complete seasons have their own Snn key, so a later season
/// pack does not erase the read state of individual episodes or another season.
fn reduce(items: Vec<Raw>, quality: &str) -> Vec<Episode> {
    let mut groups: BTreeMap<(u16, u16), Vec<Raw>> = BTreeMap::new();
    for mut it in items {
        if it.magnet.is_empty() {
            continue;
        }
        if it.episode <= 0 {
            // EZTV sometimes reports 0/0; fall back to the release name.
            if let Some(ep) = crate::release::parse(&it.title).episode {
                if let Some((s, e)) = split_key(&ep) {
                    it.season = s as i64;
                    it.episode = e as i64;
                }
            }
            if it.episode == 0 && it.season <= 0 {
                if let Some(s) = season_of(&it.title) {
                    it.season = s as i64;
                }
            }
        }
        if it.episode < 0
            || it.episode > 9999
            || it.season < 0
            || it.season > 999
            || (it.episode == 0 && it.season == 0)
        {
            continue;
        }
        groups
            .entry((it.season as u16, it.episode as u16))
            .or_default()
            .push(it);
    }
    let want = quality.to_ascii_lowercase();
    let mut out: Vec<Episode> = groups
        .into_iter()
        .map(|((s, e), mut v)| {
            let candidates = v.len();
            v.sort_by(|a, b| {
                let qa = !want.is_empty() && a.title.to_ascii_lowercase().contains(&want);
                let qb = !want.is_empty() && b.title.to_ascii_lowercase().contains(&want);
                qb.cmp(&qa)
                    .then(b.seeds.cmp(&a.seeds))
                    .then(b.released.cmp(&a.released))
            });
            let best = v.remove(0);
            Episode {
                key: if e == 0 {
                    format!("S{s:02}")
                } else {
                    format!("S{s:02}E{e:02}")
                },
                season: s,
                episode: e,
                quality: quality_of(&best.title),
                title: best.title,
                magnet: best.magnet,
                seeds: best.seeds,
                size: best.size,
                released: best.released,
                candidates,
            }
        })
        .collect();
    out.sort_by(|a, b| (b.season, b.episode).cmp(&(a.season, a.episode)));
    out
}

fn split_key(key: &str) -> Option<(u16, u16)> {
    let k = key.to_ascii_uppercase();
    let (s, e) = if let Some(rest) = k.strip_prefix('S') {
        rest.split_once('E')?
    } else {
        k.split_once('X')?
    };
    Some((s.parse().ok()?, e.parse().ok()?))
}

fn season_of(name: &str) -> Option<u16> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let digits = word
            .strip_prefix('s')
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .or_else(|| {
                (*word == "season")
                    .then(|| words.get(i + 1).copied())
                    .flatten()
            });
        if let Some(s) = digits
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|&s| s > 0 && s <= 999)
        {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests;
