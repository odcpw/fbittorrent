//! Shared client for the app-owned engine and the legacy qBittorrent WebUI.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::Config;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub state: String,
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

fn known_progress() -> bool { true }

/// Upload cap that means "no upload". Not literally zero: libtorrent charges protocol
/// messages (piece and metadata requests) against the limit, and at 1 B/s a download
/// never gets past fetching metadata. 32 KiB/s leaves room for the chatter while
/// keeping payload upload negligible; torrents stop the moment they complete anyway.
pub const NO_UPLOAD_LIMIT: i64 = 32 * 1024;
pub const SEED_TAG: &str = "seed";

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
    /// True when uploading is allowed (no default cap or explicitly tagged "seed").
    pub fn seed_allowed(&self) -> bool {
        self.has_tag(SEED_TAG) || self.up_limit != NO_UPLOAD_LIMIT
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
    pub name: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    pub enabled: bool,
    pub url: String,
    pub version: String,
}

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
    pub origin: Option<String>,
    pub sequential: bool,
    pub first_last: bool,
    /// Allow upload. When false the torrent is capped and stops once complete.
    pub seed: bool,
    pub stopped: bool,
    pub save_path: Option<String>,
    pub category: Option<String>,
}

pub struct Client {
    cfg: Config,
    agent: ureq::Agent,
    base: String,
    username: Option<String>,
    password: Option<String>,
}

impl Client {
    pub fn from_config(cfg: &Config) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(cfg.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            cfg: cfg.clone(),
            agent,
            base: cfg.url.trim_end_matches('/').to_string(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v2/{}", self.base, path)
    }

    /// Log in when credentials are configured. Localhost with LocalHostAuth=false needs none.
    pub fn login(&self) -> Result<()> {
        if self.cfg.native() { return Ok(()); }
        let (Some(u), Some(p)) = (&self.username, &self.password) else { return Ok(()) };
        let mut resp = self
            .agent
            .post(self.url("auth/login"))
            .header("Referer", &self.base)
            .send_form([("username", u.as_str()), ("password", p.as_str())])
            .context("connecting to qBittorrent")?;
        let body = resp.body_mut().read_to_string()?;
        if resp.status().as_u16() != 200 || body.trim() != "Ok." {
            bail!("qBittorrent login failed ({}): {}", resp.status(), body.trim());
        }
        Ok(())
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<String> {
        if self.cfg.native() { return crate::ipc::request(&self.cfg, "GET", path, query, None); }
        let mut url = self.url(path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&encode_query(query));
        }
        let do_get = || -> Result<(u16, String)> {
            let mut resp = self.agent.get(&url).call().context("connecting to qBittorrent")?;
            let status = resp.status().as_u16();
            let body = resp.body_mut().read_to_string()?;
            Ok((status, body))
        };
        let (mut status, mut body) = do_get()?;
        if status == 403 && self.username.is_some() {
            self.login()?;
            (status, body) = do_get()?;
        }
        check(status, &body, path)?;
        Ok(body)
    }

    /// multipart/form-data POST with one file part (for `torrents/add` with a .torrent).
    fn post_multipart(&self, path: &str, fields: &[(&str, &str)], file_field: &str, filename: &str, bytes: &[u8]) -> Result<String> {
        if self.cfg.native() { return crate::ipc::request(&self.cfg, "POST", path, fields, Some((filename, bytes))); }
        let boundary = format!("----fbittorrent{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let mut body: Vec<u8> = Vec::with_capacity(bytes.len() + 1024);
        for (k, v) in fields {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
        }
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: application/x-bittorrent\r\n\r\n", filename.replace('"', "_")).as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = self.url(path);
        let do_post = |body: &[u8]| -> Result<(u16, String)> {
            let mut resp = self.agent.post(&url)
                .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
                .send(body)
                .context("connecting to qBittorrent")?;
            let status = resp.status().as_u16();
            let text = resp.body_mut().read_to_string()?;
            Ok((status, text))
        };
        let (mut status, mut text) = do_post(&body)?;
        if status == 403 && self.username.is_some() {
            self.login()?;
            (status, text) = do_post(&body)?;
        }
        check(status, &text, path)?;
        Ok(text)
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<String> {
        if self.cfg.native() { return crate::ipc::request(&self.cfg, "POST", path, form, None); }
        let url = self.url(path);
        let do_post = || -> Result<(u16, String)> {
            let mut resp = self
                .agent
                .post(&url)
                .send_form(form.iter().map(|(k, v)| (*k, *v)))
                .context("connecting to qBittorrent")?;
            let status = resp.status().as_u16();
            let body = resp.body_mut().read_to_string()?;
            Ok((status, body))
        };
        let (mut status, mut body) = do_post()?;
        if status == 403 && self.username.is_some() {
            self.login()?;
            (status, body) = do_post()?;
        }
        check(status, &body, path)?;
        Ok(body)
    }

    // ---- app ----
    pub fn version(&self) -> Result<String> {
        Ok(self.get("app/version", &[])?.trim().to_string())
    }
    pub fn set_queue_limit(&self, limit: usize) -> Result<()> {
        self.post("app/setQueue", &[("max_active_downloads", &limit.to_string())])?;
        Ok(())
    }
    pub fn restart_app(&self) -> Result<()> {
        self.post("app/restart", &[])?;
        Ok(())
    }
    pub fn transfer_info(&self) -> Result<TransferInfo> {
        Ok(serde_json::from_str(&self.get("transfer/info", &[])?)?)
    }
    pub fn set_preferences(&self, json: &serde_json::Value) -> Result<()> {
        self.post("app/setPreferences", &[("json", &json.to_string())])?;
        Ok(())
    }

    // ---- torrents ----
    pub fn torrents(&self) -> Result<Vec<Torrent>> {
        let body = self.get("torrents/info", &[("sort", "added_on"), ("reverse", "true")])?;
        Ok(serde_json::from_str(&body)?)
    }
    pub fn add_url(&self, url: &str, o: &AddOptions) -> Result<()> {
        let mut form: Vec<(&str, &str)> = vec![
            ("urls", url),
            ("sequentialDownload", bool_s(o.sequential)),
            ("firstLastPiecePrio", bool_s(o.first_last)),
            ("stopped", bool_s(o.stopped)),
        ];
        if let Some(p) = &o.save_path { form.push(("savepath", p)); }
        if let Some(c) = &o.category { form.push(("category", c)); }
        let no_up = NO_UPLOAD_LIMIT.to_string();
        if !o.seed {
            form.push(("upLimit", &no_up));
            form.push(("ratioLimit", "0"));
            form.push(("shareLimitAction", "Stop"));
        }
        let tags = [o.seed.then_some(SEED_TAG.to_owned()), o.origin.as_ref().map(|origin| format!("fbt-origin:{origin}"))].into_iter().flatten().collect::<Vec<_>>().join(",");
        if !tags.is_empty() { form.push(("tags", &tags)); }
        let body = self.post("torrents/add", &form)?;
        add_response_ok(&body)
    }
    /// Resolve a selected provider result and preserve the caller's add settings.
    pub fn add_search_result(&self, result: &SearchResult, o: &AddOptions) -> Result<()> {
        if self.cfg.native() && self.cfg.network_interface.is_some() {
            // Provider download hooks run outside the engine's bound sockets.
            // Direct links are fetched by the protected engine instead.
            let source = result.file_url.trim();
            let direct = source.starts_with("magnet:") || ((source.starts_with("https://") || source.starts_with("http://")) && source.split(['?', '#']).next().is_some_and(|path| path.to_ascii_lowercase().ends_with(".torrent")));
            if direct { return self.add_url(source, o); }
            let known_provider = crate::search::plugins()?.iter().any(|plugin| plugin.name == result.engine);
            if !known_provider && (source.starts_with("https://") || source.starts_with("http://")) { return self.add_url(source, o); }
            bail!("this provider needs an external download helper; use its magnet link or a .torrent URL with VPN binding enabled");
        }
        match crate::search::resolve_download(result, self.cfg.timeout_secs)? {
            crate::search::ResolvedDownload::Url(url) => self.add_url(&url, o),
            crate::search::ResolvedDownload::Torrent { filename, bytes } => self.add_file(&filename, &bytes, o),
        }
    }

    /// Add a .torrent file (bytes). Same defaults as `add_url`.
    pub fn add_file(&self, filename: &str, bytes: &[u8], o: &AddOptions) -> Result<()> {
        let no_up = NO_UPLOAD_LIMIT.to_string();
        let mut fields: Vec<(&str, &str)> = vec![
            ("sequentialDownload", bool_s(o.sequential)),
            ("firstLastPiecePrio", bool_s(o.first_last)),
            ("stopped", bool_s(o.stopped)),
        ];
        if let Some(p) = &o.save_path { fields.push(("savepath", p)); }
        if let Some(c) = &o.category { fields.push(("category", c)); }
        if !o.seed {
            fields.push(("upLimit", &no_up));
            fields.push(("ratioLimit", "0"));
            fields.push(("shareLimitAction", "Stop"));
        }
        let tags = [o.seed.then_some(SEED_TAG.to_owned()), o.origin.as_ref().map(|origin| format!("fbt-origin:{origin}"))].into_iter().flatten().collect::<Vec<_>>().join(",");
        if !tags.is_empty() { fields.push(("tags", &tags)); }
        let body = self.post_multipart("torrents/add", &fields, "torrents", filename, bytes)?;
        add_response_ok(&body)
    }

    pub fn start(&self, hashes: &[String]) -> Result<()> {
        self.start_report(hashes, false)?;
        Ok(())
    }
    pub fn start_report(&self, hashes: &[String], skip_complete: bool) -> Result<StartResult> {
        let mut result = StartResult::default();
        for chunk in hashes.chunks(256) {
            let body = self.post("torrents/start", &[("hashes", &chunk.join("|")), ("skip_complete", bool_s(skip_complete))])?;
            let part = if self.cfg.native() { serde_json::from_str::<StartResult>(&body)? }
                else { StartResult { started: chunk.to_vec(), skipped: Vec::new() } };
            result.started.extend(part.started);
            result.skipped.extend(part.skipped);
        }
        Ok(result)
    }
    pub fn stop(&self, hashes: &[String]) -> Result<()> {
        self.post("torrents/stop", &[("hashes", &hashes.join("|"))])?;
        Ok(())
    }
    pub fn delete(&self, hashes: &[String], delete_files: bool) -> Result<()> {
        self.post("torrents/delete", &[("hashes", &hashes.join("|")), ("deleteFiles", bool_s(delete_files))])?;
        Ok(())
    }
    pub fn toggle_sequential(&self, hashes: &[String]) -> Result<()> {
        self.post("torrents/toggleSequentialDownload", &[("hashes", &hashes.join("|"))])?;
        Ok(())
    }
    pub fn toggle_first_last(&self, hashes: &[String]) -> Result<()> {
        self.post("torrents/toggleFirstLastPiecePrio", &[("hashes", &hashes.join("|"))])?;
        Ok(())
    }

    pub fn files(&self, hash: &str) -> Result<Vec<TorrentFile>> {
        Ok(serde_json::from_str(&self.get("torrents/files", &[("hash", hash)])?)?)
    }
    pub fn set_file_priority(&self, hash: &str, ids: &[i64], priority: i64) -> Result<()> {
        if ids.is_empty() { return Ok(()); }
        let id = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("|");
        self.post("torrents/filePrio", &[("hash", hash), ("id", &id), ("priority", &priority.to_string())])?;
        Ok(())
    }
    pub fn set_upload_limit(&self, hashes: &[String], bytes_per_sec: i64) -> Result<()> {
        self.post("torrents/setUploadLimit", &[("hashes", &hashes.join("|")), ("limit", &bytes_per_sec.to_string())])?;
        Ok(())
    }
    /// ratio -2 = unlimited, 0 = stop as soon as complete. Action: "Default" | "Stop" | "Remove".
    pub fn set_share_limits(&self, hashes: &[String], ratio: f64, action: &str) -> Result<()> {
        self.post("torrents/setShareLimits", &[
            ("hashes", &hashes.join("|")), ("ratioLimit", &ratio.to_string()),
            ("seedingTimeLimit", "-2"), ("inactiveSeedingTimeLimit", "-2"),
            ("shareLimitAction", action), ("shareLimitsMode", "Default"),
        ])?;
        Ok(())
    }
    pub fn add_tags(&self, hashes: &[String], tags: &str) -> Result<()> {
        self.post("torrents/addTags", &[("hashes", &hashes.join("|")), ("tags", tags)])?;
        Ok(())
    }
    pub fn remove_tags(&self, hashes: &[String], tags: &str) -> Result<()> {
        self.post("torrents/removeTags", &[("hashes", &hashes.join("|")), ("tags", tags)])?;
        Ok(())
    }
    /// Allow seeding: lift the upload cap, no share limit, tag it so reconcile leaves it alone.
    pub fn allow_seeding(&self, hashes: &[String]) -> Result<()> {
        self.set_upload_limit(hashes, 0)?;
        self.set_share_limits(hashes, -2.0, "Default")?;
        self.add_tags(hashes, SEED_TAG)
    }
    /// Cap upload, stop when complete, and stop right away if it is only seeding.
    pub fn deny_seeding(&self, hashes: &[String]) -> Result<()> {
        self.set_upload_limit(hashes, NO_UPLOAD_LIMIT)?;
        self.set_share_limits(hashes, 0.0, "Stop")?;
        self.remove_tags(hashes, SEED_TAG)?;
        let seeding: Vec<String> = self.torrents()?.into_iter()
            .filter(|t| hashes.contains(&t.hash) && t.is_seeding()).map(|t| t.hash).collect();
        if !seeding.is_empty() { self.stop(&seeding)?; }
        Ok(())
    }

    /// Turn sequential + first/last on for every torrent still downloading that lacks them.
    /// Returns the number of torrents touched.
    pub fn reconcile(&self, sequential: bool, first_last: bool, seed_by_default: bool, recent_secs: i64) -> Result<usize> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
        let list: Vec<Torrent> = self.torrents()?.into_iter()
            .filter(|t| recent_secs <= 0 || now - t.added_on <= recent_secs)
            .collect();
        let need_seq: Vec<String> = list.iter()
            .filter(|t| sequential && !t.seq_dl && t.progress < 1.0)
            .map(|t| t.hash.clone()).collect();
        let need_fl: Vec<String> = list.iter()
            .filter(|t| first_last && !t.f_l_piece_prio && t.progress < 1.0)
            .map(|t| t.hash.clone()).collect();
        // Torrents that arrived without going through us (RSS rules, WebUI) still upload freely.
        let need_noup: Vec<String> = list.iter()
            .filter(|t| !seed_by_default && !t.has_tag(SEED_TAG) && t.up_limit != NO_UPLOAD_LIMIT)
            .map(|t| t.hash.clone()).collect();
        if !need_seq.is_empty() { self.toggle_sequential(&need_seq)?; }
        if !need_fl.is_empty() { self.toggle_first_last(&need_fl)?; }
        if !need_noup.is_empty() { self.deny_seeding(&need_noup)?; }
        let mut touched: Vec<&String> = need_seq.iter().chain(need_fl.iter()).chain(need_noup.iter()).collect();
        touched.sort();
        touched.dedup();
        Ok(touched.len())
    }

    // ---- RSS ----
    pub fn rss_add_feed(&self, url: &str, name: Option<&str>) -> Result<()> {
        if self.cfg.native() { return crate::rss::add(url, name, self.cfg.timeout_secs); }
        let mut form = vec![("url", url)];
        if let Some(n) = name { form.push(("path", n)); }
        self.post("rss/addFeed", &form)?;
        Ok(())
    }
    pub fn rss_remove(&self, path: &str) -> Result<()> {
        if self.cfg.native() { return crate::rss::remove(path); }
        self.post("rss/removeItem", &[("path", path)])?;
        Ok(())
    }
    pub fn rss_refresh(&self, path: &str) -> Result<()> {
        if self.cfg.native() { return crate::rss::refresh(path, self.cfg.timeout_secs); }
        self.post("rss/refreshItem", &[("itemPath", path)])?;
        Ok(())
    }
    pub fn rss_mark_read(&self, path: &str, article_id: Option<&str>) -> Result<()> {
        if self.cfg.native() { return crate::rss::mark_read(path, article_id); }
        let mut form = vec![("itemPath", path)];
        if let Some(a) = article_id { form.push(("articleId", a)); }
        self.post("rss/markAsRead", &form)?;
        Ok(())
    }
    pub fn rss_set_read(&self, path: &str, article_id: &str, read: bool) -> Result<()> {
        if self.cfg.native() { return crate::rss::set_read(path, article_id, read); }
        if !read { anyhow::bail!("marking RSS unread requires the native engine"); }
        self.rss_mark_read(path, Some(article_id))
    }
    /// Flattened list of feeds using qBittorrent's backslash folder separator.
    pub fn rss_feeds(&self) -> Result<Vec<RssFeed>> {
        if self.cfg.native() { return crate::rss::list(); }
        let body = self.get("rss/items", &[("withData", "true")])?;
        let root: serde_json::Value = serde_json::from_str(&body)?;
        let mut out = Vec::new();
        walk_rss(&root, "", &mut out);
        Ok(out)
    }

    // ---- search ----
    pub fn search_plugins(&self) -> Result<Vec<SearchPlugin>> {
        if self.cfg.native() { return crate::search::plugins(); }
        Ok(serde_json::from_str(&self.get("search/plugins", &[])?)?)
    }
    pub fn search_enable_all(&self) -> Result<()> {
        let names: Vec<String> = self.search_plugins()?.into_iter().map(|p| p.name).collect();
        if names.is_empty() { return Ok(()); }
        if self.cfg.native() { for name in names { crate::search::set_enabled(&name, true)?; } return Ok(()); }
        self.post("search/enablePlugin", &[("names", &names.join("|")), ("enable", "true")])?;
        Ok(())
    }
    /// Install search plugins from URLs (qBittorrent fetches them itself).
    pub fn search_install_plugins(&self, sources: &[String]) -> Result<()> {
        if self.cfg.native() { return crate::search::install(sources, self.cfg.timeout_secs); }
        self.post("search/installPlugin", &[("sources", &sources.join("|"))])?;
        Ok(())
    }
    pub fn search_set_plugin(&self, name: &str, enable: bool) -> Result<()> {
        if self.cfg.native() { return crate::search::set_enabled(name, enable); }
        self.post("search/enablePlugin", &[("names", name), ("enable", bool_s(enable))])?;
        Ok(())
    }
    pub fn search_start(&self, pattern: &str) -> Result<i64> {
        if self.cfg.native() { return crate::search::start(pattern); }
        let body = self.post("search/start", &[("pattern", pattern), ("plugins", "enabled"), ("category", "all")])?;
        let v: serde_json::Value = serde_json::from_str(&body)?;
        v.get("id").and_then(|i| i.as_i64()).ok_or_else(|| anyhow!("no search id in {body}"))
    }
    pub fn search_results(&self, id: i64, limit: i64, offset: i64) -> Result<SearchResponse> {
        if self.cfg.native() { return crate::search::results(id, limit, offset); }
        let body = self.get("search/results", &[("id", &id.to_string()), ("limit", &limit.to_string()), ("offset", &offset.to_string())])?;
        Ok(serde_json::from_str(&body)?)
    }
    pub fn search_stop(&self, id: i64) -> Result<()> {
        if self.cfg.native() { return crate::search::stop(id); }
        self.post("search/stop", &[("id", &id.to_string())])?;
        Ok(())
    }
    pub fn search_delete(&self, id: i64) -> Result<()> {
        if self.cfg.native() { return crate::search::delete(id); }
        self.post("search/delete", &[("id", &id.to_string())])?;
        Ok(())
    }
}

fn walk_rss(node: &serde_json::Value, prefix: &str, out: &mut Vec<RssFeed>) {
    let Some(obj) = node.as_object() else { return };
    for (name, v) in obj {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}\\{name}") };
        if v.get("uid").is_some() {
            let mut articles = v.get("articles")
                .and_then(|a| serde_json::from_value::<Vec<RssArticle>>(a.clone()).ok())
                .unwrap_or_default();
            for a in articles.iter_mut() { a.ts = parse_rfc2822(&a.date); }
            articles.sort_by(|a, b| b.ts.cmp(&a.ts));
            out.push(RssFeed {
                name: path,
                uid: v.get("uid").and_then(|x| x.as_str()).unwrap_or("").into(),
                url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").into(),
                title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").into(),
                last_build_date: v.get("lastBuildDate").and_then(|x| x.as_str()).unwrap_or("").into(),
                is_loading: v.get("isLoading").and_then(|x| x.as_bool()).unwrap_or(false),
                has_error: v.get("hasError").and_then(|x| x.as_bool()).unwrap_or(false),
                articles,
            });
        } else {
            walk_rss(v, &path, out);
        }
    }
}

/// qBittorrent < 5.1 answers "Ok."; newer versions return a JSON summary.
fn add_response_ok(body: &str) -> Result<()> {
    let b = body.trim();
    if b == "Ok." { return Ok(()); }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(b) {
        let ok = v.get("success_count").and_then(|n| n.as_i64()).unwrap_or(0)
            + v.get("pending_count").and_then(|n| n.as_i64()).unwrap_or(0);
        let failed = v.get("failure_count").and_then(|n| n.as_i64()).unwrap_or(0);
        if ok > 0 && failed == 0 { return Ok(()); }
        if failed > 0 { bail!("qBittorrent could not add the torrent (already present or invalid)"); }
    }
    bail!("qBittorrent rejected the torrent: {b}")
}

fn check(status: u16, body: &str, path: &str) -> Result<()> {
    match status {
        200 => Ok(()),
        403 => bail!("qBittorrent refused {path} (403). Set username/password in config or disable LocalHostAuth."),
        404 => bail!("qBittorrent has no endpoint {path} (404). Is this qBittorrent 5.x?"),
        409 => bail!("qBittorrent conflict on {path}: {}", body.trim()),
        s => bail!("qBittorrent {path} returned HTTP {s}: {}", body.trim()),
    }
}

fn bool_s(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

pub fn encode_query(q: &[(&str, &str)]) -> String {
    q.iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
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

/// What `open` should launch for a torrent: the file itself for single-file
/// torrents, the folder otherwise. Falls back to the save path.
pub fn open_target(t: &Torrent) -> String {
    let p = std::path::Path::new(&t.content_path);
    if !t.content_path.is_empty() && p.exists() { return t.content_path.clone(); }
    t.save_path.clone()
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
