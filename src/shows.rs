//! Followed TV shows: a local list that is refreshed from EZTV when the app opens.
//!
//! No service, no timer. `Shows` lives in `~/.local/share/fbittorrent/shows.json` and
//! carries, per show, the episodes found at the last check plus what the user has
//! grabbed or marked seen. The TUI refreshes stale shows on launch; the CLI does it
//! on `shows check`.

use anyhow::{anyhow, Context, Result};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpState {
    New,
    Grabbed,
    Seen,
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
    pub fn state(&self, key: &str) -> EpState {
        if self.grabbed.contains_key(key) {
            EpState::Grabbed
        } else if self.seen.contains(key) {
            EpState::Seen
        } else {
            EpState::New
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
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "imdb": self.imdb, "title": self.title, "year": self.year, "quality": self.quality,
            "added": self.added, "last_checked": self.last_checked, "error": self.error,
            "catalogue_complete": self.catalogue_complete,
            "episodes": self.episodes.len(), "new": self.new_count(),
            "seen": self.seen.len(), "grabbed": self.grabbed.len(),
        })
    }
    pub fn episode_json(&self, e: &Episode) -> serde_json::Value {
        let state = match self.state(&e.key) {
            EpState::New => "new",
            EpState::Grabbed => "grabbed",
            EpState::Seen => "seen",
        };
        serde_json::json!({
            "key": e.key, "season": e.season, "episode": e.episode, "state": state,
            "read": self.seen.contains(&e.key), "new": self.is_new(&e.key),
            "grabbed": self.grabbed.contains_key(&e.key), "season_pack": e.is_season_pack(),
            "title": e.title, "magnet": e.magnet, "seeds": e.seeds, "size": e.size,
            "released": e.released, "quality": e.quality, "candidates": e.candidates,
        })
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

    /// Find a show by IMDb id, exact title, or a unique part of the title.
    pub fn find(&self, sel: &str) -> Result<&Show> {
        let s = sel.trim();
        if let Some(sh) = self
            .shows
            .iter()
            .find(|x| x.imdb.eq_ignore_ascii_case(s) || x.title.eq_ignore_ascii_case(s))
        {
            return Ok(sh);
        }
        let low = s.to_lowercase();
        let hits: Vec<&Show> = self
            .shows
            .iter()
            .filter(|x| x.title.to_lowercase().contains(&low))
            .collect();
        match hits.len() {
            1 => Ok(hits[0]),
            0 => Err(crate::agent::AppError::new(
                crate::agent::Code::NotFound,
                format!("no followed show matches '{sel}'"),
            )
            .hint("fbittorrent shows list --json")
            .into()),
            _ => Err(crate::agent::AppError::new(
                crate::agent::Code::Ambiguous,
                format!(
                    "'{sel}' matches {} shows: {}",
                    hits.len(),
                    hits.iter()
                        .map(|h| h.title.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .hint("use the IMDb id")
            .into()),
        }
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

// ---------------------------------------------------------------- lookup

/// Resolve what the user typed into a title: an IMDb id, or a name (TV results first).
pub fn resolve_title(query: &str) -> Result<Title> {
    let q = query.trim();
    if q.is_empty() {
        return Err(anyhow!("empty show name"));
    }
    if q.starts_with("tt") && q[2..].chars().all(|c| c.is_ascii_digit()) {
        return crate::imdb::lookup_id(q)?.ok_or_else(|| anyhow!("IMDb knows no title {q}"));
    }
    let mut rel = crate::release::parse(q);
    if rel.title.is_empty() {
        rel.title = q.to_string();
    }
    let mut cands = crate::imdb::matching_candidates(
        &rel, true, crate::imdb::search(&rel.title, rel.year)?,
    );
    if cands.is_empty() && rel.title != q {
        cands = crate::imdb::matching_candidates(&rel, true, crate::imdb::search(q, None)?);
    }
    // The shared matcher prefers series and year only among matching titles.
    let mut t = cands
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("IMDb has no title matching '{q}'"))?;
    crate::imdb::enrich(&mut t);
    Ok(t)
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

/// Check on app launch or an explicit refresh. An established catalogue usually
/// needs one newest page and one overlap page; no work continues after app exit.
pub fn fetch_updates(show: &Show, default_quality: &str) -> Result<EpisodeCheck> {
    let quality = show.quality.as_deref().unwrap_or(default_quality);
    let boundary = show
        .episodes
        .iter()
        .map(|e| e.released)
        .filter(|&t| t > 0)
        .max();
    fetch_eztv(&show.imdb, quality, boundary, show.catalogue_complete)
}

fn fetch_eztv(
    imdb: &str,
    quality: &str,
    boundary: Option<i64>,
    complete_before: bool,
) -> Result<EpisodeCheck> {
    let digits = imdb.trim_start_matches("tt");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent("fbittorrent/0.1 (show feed)")
        .build()
        .into();
    fetch_pages(quality, boundary, complete_before, |page, remaining| {
        let url =
            format!("https://eztv.re/api/get-torrents?imdb_id={digits}&limit=100&page={page}");
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

/// Refresh one show in place. Keeps the old episode list on failure and records the error.
pub fn refresh(show: &mut Show, default_quality: &str) -> Result<usize> {
    match fetch_updates(show, default_quality) {
        Ok(check) => Ok(show.apply_check(check)),
        Err(e) => {
            show.error = Some(format!("{e:#}"));
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_save_is_private_for_new_and_existing_stores() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-private-{}", std::process::id()));
        let path = dir.join("shows.json");
        let mut shows = Shows::default();
        shows.follow(&Title { id: "tt1234001".into(), name: "Private fixture".into(), ..Default::default() });
        for replacement in [false, true] {
            if replacement {
                for file in [&path, &path.with_extension("json.lock")] {
                    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            shows.save_to(&path).unwrap();
            for file in [&path, &path.with_extension("json.lock")] {
                let mode = std::fs::metadata(file).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", file.display());
                println!("{}: {mode:o} after Shows save", file.display());
            }
            assert_eq!(Shows::load_from(&path).unwrap().shows[0].title, "Private fixture");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn raw(title: &str, s: i64, e: i64, seeds: i64) -> Raw {
        Raw {
            title: title.into(),
            magnet: format!("magnet:?xt=urn:btih:{title}"),
            season: s,
            episode: e,
            seeds,
            size: 1,
            released: 0,
        }
    }

    #[test]
    fn picks_preferred_quality_then_seeds() {
        let eps = reduce(
            vec![
                raw("Show.S01E01.720p", 1, 1, 900),
                raw("Show.S01E01.1080p", 1, 1, 50),
                raw("Show.S01E02.2160p", 1, 2, 10),
                raw("Show.S01E02.720p", 1, 2, 500),
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].key, "S01E02"); // newest first
        assert_eq!(eps[0].title, "Show.S01E02.720p"); // no 1080p: most seeds wins
        assert_eq!(eps[1].title, "Show.S01E01.1080p");
        assert_eq!(eps[1].quality, "1080p");
        assert_eq!(eps[1].candidates, 2);
    }

    #[test]
    fn falls_back_to_release_name_and_drops_junk() {
        let eps = reduce(
            vec![
                raw("Show.S03E04.1080p.WEB", 0, 0, 5),
                raw("Show.Complete.Season.Pack", 0, 0, 5),
                Raw {
                    magnet: String::new(),
                    ..raw("Show.S03E05", 3, 5, 5)
                },
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].key, "S03E04");
    }

    #[test]
    fn states_and_counts() {
        let mut s = Show {
            imdb: "tt1".into(),
            title: "T".into(),
            ..Default::default()
        };
        s.episodes = reduce(
            vec![raw("a", 1, 1, 1), raw("b", 1, 2, 1), raw("c", 1, 3, 1)],
            "",
        );
        assert_eq!(s.new_count(), 3);
        s.mark_grabbed("S01E03", "magnet:x");
        assert!(s.toggle_seen("S01E02"));
        assert_eq!(s.new_count(), 1);
        assert_eq!(s.state("S01E03"), EpState::Grabbed);
        assert_eq!(s.state("S01E02"), EpState::Seen);
        assert_eq!(s.mark_all_seen(), 1);
        assert_eq!(s.new_count(), 0);
        assert!(!s.toggle_seen("S01E02"));
        assert_eq!(s.new_count(), 1);
    }

    #[test]
    fn roundtrip_and_find() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-{}", std::process::id()));
        let p = dir.join("shows.json");
        let mut all = Shows::default();
        let t = Title {
            id: "tt0903747".into(),
            name: "Breaking Bad".into(),
            year: Some(2008),
            ..Default::default()
        };
        assert!(all.follow(&t));
        assert!(!all.follow(&t));
        all.get_mut("tt0903747")
            .unwrap()
            .seen
            .insert("S01E01".into());
        all.save_to(&p).unwrap();
        let back = Shows::load_from(&p).unwrap();
        assert_eq!(back.shows.len(), 1);
        assert!(back.get("tt0903747").unwrap().seen.contains("S01E01"));
        assert_eq!(back.find("breaking").unwrap().imdb, "tt0903747");
        assert!(back.find("nothing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eztv_json_shape() {
        let body = r#"{"torrents_count":2,"limit":100,"page":1,"torrents":[
          {"title":"Show S05E16 1080p WEB","magnet_url":"magnet:?xt=urn:btih:aa","season":"5","episode":"16","seeds":120,"size_bytes":"1500000000","date_released_unix":1700000000},
          {"title":"Show S05E16 720p HDTV","magnet_url":"magnet:?xt=urn:btih:bb","season":"5","episode":"16","seeds":300,"size_bytes":"700000000","date_released_unix":1700000100}]}"#;
        let r: EztvResp = serde_json::from_str(body).unwrap();
        assert_eq!(r.torrents_count, 2);
        let eps = reduce(
            r.torrents
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
            "1080p",
        );
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].magnet, "magnet:?xt=urn:btih:aa");
        assert_eq!(eps[0].size, 1_500_000_000);
    }

    #[test]
    fn refresh_retains_old_unread_releases_and_per_key_history() {
        let mut s = Show::default();
        s.episodes = reduce(
            vec![
                raw("Show.S01E01.720p", 1, 1, 10),
                raw("Show.S01E02.720p", 1, 2, 10),
            ],
            "720p",
        );
        s.mark_grabbed("S01E01", "magnet:old");
        assert!(!s.toggle_seen("S01E01")); // unread is independent of downloaded
        let new = reduce(
            vec![
                raw("Show.S03E01.1080p", 3, 1, 20),
                raw("Show.S01E01.1080p", 1, 1, 50),
            ],
            "1080p",
        );
        assert_eq!(s.merge_episodes(new), 1);
        assert_eq!(s.episodes.len(), 3);
        assert_eq!(s.episodes[0].key, "S03E01");
        assert!(s.episode("S01E02").is_some()); // missing from this source response
        assert_eq!(s.episode("S01E01").unwrap().quality, "1080p");
        assert_eq!(
            s.grabbed.get("S01E01").map(String::as_str),
            Some("magnet:old")
        );
        assert!(s.is_new("S01E01"));
        assert_eq!(s.new_count(), 3);
        assert_eq!(s.merge_episodes(Vec::new()), 0);
        assert_eq!(s.episodes.len(), 3);
    }

    #[test]
    fn season_packs_are_separate_from_episodes_and_survive_read_changes() {
        let eps = reduce(
            vec![
                raw("Show.S02.COMPLETE.720p", 2, 0, 50),
                raw("Show.S02.COMPLETE.1080p", 2, 0, 10),
                raw("Show.S02E01.1080p", 2, 1, 10),
                raw("Show.Complete.Season.3.1080p", 0, 0, 20),
                raw("Show.1x04.720p", 0, 0, 10),
            ],
            "1080p",
        );
        assert_eq!(eps.len(), 4);
        let pack = eps.iter().find(|e| e.key == "S02").unwrap();
        assert!(pack.is_season_pack());
        assert_eq!(pack.quality, "1080p");
        assert_eq!(pack.candidates, 2);
        let mut s = Show {
            episodes: eps,
            ..Default::default()
        };
        assert!(s.toggle_seen("S02"));
        assert!(!s.is_new("S02"));
        assert!(s.is_new("S02E01"));
        assert!(s.is_new("S03"));
        assert!(s.episode("S01E04").is_some());
    }

    #[test]
    fn concurrent_saves_merge_read_changes_without_resurrecting_unfollowed_shows() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-merge-{}", std::process::id()));
        let p = dir.join("shows.json");
        let t = Title {
            id: "tt1".into(),
            name: "Show".into(),
            ..Default::default()
        };
        let mut initial = Shows::default();
        initial.follow(&t);
        initial.get_mut("tt1").unwrap().episodes = reduce(
            vec![raw("Show.S01E01", 1, 1, 1), raw("Show.S01E02", 1, 2, 1)],
            "",
        );
        initial.save_to(&p).unwrap();
        let mut tui = Shows::load_from(&p).unwrap();
        let mut cli = Shows::load_from(&p).unwrap();
        cli.get_mut("tt1").unwrap().toggle_seen("S01E01");
        cli.save_to(&p).unwrap();
        tui.get_mut("tt1")
            .unwrap()
            .mark_grabbed("S01E02", "magnet:second");
        tui.save_to(&p).unwrap();
        tui.save_to(&p).unwrap(); // another unchanged save retains the CLI edit
        let merged = Shows::load_from(&p).unwrap();
        assert_eq!(merged.get("tt1").unwrap().new_count(), 0);
        assert!(merged.get("tt1").unwrap().seen.contains("S01E01"));
        assert!(merged.get("tt1").unwrap().grabbed.contains_key("S01E02"));
        let mut removal = Shows::load_from(&p).unwrap();
        removal.unfollow("tt1");
        removal.save_to(&p).unwrap();
        tui.get_mut("tt1").unwrap().toggle_seen("S01E01");
        tui.save_to(&p).unwrap();
        assert!(Shows::load_from(&p).unwrap().shows.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_history_is_an_error_and_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("fbt-shows-corrupt-{}", std::process::id()));
        let p = dir.join("shows.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "{broken history").unwrap();
        assert!(Shows::load_from(&p).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{broken history");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn source_page(page: usize, total: i64) -> EztvResp {
        EztvResp {
            torrents_count: total,
            torrents: (0..100)
                .map(|i| EztvItem {
                    title: format!("Show.S01E{:02}.1080p", (page - 1) * 100 + i + 1),
                    magnet_url: format!("magnet:{page}:{i}"),
                    season: 1.into(),
                    episode: ((page - 1) * 100 + i + 1).into(),
                    seeds: 1,
                    size_bytes: 1.into(),
                    date_released_unix: 10_000 - ((page - 1) * 100 + i) as i64,
                })
                .collect(),
        }
    }

    #[test]
    fn incremental_check_fetches_boundary_and_one_overlap_page() {
        let mut pages = Vec::new();
        let check = fetch_pages("1080p", Some(9_950), true, |p, _| {
            pages.push(p);
            Ok(source_page(p, 20_000))
        })
        .unwrap();
        assert_eq!(pages, [1, 2]);
        assert_eq!(check.episodes.len(), 200);
        assert!(check.catalogue_complete);
        assert!(check.warning.is_none());
    }

    #[test]
    fn large_first_catalogue_has_an_explicit_budget_warning() {
        let mut pages = 0;
        let check = fetch_pages("1080p", None, false, |p, _| {
            pages += 1;
            Ok(source_page(p, 20_000))
        })
        .unwrap();
        assert_eq!(pages, MAX_PAGES);
        assert_eq!(check.episodes.len(), MAX_PAGES * 100);
        assert!(!check.catalogue_complete);
        assert!(check
            .warning
            .unwrap()
            .contains("older source releases may be missing"));
    }
}
