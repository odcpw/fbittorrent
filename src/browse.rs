//! Provider browser: trending / top lists straight from the few sites that expose JSON.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::time::Duration;

use crate::qbt::{urlencode, SearchResult};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/128 Safari/537.36";
const TRACKERS: &str = "&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce";

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Open this URL in the browser.
    Site(String),
    /// apibay precompiled top-100 for a category id.
    PirateBayTop(u16),
    /// EZTV latest episodes, sorted by seeds.
    EztvLatest,
    /// YTS list_movies with a sort key.
    Yts(&'static str),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub provider: &'static str,
    pub label: &'static str,
    pub source: Source,
}

pub fn catalogue() -> Vec<Entry> {
    let e = |provider, label, source| Entry { provider, label, source };
    vec![
        e("1337x", "Trending", Source::Site("https://1337x.to/trending".into())),
        e("1337x", "Top 100 · Movies", Source::Site("https://1337x.to/top-100-movies".into())),
        e("1337x", "Top 100 · TV", Source::Site("https://1337x.to/top-100-television".into())),
        e("YTS", "Latest movies", Source::Yts("date_added")),
        e("YTS", "Open site", Source::Site("https://yts.mx/browse-movies".into())),
        e("EZTV", "Latest TV episodes · by seeds", Source::EztvLatest),
        e("EZTV", "Open site", Source::Site("https://eztvx.to/".into())),
        e("The Pirate Bay", "Top 100 · Movies (HD)", Source::PirateBayTop(207)),
        e("The Pirate Bay", "Top 100 · TV (HD)", Source::PirateBayTop(208)),
        e("The Pirate Bay", "Open site", Source::Site("https://thepiratebay.org/search.php?q=top100:all".into())),
    ]
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(UA)
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T> {
    let mut resp = agent().get(url).call().with_context(|| format!("GET {url}"))?;
    let code = resp.status().as_u16();
    if code != 200 { return Err(anyhow!("{url}: HTTP {code}")); }
    let body = resp.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string()?;
    serde_json::from_str(&body).with_context(|| format!("parsing {url}"))
}

pub fn fetch(source: &Source) -> Result<Vec<SearchResult>> {
    match source {
        Source::Site(_) => Ok(vec![]),
        Source::PirateBayTop(cat) => piratebay_top(*cat),
        Source::EztvLatest => eztv_latest(),
        Source::Yts(sort) => yts(sort),
    }
}

// ---- The Pirate Bay (apibay) ----

#[derive(Deserialize)]
struct TpbItem {
    id: serde_json::Value,
    info_hash: String,
    name: String,
    #[serde(default)] size: i64,
    #[serde(default)] seeders: i64,
    #[serde(default)] leechers: i64,
    #[serde(default)] imdb: Option<String>,
    #[serde(default)] added: i64,
}

fn piratebay_top(cat: u16) -> Result<Vec<SearchResult>> {
    let items: Vec<TpbItem> = get_json(&format!("https://apibay.org/precompiled/data_top100_{cat}.json"))?;
    Ok(items.into_iter().map(|i| SearchResult {
        file_url: format!("magnet:?xt=urn:btih:{}&dn={}{TRACKERS}", i.info_hash.to_lowercase(), urlencode(&i.name)),
        descr_link: format!("https://thepiratebay.org/description.php?id={}", i.id),
        file_name: i.name,
        file_size: i.size,
        seeders: i.seeders,
        leechers: i.leechers,
        site_url: "https://thepiratebay.org".into(),
        engine: "piratebay".into(),
        imdb: i.imdb.unwrap_or_default(),
        pub_date: i.added,
    }).collect())
}

// ---- EZTV ----

#[derive(Deserialize)]
struct EztvResp { #[serde(default)] torrents: Vec<EztvItem> }
#[derive(Deserialize)]
struct EztvItem {
    title: String,
    magnet_url: String,
    #[serde(default)] seeds: i64,
    #[serde(default)] peers: i64,
    #[serde(default)] size_bytes: serde_json::Value,
    #[serde(default)] imdb_id: Option<String>,
    #[serde(default)] episode_url: Option<String>,
    #[serde(default)] date_released_unix: i64,
}

fn eztv_latest() -> Result<Vec<SearchResult>> {
    let r: EztvResp = get_json("https://eztv.re/api/get-torrents?limit=100&page=1")?;
    let mut out: Vec<SearchResult> = r.torrents.into_iter().map(|t| SearchResult {
        file_name: t.title,
        file_url: t.magnet_url,
        file_size: t.size_bytes.as_i64().or_else(|| t.size_bytes.as_str().and_then(|s| s.parse().ok())).unwrap_or(0),
        seeders: t.seeds,
        leechers: t.peers,
        site_url: "https://eztvx.to".into(),
        descr_link: t.episode_url.unwrap_or_default(),
        engine: "eztv".into(),
        imdb: match t.imdb_id.as_deref() { Some(id) if !id.is_empty() && id != "0" => format!("tt{id}"), _ => String::new() },
        pub_date: t.date_released_unix,
    }).collect();
    out.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(out)
}

// ---- YTS ----

#[derive(Deserialize)]
struct YtsResp { data: Option<YtsData> }
#[derive(Deserialize)]
struct YtsData { #[serde(default)] movies: Vec<YtsMovie> }
#[derive(Deserialize)]
struct YtsMovie {
    title_long: String,
    #[serde(default)] imdb_code: Option<String>,
    #[serde(default)] rating: f64,
    #[serde(default)] url: Option<String>,
    #[serde(default)] date_uploaded_unix: i64,
    #[serde(default)] torrents: Vec<YtsTorrent>,
}
#[derive(Deserialize)]
struct YtsTorrent {
    hash: String,
    #[serde(default)] quality: String,
    #[serde(default)] size_bytes: i64,
    #[serde(default)] seeds: i64,
    #[serde(default)] peers: i64,
}

const YTS_HOSTS: [&str; 3] = ["yts.mx", "yts.lt", "yts.am"];

fn yts(sort: &str) -> Result<Vec<SearchResult>> {
    let mut last = anyhow!("YTS unreachable");
    for host in YTS_HOSTS {
        let url = format!("https://{host}/api/v2/list_movies.json?sort_by={sort}&limit=50");
        match get_json::<YtsResp>(&url) {
            Ok(r) => {
                let movies = r.data.map(|d| d.movies).unwrap_or_default();
                return Ok(movies.into_iter().filter_map(|m| {
                    // Prefer 1080p, then the largest available quality.
                    let t = m.torrents.iter().find(|t| t.quality == "1080p")
                        .or_else(|| m.torrents.iter().max_by_key(|t| t.size_bytes))?;
                    let name = format!("{} [{}] [YTS] ★{:.1}", m.title_long, t.quality, m.rating);
                    Some(SearchResult {
                        file_url: format!("magnet:?xt=urn:btih:{}&dn={}{TRACKERS}", t.hash.to_lowercase(), urlencode(&name)),
                        file_name: name,
                        file_size: t.size_bytes,
                        seeders: t.seeds,
                        leechers: t.peers,
                        site_url: format!("https://{host}"),
                        descr_link: m.url.clone().unwrap_or_default(),
                        engine: "yts".into(),
                        imdb: m.imdb_code.clone().unwrap_or_default(),
                        pub_date: m.date_uploaded_unix,
                    })
                }).collect());
            }
            Err(e) => last = e,
        }
    }
    Err(anyhow!("{last:#}. YTS is not reachable from this network; use 'Open site'."))
}
