//! Deterministic identities, conservative text repair and provider aggregation.
use super::*;
use crate::api::SearchItem;

pub(crate) fn repair_name(name: &str) -> String {
    // Windows-1252 punctuation is commonly mixed with Latin-1 by web scrapers.
    // Accept a repair only when the *whole* inverse encoding is valid UTF-8.
    let punctuation = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ";
    let bytes: Option<Vec<u8>> = name
        .chars()
        .map(|c| {
            if u32::from(c) <= 255 {
                Some(c as u8)
            } else {
                punctuation
                    .chars()
                    .position(|p| p == c)
                    .map(|n| n as u8 + 128)
            }
        })
        .collect();
    bytes
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| name.into())
}

pub(crate) fn btih(source: &str) -> Option<String> {
    librqbit::Magnet::parse(source)
        .ok()?
        .as_id20()
        .map(|h| h.as_string())
}

/// Inspect local metainfo only; never a payload or a provider download hook.
/// Remote result URLs are never fetched while browsing results: they are
/// provider-controlled, and a prefetch would contact arbitrary hosts (over
/// plain HTTP, through redirects, outside the VPN binding) for every row.
pub(super) fn torrent_hash(source: &str) -> Option<String> {
    if !source.to_ascii_lowercase().ends_with(".torrent") || url::Url::parse(source).is_ok() {
        return None;
    }
    let bytes = if Path::new(source).is_absolute() {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(source)
            .ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_TORRENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_TORRENT_BYTES {
            return None;
        }
        bytes
    } else {
        return None;
    };
    Some(
        librqbit::torrent_from_bytes(&bytes)
            .ok()?
            .info_hash
            .as_string(),
    )
}

pub(crate) fn dedupe_key(result: &SearchResult) -> String {
    btih(&result.file_url).unwrap_or_else(|| {
        let title = repair_name(&result.file_name)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        // The unknown size has its own token; it is never a fabricated zero.
        format!(
            "title:{title}|size:{}",
            if result.file_size < 0 {
                "?".into()
            } else {
                result.file_size.to_string()
            }
        )
    })
}

pub(crate) fn item(result: &SearchResult) -> SearchItem {
    let optional = |s: &String| (!s.is_empty()).then(|| s.clone());
    SearchItem {
        key: dedupe_key(result),
        name: repair_name(&result.file_name),
        providers: vec![result.engine.clone()],
        size: u64::try_from(result.file_size).ok(),
        seeds: u64::try_from(result.seeders).ok(),
        leechers: u64::try_from(result.leechers).ok(),
        uploaded_at: (result.pub_date > 0).then_some(result.pub_date as u64),
        magnet: result
            .file_url
            .starts_with("magnet:")
            .then(|| result.file_url.clone()),
        url: (!result.file_url.starts_with("magnet:"))
            .then(|| result.file_url.clone())
            .filter(|s| !s.is_empty()),
        page_url: optional(&result.descr_link),
        imdb_id: optional(&result.imdb),
        rating: None,
        provider_url: optional(&result.site_url),
        added: None,
    }
}

pub(crate) fn aggregate(rows: &[SearchResult], limit: usize) -> Vec<SearchItem> {
    aggregate_with_hashes(rows, limit, &BTreeMap::new())
}

pub(super) fn aggregate_with_hashes(
    rows: &[SearchResult],
    limit: usize,
    hashes: &BTreeMap<String, String>,
) -> Vec<SearchItem> {
    let mut unique: BTreeMap<String, SearchItem> = BTreeMap::new();
    for row in rows {
        let mut next = item(row);
        if let Some(hash) = hashes.get(&row.file_url) {
            next.key = hash.clone();
        }
        // Untrusted provider strings share the strict output bounds.
        if next.validate().is_err() {
            continue;
        }
        if let Some(existing) = unique.get_mut(&next.key) {
            let mut providers = existing.providers.clone();
            providers.extend(next.providers.clone());
            providers.sort();
            providers.dedup();
            if next.seeds > existing.seeds
                || (next.seeds == existing.seeds
                    && (
                        &next.name,
                        &next.provider_url,
                        &next.magnet,
                        &next.url,
                        &next.page_url,
                    ) < (
                        &existing.name,
                        &existing.provider_url,
                        &existing.magnet,
                        &existing.url,
                        &existing.page_url,
                    ))
            {
                *existing = next;
            }
            existing.providers = providers;
        } else {
            unique.insert(next.key.clone(), next);
        }
    }
    let mut items: Vec<_> = unique.into_values().collect();
    items.sort_by(|a, b| {
        b.seeds
            .cmp(&a.seeds)
            .then(a.name.cmp(&b.name))
            .then(a.providers.cmp(&b.providers))
    });
    items.truncate(limit);
    items
}
