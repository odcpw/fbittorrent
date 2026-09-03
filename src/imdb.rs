//! IMDb lookups without an API key:
//!  - title search + poster: IMDb's own suggestion endpoint
//!  - rating + votes: the official daily ratings dataset, cached locally
//!  - blurb: Wikipedia REST summary

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) fbittorrent/0.1";
const RATINGS_URL: &str = "https://datasets.imdbws.com/title.ratings.tsv.gz";
const RATINGS_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Title {
    pub id: String,
    pub name: String,
    pub year: Option<u16>,
    pub kind: String,      // "feature", "tvSeries", ...
    pub stars: String,     // top-billed cast, from the suggestion endpoint
    pub poster_url: String,
    pub rating: Option<f32>,
    pub votes: Option<u32>,
    pub blurb: String,
    pub wiki_url: String,
}

impl Title {
    pub fn url(&self) -> String { format!("https://www.imdb.com/title/{}/", self.id) }
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("fbittorrent")
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .user_agent(UA)
        .http_status_as_error(false)
        .build()
        .into()
}

// ---------- suggestion endpoint ----------

#[derive(Deserialize)]
struct Sugg { d: Option<Vec<SuggItem>> }
#[derive(Deserialize)]
struct SuggItem {
    id: String,
    l: Option<String>,
    y: Option<u16>,
    q: Option<String>,
    s: Option<String>,
    i: Option<SuggImage>,
}
#[derive(Deserialize)]
struct SuggImage { #[serde(rename = "imageUrl")] image_url: Option<String> }

/// Search IMDb titles. Returns up to ~8 candidates, best first.
pub fn search(query: &str, year: Option<u16>) -> Result<Vec<Title>> {
    let q = query.trim().to_ascii_lowercase().replace(' ', "_");
    if q.is_empty() { return Ok(vec![]); }
    let first = q.chars().next().unwrap();
    let url = format!("https://v3.sg.media-imdb.com/suggestion/{first}/{}.json", crate::qbt::urlencode(&q));
    let mut resp = agent(8).get(&url).call().context("IMDb suggestion request")?;
    if resp.status().as_u16() != 200 { return Err(anyhow!("IMDb suggestion HTTP {}", resp.status())); }
    let body = resp.body_mut().read_to_string()?;
    let s: Sugg = serde_json::from_str(&body).context("IMDb suggestion JSON")?;
    let mut out: Vec<Title> = s.d.unwrap_or_default().into_iter()
        .filter(|i| i.id.starts_with("tt"))
        .map(|i| Title {
            id: i.id,
            name: i.l.unwrap_or_default(),
            year: i.y,
            kind: i.q.unwrap_or_default(),
            stars: i.s.unwrap_or_default(),
            poster_url: i.i.and_then(|p| p.image_url).unwrap_or_default(),
            ..Default::default()
        })
        .collect();
    if let Some(y) = year {
        // Stable partition: year matches first, IMDb's own order otherwise.
        let (hit, miss): (Vec<_>, Vec<_>) = out.into_iter().partition(|t| t.year == Some(y));
        out = hit; out.extend(miss);
    }
    Ok(out)
}

/// Poster URL resized server-side (IMDb image CDN understands `_V1_UX<width>_`).
pub fn poster_url_sized(url: &str, width: u32) -> String {
    match url.rfind("._V1_") {
        Some(i) => format!("{}._V1_UX{width}_.jpg", &url[..i]),
        None => url.to_string(),
    }
}

pub fn fetch_poster(url: &str, width: u32) -> Result<PathBuf> {
    let dir = cache_dir().join("posters");
    std::fs::create_dir_all(&dir)?;
    let key = url.rsplit('/').next().unwrap_or("poster").split('.').next().unwrap_or("poster");
    let path = dir.join(format!("{key}_{width}.jpg"));
    if path.exists() { return Ok(path); }
    let sized = poster_url_sized(url, width);
    let mut resp = agent(10).get(&sized).call().context("poster download")?;
    if resp.status().as_u16() != 200 { return Err(anyhow!("poster HTTP {}", resp.status())); }
    let bytes = resp.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec()?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

// ---------- ratings dataset ----------

pub fn ratings_path() -> PathBuf { cache_dir().join("title.ratings.tsv") }

pub fn ratings_stale() -> bool {
    match std::fs::metadata(ratings_path()).and_then(|m| m.modified()) {
        Ok(t) => SystemTime::now().duration_since(t).map(|d| d > RATINGS_MAX_AGE).unwrap_or(true),
        Err(_) => true,
    }
}

/// Download and unpack the official ratings dataset (about 9 MB gzipped).
static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn refresh_ratings() -> Result<()> {
    // Several background tasks may notice staleness at once (index loader, hover
    // lookups); only one downloads, the others find a fresh file when they get the lock.
    let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !ratings_stale() { return Ok(()); }
    std::fs::create_dir_all(cache_dir())?;
    let mut resp = agent(120).get(RATINGS_URL).call().context("ratings dataset download")?;
    if resp.status().as_u16() != 200 { return Err(anyhow!("ratings dataset HTTP {}", resp.status())); }
    let gz = resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec()?;
    let mut tsv = Vec::with_capacity(gz.len() * 3);
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut tsv)?;
    let tmp = ratings_path().with_extension("tmp");
    std::fs::write(&tmp, &tsv)?;
    std::fs::rename(&tmp, ratings_path())?;
    Ok(())
}

/// Whole ratings dataset in memory: ~1.6M titles, 10 bytes each, binary-searched.
pub struct RatingsIndex {
    ids: Vec<u32>,
    ratings: Vec<u16>, // rating * 10
    votes: Vec<u32>,
}

impl RatingsIndex {
    pub fn load() -> Result<Self> {
        let data = std::fs::read(ratings_path()).context("ratings dataset not downloaded yet")?;
        let mut rows: Vec<(u32, u16, u32)> = Vec::with_capacity(1_700_000);
        for line in data.split(|&b| b == b'\n').skip(1) {
            let mut it = line.split(|&b| b == b'\t');
            let (Some(id), Some(r), Some(v)) = (it.next(), it.next(), it.next()) else { continue };
            let Some(digits) = id.strip_prefix(b"tt") else { continue };
            let Ok(id) = std::str::from_utf8(digits).unwrap_or("x").parse::<u32>() else { continue };
            let rating = std::str::from_utf8(r).ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
            let votes = std::str::from_utf8(v).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
            rows.push((id, (rating * 10.0).round() as u16, votes));
        }
        rows.sort_unstable_by_key(|r| r.0);
        Ok(Self {
            ids: rows.iter().map(|r| r.0).collect(),
            ratings: rows.iter().map(|r| r.1).collect(),
            votes: rows.iter().map(|r| r.2).collect(),
        })
    }


    pub fn get(&self, imdb_id: &str) -> Option<(f32, u32)> {
        let id: u32 = imdb_id.trim().trim_start_matches("tt").parse().ok()?;
        let i = self.ids.binary_search(&id).ok()?;
        Some((self.ratings[i] as f32 / 10.0, self.votes[i]))
    }
}

/// Look one title up in the cached dataset. Ok(None) when unknown or no cache yet.
pub fn rating_for(id: &str) -> Result<Option<(f32, u32)>> {
    let Ok(data) = std::fs::read(ratings_path()) else { return Ok(None) };
    Ok(find_rating(&data, id))
}

pub fn find_rating(data: &[u8], id: &str) -> Option<(f32, u32)> {
    let needle = format!("\n{id}\t");
    let pos = memfind(data, needle.as_bytes())?;
    let start = pos + 1;
    let end = data[start..].iter().position(|&b| b == b'\n').map(|e| start + e).unwrap_or(data.len());
    let line = std::str::from_utf8(&data[start..end]).ok()?;
    let mut it = line.split('\t');
    it.next()?;
    let rating: f32 = it.next()?.parse().ok()?;
    let votes: u32 = it.next()?.trim().parse().ok()?;
    Some((rating, votes))
}

fn memfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() { return None; }
    let first = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        match hay[i..].iter().position(|&b| b == first) {
            None => return None,
            Some(off) => {
                let s = i + off;
                if s + needle.len() <= hay.len() && &hay[s..s + needle.len()] == needle { return Some(s); }
                i = s + 1;
            }
        }
    }
    None
}

// ---------- Wikipedia blurb ----------

#[derive(Deserialize)]
struct WikiSearch { query: Option<WikiQuery> }
#[derive(Deserialize)]
struct WikiQuery { search: Vec<WikiHit> }
#[derive(Deserialize)]
struct WikiHit { title: String }
#[derive(Deserialize)]
struct WikiSummary {
    extract: Option<String>,
    content_urls: Option<WikiUrls>,
}
#[derive(Deserialize)]
struct WikiUrls { desktop: Option<WikiDesktop> }
#[derive(Deserialize)]
struct WikiDesktop { page: Option<String> }

pub fn wiki_blurb(t: &Title) -> Result<(String, String)> {
    let kind = if t.kind.to_ascii_lowercase().starts_with("tv") { "TV series" } else { "film" };
    let q = match t.year { Some(y) => format!("{} {y} {kind}", t.name), None => format!("{} {kind}", t.name) };
    let url = format!("https://en.wikipedia.org/w/api.php?action=query&list=search&format=json&srlimit=1&srsearch={}", crate::qbt::urlencode(&q));
    let mut resp = agent(8).get(&url).call().context("Wikipedia search")?;
    let body = resp.body_mut().read_to_string()?;
    let s: WikiSearch = serde_json::from_str(&body).context("Wikipedia search JSON")?;
    let Some(hit) = s.query.and_then(|q| q.search.into_iter().next()) else { return Ok((String::new(), String::new())) };
    let url = format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", crate::qbt::urlencode(&hit.title.replace(' ', "_")));
    let mut resp = agent(8).get(&url).call().context("Wikipedia summary")?;
    let body = resp.body_mut().read_to_string()?;
    let sum: WikiSummary = serde_json::from_str(&body).context("Wikipedia summary JSON")?;
    let page = sum.content_urls.and_then(|c| c.desktop).and_then(|d| d.page).unwrap_or_default();
    Ok((sum.extract.unwrap_or_default(), page))
}

// ---------- disk cache ----------
// by-name/<key>.json  -> {"id": "tt…"} or {"missing": true}   (release name -> id)
// by-id/<tt…>.json    -> Title + fetched_at                    (full card)

const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

fn cache_file(kind: &str, key: &str) -> PathBuf {
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).take(120).collect();
    cache_dir().join("imdb").join(kind).join(format!("{safe}.json"))
}

fn cache_read(kind: &str, key: &str) -> Option<serde_json::Value> {
    let path = cache_file(kind, key);
    let meta = std::fs::metadata(&path).ok()?;
    let age = SystemTime::now().duration_since(meta.modified().ok()?).ok()?;
    if age > CACHE_TTL { return None; }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn cache_write(kind: &str, key: &str, v: &serde_json::Value) {
    let path = cache_file(kind, key);
    if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(path, v.to_string());
}

fn cached_title(id: &str) -> Option<Title> {
    cache_read("by-id", id).and_then(|v| serde_json::from_value(v).ok())
}

fn store_title(t: &Title) {
    if let Ok(v) = serde_json::to_value(t) { cache_write("by-id", &t.id, &v); }
}

/// Lookup by IMDb id (the suggestion endpoint resolves "tt…" queries directly).
pub fn lookup_id(id: &str) -> Result<Option<Title>> {
    if let Some(t) = cached_title(id) { return Ok(Some(t)); }
    let mut cands = search(id, None)?;
    let Some(pos) = cands.iter().position(|t| t.id == id) else { return Ok(None) };
    let mut t = cands.remove(pos);
    enrich(&mut t);
    store_title(&t);
    Ok(Some(t))
}

/// Cheap id resolution for a release name: one suggestion call, cached by name.
/// No rating, blurb or poster fetch. Used to fill the ratings column of search results.
pub fn resolve_id(release_name: &str) -> Option<String> {
    if let Some(v) = cache_read("by-name", release_name) {
        if v.get("missing").and_then(|m| m.as_bool()) == Some(true) { return None; }
        return v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
    }
    let r = crate::release::parse(release_name);
    if r.title.is_empty() { return None; }
    let mut cands = candidates(&r).ok()?;
    if cands.is_empty() {
        cache_write("by-name", release_name, &serde_json::json!({"missing": true}));
        return None;
    }
    let t = cands.remove(0);
    cache_write("by-name", release_name, &serde_json::json!({"id": t.id}));
    Some(t.id)
}

/// IMDb suggestions for a parsed release. An episode number means a show, so TV
/// titles go first: "The Ghost in the Shell S01E09" must not land on a film.
fn candidates(r: &crate::release::Release) -> Result<Vec<Title>> {
    let mut out = search(&r.title, r.year)?;
    if r.episode.is_some() {
        let (tv, rest): (Vec<_>, Vec<_>) = out.into_iter().partition(|t| t.kind.to_ascii_lowercase().starts_with("tv"));
        out = tv; out.extend(rest);
    }
    Ok(out)
}

/// Best-effort full lookup for a release name: title + rating + blurb.
/// Cached on disk for a week, by name and by id, so the same show under
/// several release names costs one network round trip.
pub fn lookup(release_name: &str) -> Result<Option<Title>> {
    if let Some(v) = cache_read("by-name", release_name) {
        if v.get("missing").and_then(|m| m.as_bool()) == Some(true) { return Ok(None); }
        if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
            if let Some(t) = cached_title(id) { return Ok(Some(t)); }
            return lookup_id(id);
        }
    }
    let r = crate::release::parse(release_name);
    if r.title.is_empty() { return Ok(None); }
    let mut cands = candidates(&r)?;
    if cands.is_empty() {
        cache_write("by-name", release_name, &serde_json::json!({"missing": true}));
        return Ok(None);
    }
    let mut t = cands.remove(0);
    cache_write("by-name", release_name, &serde_json::json!({"id": t.id}));
    if let Some(full) = cached_title(&t.id) { return Ok(Some(full)); }
    enrich(&mut t);
    store_title(&t);
    Ok(Some(t))
}

pub fn enrich(t: &mut Title) {
    if let Ok(Some((r, v))) = rating_for(&t.id) { t.rating = Some(r); t.votes = Some(v); }
    if let Ok((blurb, url)) = wiki_blurb(t) { t.blurb = blurb; t.wiki_url = url; }
}
