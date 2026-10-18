//! App-owned RSS/Atom subscriptions and read history.
//!
//! Listing only reads the local cache. Network requests happen on an explicit
//! refresh; there are no timers or workers outside the running application.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::qbt::{parse_rfc2822, RssArticle, RssFeed};

const MAX_BODY: u64 = 8 * 1024 * 1024;
const MAX_CACHED_ARTICLES: usize = 500;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Database {
    version: u32,
    feeds: Vec<StoredFeed>,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct StoredFeed {
    feed: RssFeed,
    /// Unlike cached articles, these identifiers are never aged out.
    read_ids: BTreeSet<String>,
    last_checked: i64,
    etag: String,
    last_modified: String,
}

impl Default for StoredFeed {
    fn default() -> Self {
        Self {
            feed: RssFeed::default(), read_ids: BTreeSet::new(), last_checked: 0,
            etag: String::new(), last_modified: String::new(),
        }
    }
}

struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        // The separate lock file is never renamed with the data file.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN); }
    }
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl Default for Store {
    fn default() -> Self { Self::new(path()) }
}

pub fn path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("fbittorrent/rss.json")
}

pub fn list() -> Result<Vec<RssFeed>> { Store::default().list() }
pub fn add(url: &str, name: Option<&str>, timeout_secs: u64) -> Result<()> {
    Store::default().add(url, name, timeout_secs)
}
pub fn remove(path: &str) -> Result<()> { Store::default().remove(path) }
pub fn refresh(path: &str, timeout_secs: u64) -> Result<()> {
    Store::default().refresh(path, timeout_secs)
}
pub fn mark_read(path: &str, article_id: Option<&str>) -> Result<()> {
    Store::default().mark_read(path, article_id)
}
pub fn set_read(path: &str, article_id: &str, read: bool) -> Result<()> {
    Store::default().set_read(path, Some(article_id), read)
}
/// Merge qBittorrent's subscriptions/cache once during migration. Read states
/// are combined, so retrying an import never makes a read item new again.
pub fn import(feeds: &[RssFeed]) -> Result<usize> { Store::default().import(feeds) }

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self { Self { path: path.into() } }

    fn lock(&self) -> Result<FileLock> {
        let parent = self.path.parent().context("RSS cache has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .mode(0o600).open(self.path.with_extension("lock"))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(FileLock(file));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted { return Err(error.into()); }
        }
    }

    fn load(&self) -> Result<Database> {
        let mut data: Database = match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing RSS cache {}", self.path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Database::default(),
            Err(error) => return Err(error).context("reading RSS cache"),
        };
        for stored in &mut data.feeds {
            stored.feed.is_loading = false;
            for article in &mut stored.feed.articles {
                article.ts = parse_date(&article.date);
                if article.is_read { stored.read_ids.insert(article.id.clone()); }
                article.is_read = stored.read_ids.contains(&article.id);
            }
            sort_articles(&mut stored.feed.articles);
        }
        Ok(data)
    }

    fn save(&self, data: &Database) -> Result<()> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let tmp = self.path.with_extension(format!("{}.{}.tmp", std::process::id(), suffix));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            serde_json::to_writer(&mut file, data)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            if let Some(parent) = self.path.parent() { File::open(parent)?.sync_all()?; }
            Ok(())
        })();
        if result.is_err() { let _ = std::fs::remove_file(tmp); }
        result.context("saving RSS subscriptions and read history")
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut Database) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        let mut data = self.load()?;
        data.version = 1;
        let result = change(&mut data)?;
        self.save(&data)?;
        Ok(result)
    }

    pub fn list(&self) -> Result<Vec<RssFeed>> {
        // An atomic rename makes unlocked reads safe, including during refresh.
        Ok(self.load()?.feeds.into_iter().map(|stored| stored.feed).collect())
    }

    pub fn add(&self, url: &str, name: Option<&str>, timeout_secs: u64) -> Result<()> {
        validate_url(url)?;
        let chosen = self.mutate(|data| {
            if data.feeds.iter().any(|stored| stored.feed.url == url) {
                bail!("RSS feed is already followed: {url}");
            }
            let mut chosen = name.map(str::trim).filter(|name| !name.is_empty())
                .map(str::to_owned).unwrap_or_else(|| host_name(url));
            if name.is_some() && data.feeds.iter().any(|stored| stored.feed.name == chosen) {
                bail!("RSS feed name already exists: {chosen}");
            }
            let base = chosen.clone();
            let mut suffix = 2;
            while data.feeds.iter().any(|stored| stored.feed.name == chosen) {
                chosen = format!("{base} ({suffix})"); suffix += 1;
            }
            data.feeds.push(StoredFeed {
                feed: RssFeed { name: chosen.clone(), uid: stable_id(url), url: url.into(), ..RssFeed::default() },
                ..StoredFeed::default()
            });
            Ok(chosen)
        })?;
        // Keep a subscription even if its source is temporarily unavailable.
        self.refresh(&chosen, timeout_secs)
    }

    pub fn remove(&self, path: &str) -> Result<()> {
        if path.is_empty() { bail!("choose an RSS feed or folder to remove"); }
        self.mutate(|data| {
            let before = data.feeds.len();
            data.feeds.retain(|stored| !matches_path(&stored.feed.name, path));
            if data.feeds.len() == before { bail!("RSS feed not found: {path}"); }
            Ok(())
        })
    }

    pub fn mark_read(&self, path: &str, article_id: Option<&str>) -> Result<()> {
        self.set_read(path, article_id, true)
    }

    pub fn set_read(&self, path: &str, article_id: Option<&str>, read: bool) -> Result<()> {
        self.mutate(|data| {
            let mut found = false;
            for stored in &mut data.feeds {
                if !matches_path(&stored.feed.name, path) { continue; }
                found = true;
                if let Some(id) = article_id {
                    // Remember identifiers even when their row has left the cache.
                    if read { stored.read_ids.insert(id.into()); }
                    else { stored.read_ids.remove(id); }
                }
                for article in &mut stored.feed.articles {
                    if article_id.is_none() || article_id == Some(article.id.as_str()) {
                        article.is_read = read;
                        if read { stored.read_ids.insert(article.id.clone()); }
                        else { stored.read_ids.remove(&article.id); }
                    }
                }
            }
            if !found { bail!("RSS feed not found: {path}"); }
            Ok(())
        })
    }

    pub fn refresh(&self, path: &str, timeout_secs: u64) -> Result<()> {
        let snapshots: Vec<StoredFeed> = self.load()?.feeds.into_iter()
            .filter(|stored| matches_path(&stored.feed.name, path)).collect();
        if snapshots.is_empty() && !path.is_empty() { bail!("RSS feed not found: {path}"); }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(timeout_secs.max(1))))
            .http_status_as_error(false).build().into();
        let mut failures = Vec::new();
        for snapshot in snapshots {
            // Fetch outside the write lock. Mark-as-read calls during a slow
            // response are folded into the result rather than overwritten.
            let fetched = fetch(&agent, &snapshot);
            self.mutate(|data| {
                let Some(stored) = data.feeds.iter_mut().find(|stored|
                    stored.feed.uid == snapshot.feed.uid && stored.feed.url == snapshot.feed.url)
                else { return Ok(()); }; // subscription removed during fetch
                stored.last_checked = now();
                stored.feed.is_loading = false;
                match &fetched {
                    Ok(Some(result)) => {
                        if !result.title.is_empty() { stored.feed.title = result.title.clone(); }
                        stored.feed.last_build_date = result.last_build_date.clone();
                        merge_articles(stored, result.articles.clone());
                        stored.etag = result.etag.clone();
                        stored.last_modified = result.last_modified.clone();
                        stored.feed.has_error = false;
                    }
                    Ok(None) => stored.feed.has_error = false,
                    Err(_) => stored.feed.has_error = true,
                }
                Ok(())
            })?;
            if let Err(error) = fetched { failures.push(format!("{}: {error:#}", snapshot.feed.name)); }
        }
        if !failures.is_empty() { bail!("RSS refresh failed: {}", failures.join("; ")); }
        Ok(())
    }

    pub fn import(&self, feeds: &[RssFeed]) -> Result<usize> {
        self.mutate(|data| {
            let mut added = 0;
            for feed in feeds {
                if feed.url.is_empty() { continue; }
                let index = match data.feeds.iter().position(|stored| stored.feed.url == feed.url) {
                    Some(index) => index,
                    None => {
                        let mut imported = feed.clone();
                        imported.is_loading = false;
                        if imported.uid.is_empty() { imported.uid = stable_id(&imported.url); }
                        let mut name = imported.name.clone();
                        let base = name.clone();
                        let mut suffix = 2;
                        while data.feeds.iter().any(|stored| stored.feed.name == name) {
                            name = format!("{base} ({suffix})"); suffix += 1;
                        }
                        imported.name = name;
                        imported.articles.clear();
                        data.feeds.push(StoredFeed { feed: imported, ..StoredFeed::default() });
                        added += 1;
                        data.feeds.len() - 1
                    }
                };
                merge_articles(&mut data.feeds[index], feed.articles.clone());
            }
            Ok(added)
        })
    }
}

fn matches_path(name: &str, path: &str) -> bool {
    path.is_empty() || name == path || name.strip_prefix(path)
        .is_some_and(|tail| tail.starts_with('\\') || tail.starts_with('/'))
}

fn merge_articles(stored: &mut StoredFeed, incoming: Vec<RssArticle>) {
    let mut articles: BTreeMap<String, RssArticle> = stored.feed.articles.drain(..)
        .map(|article| (article.id.clone(), article)).collect();
    for mut article in incoming {
        if article.id.is_empty() { article.id = article_id(&article); }
        if article.is_read { stored.read_ids.insert(article.id.clone()); }
        article.ts = parse_date(&article.date);
        article.is_read = stored.read_ids.contains(&article.id)
            || articles.get(&article.id).is_some_and(|previous| previous.is_read);
        if article.is_read { stored.read_ids.insert(article.id.clone()); }
        articles.insert(article.id.clone(), article);
    }
    stored.feed.articles = articles.into_values().collect();
    sort_articles(&mut stored.feed.articles);
    stored.feed.articles.truncate(MAX_CACHED_ARTICLES);
}

fn sort_articles(articles: &mut [RssArticle]) {
    articles.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| a.id.cmp(&b.id)));
}

fn validate_url(url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) || host_name(url).is_empty() {
        bail!("RSS feed must have an http or https URL");
    }
    Ok(())
}

fn host_name(url: &str) -> String {
    url.split_once("://").map(|(_, rest)| rest.split('/').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

fn stable_id(value: &str) -> String {
    // Stable cache key; it is not used as a cryptographic digest.
    let hash = value.bytes().fold(0xcbf29ce484222325u64, |hash, byte|
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    format!("rss-{hash:016x}")
}

fn article_id(article: &RssArticle) -> String {
    if !article.torrent_url.is_empty() { article.torrent_url.clone() }
    else if !article.link.is_empty() { article.link.clone() }
    else { stable_id(&format!("{}\n{}", article.title, article.date)) }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

struct Fetched {
    title: String,
    last_build_date: String,
    articles: Vec<RssArticle>,
    etag: String,
    last_modified: String,
}

fn fetch(agent: &ureq::Agent, stored: &StoredFeed) -> Result<Option<Fetched>> {
    validate_url(&stored.feed.url)?;
    let mut request = agent.get(&stored.feed.url)
        .header("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml")
        .header("User-Agent", concat!("fbittorrent/", env!("CARGO_PKG_VERSION")));
    if !stored.etag.is_empty() { request = request.header("If-None-Match", &stored.etag); }
    if !stored.last_modified.is_empty() { request = request.header("If-Modified-Since", &stored.last_modified); }
    let mut response = request.call().context("fetching feed")?;
    let status = response.status().as_u16();
    if status == 304 { return Ok(None); }
    if status != 200 { bail!("feed returned HTTP {status}"); }
    let etag = response.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let last_modified = response.headers().get("last-modified").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = response.body_mut().with_config().limit(MAX_BODY).read_to_vec()?;
    let mut result = parse_feed(&bytes, &stored.feed.url)?;
    result.etag = etag;
    result.last_modified = last_modified;
    Ok(Some(result))
}

fn parse_feed(bytes: &[u8], base: &str) -> Result<Fetched> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut article: Option<RssArticle> = None;
    let mut result = Fetched { title: String::new(), last_build_date: String::new(), articles: vec![], etag: String::new(), last_modified: String::new() };
    let mut known_root = false;
    loop {
        match reader.read_event().context("parsing RSS/Atom XML")? {
            Event::Start(start) => {
                let name = local_name(start.local_name().as_ref());
                if stack.is_empty() { known_root = matches!(name.as_str(), "rss" | "feed" | "rdf"); }
                if name == "item" || name == "entry" {
                    if article.is_some() { bail!("feed has nested articles"); }
                    article = Some(RssArticle::default());
                }
                parse_attributes(&start, &reader, &name, article.as_mut(), base)?;
                stack.push((name, String::new()));
            }
            Event::Empty(start) => {
                let name = local_name(start.local_name().as_ref());
                if stack.is_empty() { known_root = matches!(name.as_str(), "rss" | "feed" | "rdf"); }
                parse_attributes(&start, &reader, &name, article.as_mut(), base)?;
            }
            Event::Text(text) => {
                if let Some((_, value)) = stack.last_mut() { value.push_str(&text.xml_content()?); }
            }
            Event::CData(text) => {
                if let Some((_, value)) = stack.last_mut() { value.push_str(&text.decode()?); }
            }
            Event::GeneralRef(reference) => {
                let encoded = format!("&{};", reference.decode()?);
                if let Some((_, value)) = stack.last_mut() {
                    value.push_str(&quick_xml::escape::unescape(&encoded)?);
                }
            }
            Event::End(_) => {
                let Some((name, value)) = stack.pop() else { bail!("feed has unmatched closing tag"); };
                if name == "item" || name == "entry" {
                    let mut row = article.take().context("feed article is missing")?;
                    row.title = row.title.trim().into();
                    row.date = row.date.trim().into();
                    if row.torrent_url.is_empty() && downloadable(&row.link, "") { row.torrent_url = row.link.clone(); }
                    if row.id.is_empty() { row.id = article_id(&row); }
                    row.ts = parse_date(&row.date);
                    if !row.title.is_empty() || !row.link.is_empty() || !row.torrent_url.is_empty() { result.articles.push(row); }
                } else if let Some(row) = article.as_mut() {
                    match name.as_str() {
                        "title" => row.title = value.trim().into(),
                        "guid" | "id" => row.id = value.trim().into(),
                        "link" if !value.trim().is_empty() => row.link = resolve_url(base, value.trim()),
                        "torrenturl" | "magneturi" | "magnet" => row.torrent_url = resolve_url(base, value.trim()),
                        "pubdate" | "published" | "date" => row.date = value.trim().into(),
                        "updated" if row.date.is_empty() => row.date = value.trim().into(),
                        "description" | "summary" | "encoded" | "content" => row.description = value.trim().into(),
                        "size" => row.size = value.trim().parse().ok(),
                        _ => {},
                    }
                } else if stack.len() == 1 || (stack.len() == 2 && stack.last().is_some_and(|(name, _)| name == "channel")) {
                    match name.as_str() {
                        "title" => result.title = value.trim().into(),
                        "lastbuilddate" | "updated" | "pubdate" => result.last_build_date = value.trim().into(),
                        _ => {},
                    }
                }
                // Nested XHTML content still contributes readable text.
                if let Some((_, parent)) = stack.last_mut() { parent.push_str(&value); }
            }
            Event::Eof => break,
            _ => {},
        }
    }
    if !known_root { bail!("response is not an RSS or Atom feed"); }
    if !stack.is_empty() || article.is_some() { bail!("feed XML ended before its closing tag"); }
    sort_articles(&mut result.articles);
    Ok(result)
}

fn local_name(bytes: &[u8]) -> String { String::from_utf8_lossy(bytes).to_ascii_lowercase() }

fn parse_attributes(start: &BytesStart<'_>, reader: &Reader<&[u8]>, name: &str, row: Option<&mut RssArticle>, base: &str) -> Result<()> {
    let Some(row) = row else { return Ok(()); };
    if !matches!(name, "link" | "enclosure" | "content" | "attr") { return Ok(()); }
    let mut attributes = BTreeMap::new();
    for attribute in start.attributes() {
        let attribute = attribute?;
        attributes.insert(local_name(attribute.key.local_name().as_ref()), attribute.decode_and_unescape_value(reader.decoder())?.into_owned());
    }
    if name == "attr" {
        match attributes.get("name").map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("size") => row.size = attributes.get("value").and_then(|value| value.parse().ok()),
            Some("magneturl") => row.torrent_url = attributes.get("value").cloned().unwrap_or_default(),
            _ => {},
        }
        return Ok(());
    }
    let url = attributes.get("href").or_else(|| attributes.get("url"));
    if let Some(url) = url {
        let mime = attributes.get("type").map(String::as_str).unwrap_or("");
        let rel = attributes.get("rel").map(String::as_str).unwrap_or("alternate");
        if downloadable(url, mime) {
            row.torrent_url = resolve_url(base, url);
            row.size = attributes.get("length").and_then(|value| value.parse().ok()).or(row.size);
        } else if name == "link" && (rel == "alternate" || row.link.is_empty()) && rel != "enclosure" {
            row.link = resolve_url(base, url);
        }
    }
    Ok(())
}

fn downloadable(url: &str, mime: &str) -> bool {
    url.starts_with("magnet:") || mime.eq_ignore_ascii_case("application/x-bittorrent")
        || url.split('?').next().is_some_and(|path| path.to_ascii_lowercase().ends_with(".torrent"))
}

fn resolve_url(base: &str, value: &str) -> String {
    if value.contains("://") || value.starts_with("magnet:") || value.is_empty() { return value.into(); }
    let Some((scheme, rest)) = base.split_once("://") else { return value.into(); };
    if value.starts_with("//") { return format!("{scheme}:{value}"); }
    let host = rest.split('/').next().unwrap_or(rest);
    if value.starts_with('/') { return format!("{scheme}://{host}{value}"); }
    let clean = base.split(['?', '#']).next().unwrap_or(base);
    let directory = clean.rsplit_once('/').filter(|(prefix, _)| prefix.contains("://"))
        .map(|(prefix, _)| prefix.to_string()).unwrap_or_else(|| format!("{scheme}://{host}"));
    format!("{directory}/{value}")
}

/// RSS dates and Atom's RFC 3339 timestamps, without a second date dependency.
fn parse_date(value: &str) -> i64 {
    let value = value.trim();
    if value.len() < 19 || value.as_bytes().get(4) != Some(&b'-') { return parse_rfc2822(value); }
    let parse = |from: usize, to: usize| value.get(from..to).and_then(|v| v.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), Some(h), Some(mi), Some(s)) =
        (parse(0, 4), parse(5, 7), parse(8, 10), parse(11, 13), parse(14, 16), parse(17, 19)) else { return 0; };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 { return 0; }
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let tail = value.get(19..).unwrap_or("");
    let zone_start = tail.find(['+', '-']);
    let zone = zone_start.map(|index| tail[index..].replace(':', "")).unwrap_or_else(|| "+0000".into());
    parse_rfc2822(&format!("{d:02} {} {y:04} {h:02}:{mi:02}:{s:02} {zone}", months[(m - 1) as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn store() -> (Self, Store) {
            let dir = std::env::temp_dir().join(format!("fbittorrent-rss-test-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
            std::fs::create_dir_all(&dir).unwrap();
            let store = Store::new(dir.join("rss.json"));
            (Self(dir), store)
        }
    }
    impl Drop for Scratch { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    const RSS: &str = r#"<?xml version="1.0"?><rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
        <title>Example &amp; releases</title><lastBuildDate>Fri, 09 Oct 2026 12:00:00 +0000</lastBuildDate>
        <item><title><![CDATA[Example.S01E01.1080p]]></title><guid>episode-1</guid><link>/release/1</link>
        <pubDate>Fri, 09 Oct 2026 12:00:00 +0000</pubDate><enclosure url="/files/1.torrent" length="1234" type="application/x-bittorrent"/>
        <description>First &lt;b&gt;episode&lt;/b&gt;.</description></item>
        <item><title>Example.S01E02</title><guid>episode-2</guid><link>https://example.org/release/2</link>
        <pubDate>10 Oct 2026 12:00:00 +0000</pubDate><torznab:attr name="magneturl" value="magnet:?xt=urn:btih:abc&amp;dn=Example"/><torznab:attr name="size" value="5678"/></item>
        </channel></rss>"#;

    #[test]
    fn rss_parses_entities_cdata_torrents_and_dates() {
        let feed = parse_feed(RSS.as_bytes(), "https://example.org/rss.xml").unwrap();
        assert_eq!(feed.title, "Example & releases");
        assert_eq!(feed.articles.len(), 2);
        assert_eq!(feed.articles[0].id, "episode-2");
        assert_eq!(feed.articles[0].torrent_url, "magnet:?xt=urn:btih:abc&dn=Example");
        assert_eq!(feed.articles[0].size, Some(5678));
        assert_eq!(feed.articles[1].torrent_url, "https://example.org/files/1.torrent");
        assert_eq!(feed.articles[1].link, "https://example.org/release/1");
        assert_eq!(feed.articles[1].description, "First <b>episode</b>.");
    }

    #[test]
    fn atom_parses_links_dates_and_xhtml_titles() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom releases</title><updated>2026-10-09T12:00:00Z</updated>
            <entry><id>tag:example.org,2026:1</id><title type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">Example &amp; friends</div></title>
            <updated>2026-10-09T14:00:00+02:00</updated><link href="/release/1"/>
            <link rel="enclosure" href="/1.torrent" type="application/x-bittorrent" length="321"/><summary>Episode details</summary></entry></feed>"#;
        let feed = parse_feed(xml.as_bytes(), "https://example.org/feed").unwrap();
        assert_eq!(feed.title, "Atom releases");
        assert_eq!(feed.articles[0].title, "Example & friends");
        assert_eq!(feed.articles[0].ts, parse_rfc2822("09 Oct 2026 12:00:00 +0000"));
        assert_eq!(feed.articles[0].size, Some(321));
        assert_eq!(feed.articles[0].torrent_url, "https://example.org/1.torrent");
        assert_eq!(feed.articles[0].link, "https://example.org/release/1");
    }

    #[test]
    fn single_item_read_can_be_undone_without_marking_other_items() {
        let (_scratch, store) = Scratch::store();
        let parsed = parse_feed(RSS.as_bytes(), "https://example.org/rss").unwrap();
        store.import(&[RssFeed { name: "Example".into(), url: "https://example.org/rss".into(), articles: parsed.articles, ..Default::default() }]).unwrap();
        store.mark_read("Example", None).unwrap();
        store.set_read("Example", Some("episode-1"), false).unwrap();
        let reopened = Store::new(&store.path).list().unwrap();
        assert!(!reopened[0].articles.iter().find(|a| a.id == "episode-1").unwrap().is_read);
        assert!(reopened[0].articles.iter().find(|a| a.id == "episode-2").unwrap().is_read);
        store.set_read("Example", Some("episode-1"), true).unwrap();
        assert!(store.list().unwrap()[0].articles.iter().all(|a| a.is_read));
    }

    #[test]
    fn imported_history_is_local_and_idempotent() {
        let (_scratch, store) = Scratch::store();
        let parsed = parse_feed(RSS.as_bytes(), "https://example.org/rss").unwrap();
        let original = RssFeed { name: "TV\\Example".into(), uid: "original-uid".into(), url: "http://127.0.0.1:1/offline".into(), articles: parsed.articles, ..RssFeed::default() };
        assert_eq!(store.import(std::slice::from_ref(&original)).unwrap(), 1);
        store.mark_read(&original.name, Some("episode-1")).unwrap();
        assert_eq!(store.import(&[original]).unwrap(), 0);
        // Opening a new Store never connects to the saved feed URL.
        let reopened = Store::new(&store.path).list().unwrap();
        assert_eq!(reopened[0].uid, "original-uid");
        assert!(reopened[0].articles.iter().find(|row| row.id == "episode-1").unwrap().is_read);
        assert!(!reopened[0].articles.iter().find(|row| row.id == "episode-2").unwrap().is_read);
    }

    #[test]
    fn read_ids_survive_cache_eviction_and_returning_releases() {
        let (_scratch, store) = Scratch::store();
        let feed = RssFeed { name: "Example".into(), uid: "uid".into(), url: "https://example.org/rss".into(), articles: vec![RssArticle { id: "old".into(), title: "old episode".into(), ..RssArticle::default() }], ..RssFeed::default() };
        store.import(&[feed]).unwrap();
        store.mark_read("Example", Some("old")).unwrap();
        store.mutate(|data| { data.feeds[0].feed.articles.clear(); Ok(()) }).unwrap();
        store.mutate(|data| {
            merge_articles(&mut data.feeds[0], vec![RssArticle { id: "old".into(), title: "returning release".into(), ..RssArticle::default() }]); Ok(())
        }).unwrap();
        assert!(store.list().unwrap()[0].articles[0].is_read);
    }

    #[test]
    fn malformed_responses_preserve_the_cached_feed() {
        assert!(parse_feed(b"<html><title>Upstream error</title></html>", "https://example.org").is_err());
        assert!(parse_feed(b"<rss><channel><item><title>partial", "https://example.org").is_err());
        let (_scratch, store) = Scratch::store();
        store.import(&[RssFeed { name: "Example".into(), url: "http://127.0.0.1:1/offline".into(), articles: vec![RssArticle { id: "cached".into(), title: "cached release".into(), ..RssArticle::default() }], ..RssFeed::default() }]).unwrap();
        assert!(store.refresh("Example", 1).is_err());
        let feeds = store.list().unwrap();
        assert!(feeds[0].has_error);
        assert_eq!(feeds[0].articles[0].id, "cached");
    }

    #[test]
    fn one_refresh_returns_current_items_and_uses_conditional_requests() {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/rss", listener.local_addr().unwrap());
        let headers = Arc::new(Mutex::new(Vec::new()));
        let log = headers.clone();
        let worker = std::thread::spawn(move || {
            for response in [format!("HTTP/1.1 200 OK\r\nETag: \"version-1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{RSS}", RSS.len()),
                "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new(); reader.read_line(&mut line).unwrap();
                    if line == "\r\n" { break; } head.push_str(&line);
                }
                log.lock().unwrap().push(head);
                socket.write_all(response.as_bytes()).unwrap();
            }
        });
        let (_scratch, store) = Scratch::store();
        store.import(&[RssFeed { name: "Example".into(), url, articles: vec![RssArticle { id: "episode-1".into(), title: "old title".into(), is_read: true, ..RssArticle::default() }], ..RssFeed::default() }]).unwrap();
        store.refresh("Example", 2).unwrap();
        let feeds = store.list().unwrap();
        assert_eq!(feeds[0].articles.len(), 2);
        assert_eq!(feeds[0].articles[0].id, "episode-2");
        assert!(feeds[0].articles[1].is_read);
        assert!(!feeds[0].articles[0].is_read);
        store.refresh("Example", 2).unwrap();
        worker.join().unwrap();
        assert!(headers.lock().unwrap()[1].to_ascii_lowercase().contains("if-none-match: \"version-1\""));
        assert_eq!(store.list().unwrap()[0].articles.len(), 2);
    }

    #[test]
    fn marking_read_during_network_refresh_is_not_overwritten() {
        use std::io::{BufRead, BufReader};
        use std::net::TcpListener;
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/rss", listener.local_addr().unwrap());
        let (started, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            loop {
                let mut line = String::new(); reader.read_line(&mut line).unwrap();
                if line == "\r\n" { break; }
            }
            started.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{RSS}", RSS.len()).as_bytes()).unwrap();
        });
        let (_scratch, store) = Scratch::store();
        store.import(&[RssFeed { name: "Example".into(), url, articles: vec![RssArticle { id: "episode-1".into(), ..RssArticle::default() }], ..RssFeed::default() }]).unwrap();
        let refresher = store.clone();
        let refreshing = std::thread::spawn(move || refresher.refresh("Example", 5));
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        store.mark_read("Example", Some("episode-1")).unwrap();
        release.send(()).unwrap();
        refreshing.join().unwrap().unwrap(); worker.join().unwrap();
        assert!(store.list().unwrap()[0].articles.iter().find(|row| row.id == "episode-1").unwrap().is_read);
    }
}
