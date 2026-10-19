//! Current CLI 1.x output shapes and shared display helpers.

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if="is_false")]
    pub staged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub progress: f64,
    #[serde(default = "known_progress")]
    pub progress_known: bool,
    pub size: i64,
    pub completed: i64,
    pub dlspeed: i64,
    pub upspeed: i64,
    pub eta: i64,
    pub num_seeds: i64,
    pub num_leechs: i64,
    pub ratio: f64,
    pub seq_dl: bool,
    pub f_l_piece_prio: bool,
    pub save_path: String,
    pub content_path: String,
    pub category: String,
    pub added_on: i64,
    pub completion_on: i64,
    pub up_limit: i64,
    pub ratio_limit: f64,
    pub tags: String,
}

fn is_false(value: &bool) -> bool { !value }

fn known_progress() -> bool { true }


impl Torrent {
    pub fn is_active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "downloading" | "metaDL" | "forcedDL" | "stalledDL" | "checkingDL"
                | "allocating" | "forcedMetaDL"
        )
    }
    pub fn is_stopped(&self) -> bool {
        matches!(self.state.as_str(), "stoppedDL" | "stoppedUP" | "pausedDL" | "pausedUP")
    }
    pub fn is_queued(&self) -> bool { self.state == "queuedDL" }
    pub fn complete_no_seed(&self) -> bool { self.completion_on > 0 && self.ratio_limit == 0.0 }
    /// Explicit permission to continue uploading after completion.
    pub fn seed_allowed(&self) -> bool {
        self.has_tag("seed") || self.ratio_limit != 0.0
    }
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.split(',').any(|t| t.trim() == tag)
    }
    pub fn is_seeding(&self) -> bool {
        matches!(self.state.as_str(), "uploading" | "stalledUP" | "forcedUP" | "queuedUP" | "checkingUP")
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StartResult {
    pub started: Vec<String>,
    pub skipped: Vec<StartSkipped>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct StartSkipped {
    pub hash: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferInfo {
    pub dl_info_speed: i64,
    pub up_info_speed: i64,
    pub dl_info_data: i64,
    pub up_info_data: i64,
    pub connection_status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchResult {
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "fileUrl")]
    pub file_url: String,
    #[serde(rename = "fileSize")]
    pub file_size: i64,
    #[serde(rename = "nbSeeders")]
    pub seeders: i64,
    #[serde(rename = "nbLeechers")]
    pub leechers: i64,
    #[serde(rename = "siteUrl")]
    pub site_url: String,
    #[serde(rename = "descrLink")]
    pub descr_link: String,
    #[serde(rename = "engineName")]
    pub engine: String,
    /// IMDb id when the source provides one (browse lists do).
    #[serde(default)]
    pub imdb: String,
    /// Upload date as unix seconds (0 = unknown). qBittorrent plugins report `pubDate`.
    #[serde(default, rename = "pubDate")]
    pub pub_date: i64,
}

/// Unix seconds -> "YYYY-MM-DD" (UTC). Empty when unknown.
pub fn ymd(unix: i64) -> String {
    if unix <= 0 { return String::new(); }
    let days = unix.div_euclid(86400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentFile {
    pub index: i64,
    pub name: String,
    pub size: i64,
    pub progress: f64,
    #[serde(default = "known_progress")]
    pub progress_known: bool,
    /// 0 = skip, 1 = normal, 6 = high, 7 = maximum
    pub priority: i64,
}

#[cfg(test)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub status: String,
    pub total: i64,
    /// Source diagnostics can coexist with useful results from other providers.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchPlugin {
    #[serde(default="nova_kind")]
    pub kind:String,
    pub name: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub enabled: bool,
    pub url: String,
    pub version: String,
}

fn nova_kind()->String {"nova".into()}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RssArticle {
    pub id: String,
    pub title: String,
    pub link: String,
    #[serde(rename = "torrentURL")]
    pub torrent_url: String,
    pub date: String,
    pub description: String,
    #[serde(rename = "isRead")]
    pub is_read: bool,
    pub size: Option<i64>,
    /// Parsed `date` as unix seconds (0 when unparseable). Filled in by the client.
    #[serde(skip)]
    pub ts: i64,
}

/// Parse the RFC 2822-ish dates qBittorrent emits ("02 Sep 2026 03:58:00 +0000").
pub fn parse_rfc2822(s: &str) -> i64 {
    let s = s.trim();
    let s = s.split_once(", ").map(|(_, r)| r).unwrap_or(s); // drop weekday
    let mut it = s.split_whitespace();
    let (Some(d), Some(mon), Some(y), Some(hms)) = (it.next(), it.next(), it.next(), it.next()) else { return 0 };
    let zone = it.next().unwrap_or("+0000");
    let months = ["jan","feb","mar","apr","may","jun","jul","aug","sep","oct","nov","dec"];
    let Some(m) = months.iter().position(|m| mon.to_ascii_lowercase().starts_with(m)) else { return 0 };
    let (Ok(d), Ok(y)) = (d.parse::<i64>(), y.parse::<i64>()) else { return 0 };
    let mut t = hms.split(':').map(|p| p.parse::<i64>().unwrap_or(0));
    let (h, mi, se) = (t.next().unwrap_or(0), t.next().unwrap_or(0), t.next().unwrap_or(0));
    // days from civil (Howard Hinnant)
    let m = m as i64 + 1;
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut ts = days * 86400 + h * 3600 + mi * 60 + se;
    if let Some(sign) = zone.chars().next() {
        if (sign == '+' || sign == '-') && zone.len() >= 5 {
            let zh: i64 = zone[1..3].parse().unwrap_or(0);
            let zm: i64 = zone[3..5].parse().unwrap_or(0);
            let off = zh * 3600 + zm * 60;
            ts = if sign == '+' { ts - off } else { ts + off };
        }
    }
    ts
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RssFeed {
    pub name: String,
    pub uid: String,
    pub url: String,
    pub title: String,
    pub last_build_date: String,
    pub is_loading: bool,
    pub has_error: bool,
    pub articles: Vec<RssArticle>,
}

/// Community search plugins from the qBittorrent wiki that are maintained and public.
pub const COMMUNITY_SEARCH_PLUGINS: &[&str] = &[
    "https://codeberg.org/lazulyra/qbit-plugins/raw/branch/main/yts/yts.py",
    "https://raw.githubusercontent.com/nindogo/qbtSearchScripts/master/torrentgalaxy.py",
    "https://raw.githubusercontent.com/nindogo/qbtSearchScripts/master/magnetdl.py",
    "https://raw.githubusercontent.com/BurningMop/qBittorrent-Search-Plugins/refs/heads/main/therarbg.py",
    "https://raw.githubusercontent.com/BurningMop/qBittorrent-Search-Plugins/refs/heads/main/bitsearch.py",
    "https://raw.githubusercontent.com/BurningMop/qBittorrent-Search-Plugins/refs/heads/main/torrentdownloads.py",
    "https://raw.githubusercontent.com/LightDestory/qBittorrent-Search-Plugins/master/src/engines/kickasstorrents.py",
    "https://raw.githubusercontent.com/LightDestory/qBittorrent-Search-Plugins/master/src/engines/snowfl.py",
    "https://raw.githubusercontent.com/444995/qbit-search-plugins/main/engines/zooqle.py",
    "https://raw.githubusercontent.com/DrPurp/eztvx-qbittorrent-plugin/main/eztvx.py",
    // nyaa.si proper. Not phuongtailtranminh/nyaa.py: that one scrapes Sukebei and imports bs4, which qBittorrent's Python lacks.
    "https://raw.githubusercontent.com/MadeOfMagicAndWires/qBit-plugins/main/engines/nyaasi.py",
];

pub struct AddOptions {
    /// Explicitly permit upload, including after completion; false inherits the global download-time policy and stops when complete.
    pub seed: bool,
    pub stopped: bool,
    pub save_path: Option<String>,
    pub category: Option<String>,
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn human_bytes(n: i64) -> String {
    let n = n.max(0) as f64;
    const U: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = n;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 { v /= 1024.0; i += 1; }
    if i == 0 { format!("{v:.0} {}", U[i]) } else { format!("{v:.1} {}", U[i]) }
}

pub fn human_rate(n: i64) -> String {
    format!("{}/s", human_bytes(n))
}

pub fn human_eta(secs: i64) -> String {
    if secs < 0 || secs >= 8640000 { return "∞".into(); }
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 { format!("{h}h{m:02}m") } else if m > 0 { format!("{m}m{s:02}s") } else { format!("{s}s") }
}

#[cfg(test)]
mod completion_tests {
    #[test]
    fn restore_error_message_is_optional_and_round_trips() {
        let old: super::Torrent = serde_json::from_value(serde_json::json!({"hash":"old"})).unwrap();
        assert!(old.error_message.is_none());
        assert!(serde_json::to_value(old).unwrap().get("error_message").is_none());
        let failed: super::Torrent = serde_json::from_value(serde_json::json!({
            "hash":"failed", "state":"error", "error_message":"could not restore: corrupt cache"
        })).unwrap();
        assert_eq!(serde_json::to_value(failed).unwrap()["error_message"], "could not restore: corrupt cache");
    }

    use super::*;

    #[test]
    fn resume_completion_requires_a_positive_timestamp() {
        for completion_on in [-1, 0, 123] {
            let torrent: Torrent = serde_json::from_value(serde_json::json!({
                "completion_on": completion_on, "ratio_limit": 0.0,
            })).unwrap();
            assert_eq!(torrent.complete_no_seed(), completion_on > 0);
        }
    }
}
