//! Selector and torrent source helpers; bodies moved without changes.
use super::*;

/// Expand selectors (hash prefix, unique name substring, or `all`) to full hashes.
pub(super) fn resolve_in(ts: &[Torrent], selectors: &[String]) -> Result<Vec<String>> {
    if selectors.is_empty() {
        return Err(AppError::new(Code::Usage, "no torrent selector given").hint("fbittorrent list --json | jq -r '.torrents[].hash'   # then pass a hash prefix, a unique part of the name, or `all`").into());
    }
    if selectors.len() == 1 && selectors[0] == "all" {
        return Ok(ts.iter().map(|t| t.hash.clone()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    for sel in selectors {
        let s = sel.trim();
        let low = s.to_lowercase();
        let is_hexish = low.len() >= 4 && low.chars().all(|c| c.is_ascii_hexdigit());
        let by_hash: Vec<&Torrent> = if is_hexish { ts.iter().filter(|t| t.hash.starts_with(&low)).collect() } else { vec![] };
        let matches: Vec<&Torrent> = if !by_hash.is_empty() { by_hash } else {
            ts.iter().filter(|t| t.name.to_lowercase().contains(&low)).collect()
        };
        match matches.len() {
            1 => { let h = matches[0].hash.clone(); if !out.contains(&h) { out.push(h); } }
            0 => return Err(AppError::new(Code::NotFound, format!("no torrent matches '{s}'"))
                .hint("fbittorrent list --json | jq -r '.torrents[] | \"\\(.hash[0:8])  \\(.name)\"'").into()),
            n => return Err(AppError::new(Code::Ambiguous, format!("'{s}' matches {n} torrents: {}",
                    matches.iter().take(6).map(|t| format!("{} ({})", &t.hash[..8], short(&t.name, 40))).collect::<Vec<_>>().join(", ")))
                .hint("use a longer hash prefix, or the exact name").into()),
        }
    }
    Ok(out)
}

pub(super) fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// Info hash from a magnet link (lowercase hex), if present.
pub fn magnet_hash(src: &str) -> Option<String> {
    let low = src.to_ascii_lowercase();
    let i = low.find("urn:btih:")?;
    let h: String = low[i + 9..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if h.len() == 40 { Some(h) } else if h.len() == 32 { Some(base32_to_hex(&h)) } else { None }
}

pub(super) fn base32_to_hex(s: &str) -> String {
    let mut bits = 0u64;
    let mut nbits = 0;
    let mut out = String::new();
    for c in s.chars() {
        let v = match c { 'a'..='z' => c as u64 - 'a' as u64, '2'..='7' => c as u64 - '2' as u64 + 26, _ => continue };
        bits = (bits << 5) | v; nbits += 5;
        while nbits >= 8 { nbits -= 8; out.push_str(&format!("{:02x}", (bits >> nbits) & 0xff)); }
    }
    out
}

pub fn magnet_name(src: &str) -> Option<String> {
    let q = src.split_once('?')?.1;
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("dn=") { return Some(percent_decode(v)); }
    }
    None
}

pub(super) fn percent_decode(s: &str) -> String {
    fn hex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { out.push(b' '); i += 1; }
            b'%' if i + 2 < b.len() => {
                if let Some(v) = hex(b[i + 1]).zip(hex(b[i + 2])).map(|(a, b)| a * 16 + b) {
                    out.push(v); i += 3;
                } else { out.push(b'%'); i += 1; }
            }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(super) fn local_torrent_source(source: &str) -> Result<String> {
    if !source.starts_with("file:") { return Ok(source.to_owned()); }
    let uri = url::Url::parse(source).context("invalid local torrent file URL")?;
    let path = uri.to_file_path().map_err(|_| anyhow::anyhow!("torrent file URL must name a local file"))?;
    path.into_os_string().into_string().map_err(|_| anyhow::anyhow!("torrent file path must be valid UTF-8"))
}
