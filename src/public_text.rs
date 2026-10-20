//! Text that leaves the process: API views, CLI output, terminal rows.
//! Private feed URLs carry passkeys in their userinfo, query and path; feed
//! and torrent names are remote text that may carry terminal escapes.
use crate::rss::stable_id;

const BASE: &str = "https://redaction.invalid/";

/// Display form of a feed or .torrent URL: no userinfo, query (shown as "?…"),
/// fragment or credential-like path segments. The raw URL stays in rss.json.
pub(crate) fn url(value: &str) -> String {
    match url::Url::parse(value) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https" | "udp") => librqbit::redact_tracker_url(&parsed),
        Ok(parsed) if parsed.scheme() == "magnet" => magnet(value),
        _ => String::new(),
    }
}

/// A page link keeps its query (details pages need ids) but loses userinfo,
/// credential-named parameters and passkey path segments; a downloadable
/// link (.torrent/magnet) is treated like a torrent URL.
pub(crate) fn link(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else { return String::new() };
    if parsed.scheme() == "magnet" { return magnet(value); }
    if !matches!(parsed.scheme(), "http" | "https") { return String::new(); }
    if parsed.path().to_ascii_lowercase().ends_with(".torrent")
        || parsed.path_segments().into_iter().flatten().any(|s| s.to_ascii_lowercase().contains("passkey")) {
        return url(value);
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    if parsed.query().is_some() {
        let pairs: Vec<(String, String)> = parsed.query_pairs()
            .filter(|(name, _)| !credential_name(name))
            .map(|(name, value)| (name.into_owned(), value.into_owned())).collect();
        parsed.set_query(None);
        if !pairs.is_empty() { parsed.query_pairs_mut().extend_pairs(pairs); }
    }
    parsed.into()
}

fn credential_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["pass", "key", "token", "auth", "secret", "session", "sign"].iter().any(|part| name.contains(part))
        || matches!(name.as_str(), "uid" | "pk" | "sid")
}

/// Magnets keep their identity (xt, dn) and public trackers; tracker and
/// source URLs (tr, xs, as, ws) lose credentials.
pub(crate) fn magnet(value: &str) -> String {
    let Some(query) = value.strip_prefix("magnet:?") else { return String::new() };
    // Rewrite only the URL-valued parameters; xt/dn keep their exact text.
    let pairs: Vec<String> = query.split('&').map(|pair| match pair.split_once('=') {
        Some((name @ ("tr" | "xs" | "as" | "ws"), raw)) => {
            let decoded = urlencoding::decode(&raw.replace('+', " ")).map(|v| v.into_owned()).unwrap_or_default();
            format!("{name}={}", urlencoding::encode(&url(&decoded)))
        }
        _ => pair.to_owned(),
    }).collect();
    format!("magnet:?{}", pairs.join("&"))
}

/// The API view of a cached feed item.
pub(crate) fn feed_item(item: &crate::api::FeedItem) -> crate::api::FeedItem {
    let shown = |value: &Option<String>, view: fn(&str) -> String| value.as_deref().map(view).filter(|v| !v.is_empty());
    crate::api::FeedItem { link: shown(&item.link, link), magnet: shown(&item.magnet, magnet), torrent_url: shown(&item.torrent_url, url), ..item.clone() }
}

/// Remote diagnostics (feed errors) without URL credentials or echoed values.
pub(crate) fn message(source: &str, text: &str) -> String {
    let base = url::Url::parse(source).unwrap_or_else(|_| url::Url::parse(BASE).unwrap());
    librqbit::sanitize_tracker_message(&base, text).replace("?…", "")
}

/// A short feed identity for messages: its display URL, or its stable id.
pub(crate) fn feed_label(value: &str) -> String {
    let public = url(value);
    if public.is_empty() { stable_id(value) } else { public }
}

/// Human CLI output: remote names, titles and URLs never carry terminal
/// control sequences (ESC, CSI, OSC, BEL, CR, backspace...).
pub(crate) fn terminal(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// Printed CLI/log text: as `terminal`, but line breaks and tabs remain.
pub(crate) fn terminal_lines(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') { return text.into(); }
    text.chars().map(|c| if c.is_control() && c != '\n' && c != '\t' { ' ' } else { c }).collect::<String>().into()
}

/// Only web pages may be handed to xdg-open from untrusted results/cards.
pub(crate) fn web_link(value: &str) -> Option<&str> {
    url::Url::parse(value).ok().filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()).map(|_| value)
}

/// Hand an untrusted web link (search result page, Wikipedia card, provider
/// home page) to the desktop opener; any other scheme, path or option-like
/// text is refused, never executed. Returns whether it was opened.
pub(crate) fn open_web(value: &str) -> bool {
    match web_link(value) {
        Some(link) => std::process::Command::new("xdg-open").arg(link).spawn().is_ok(),
        None => false,
    }
}

/// Downloaded payload handed to the desktop opener: a torrent chooses its file
/// names, and xdg-open may *launch* a desktop entry or an executable instead
/// of showing it. Folders and ordinary media/documents are fine.
pub(crate) fn openable_payload(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let launcher = path.extension().is_some_and(|ext| {
        ["desktop", "appimage", "exe", "msi", "bat", "cmd", "com", "scr", "sh", "run", "bin", "jar", "lnk"]
            .iter().any(|bad| ext.eq_ignore_ascii_case(bad))
    });
    anyhow::ensure!(!launcher, "refusing to open a launcher or executable from a torrent: {}", path.display());
    // Some filesystems mark every file executable; refuse only real programs.
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            let mut magic = [0u8; 4];
            let read = std::fs::File::open(path).and_then(|mut file| std::io::Read::read(&mut file, &mut magic)).unwrap_or(0);
            let program = magic[..read].starts_with(b"#!") || magic[..read].starts_with(b"\x7fELF");
            anyhow::ensure!(!program, "refusing to open an executable file from a torrent: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_feed_items_show_no_credentials_but_keep_identity() {
        let raw = crate::api::FeedItem {
            id: "p1".into(), title: "Private".into(), description: None, published_at: None, size: None,
            link: Some("https://u:SECRETPW@t.invalid/details.php?id=7&authkey=SECRETAUTH".into()),
            magnet: Some("magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&tr=https%3A%2F%2Ft.invalid%2Fa%3Fpasskey%3DSECRETPASS".into()),
            torrent_url: Some("https://t.invalid/dl/42.torrent?passkey=SECRETPASS".into()),
            read: false, grabbed: false,
        };
        let shown = serde_json::to_string(&feed_item(&raw)).unwrap();
        for secret in ["SECRETPW", "SECRETAUTH", "SECRETPASS"] { assert!(!shown.contains(secret), "{shown}"); }
        let item = feed_item(&raw);
        assert_eq!(item.id, "p1");
        assert_eq!(item.link.as_deref(), Some("https://t.invalid/details.php?id=7"));
        assert_eq!(item.torrent_url.as_deref(), Some("https://t.invalid/dl/42.torrent?…"));
        assert!(item.magnet.unwrap().contains("btih:aaaa"));
    }

    #[test]
    fn rss_urls_never_expose_credentials() {
        for (raw, secret, page) in [
            ("https://user:hunter2@tracker.invalid/rss.php?passkey=abc123", "hunter2", true),
            ("https://tracker.invalid/rss.php?cat=1&passkey=abc123", "abc123", true),
            ("https://tracker.invalid/rss/0123456789abcdef0123456789abcdef/feed.xml", "0123456789abcdef", false),
            ("https://tracker.invalid/download.php/42/passkey=abc123/file.torrent", "abc123", true),
        ] {
            // Feed and torrent URLs are fully redacted; page links (which keep
            // their path for opening) lose userinfo and credential parameters.
            let mut shown = vec![url(raw), feed_label(raw), message(raw, &format!("fetching {raw}: refused"))];
            if page { shown.push(link(raw)); }
            for shown in shown {
                assert!(!shown.contains(secret), "{raw} -> {shown}");
                assert!(!shown.contains("user:"), "{raw} -> {shown}");
            }
        }
        assert_eq!(url("https://tracker.invalid/rss.php?passkey=abc"), "https://tracker.invalid/rss.php?…");
        // Details links keep their ids; only credential parameters go.
        assert_eq!(link("https://tracker.invalid/details.php?id=42&authkey=x&torrent_pass=y"), "https://tracker.invalid/details.php?id=42");
        assert_eq!(link("https://site.invalid/torrent/123/some-long-release-name-2026"), "https://site.invalid/torrent/123/some-long-release-name-2026");
        let shown = magnet("magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&dn=Name&tr=https%3A%2F%2Ftracker.invalid%2Fannounce%3Fpasskey%3Dsecret&tr=udp%3A%2F%2Fopen.invalid%3A1337%2Fannounce");
        assert!(!shown.contains("secret") && shown.contains("btih:aaaa") && shown.contains("open.invalid"), "{shown}");
        assert_eq!(url("javascript:alert(1)"), "");
    }

    #[test]
    fn payload_launchers_and_executables_are_never_opened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("fbt-open-payload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let movie = dir.join("Movie.2026.mkv");
        std::fs::write(&movie, b"x").unwrap();
        std::fs::set_permissions(&movie, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(openable_payload(&movie).is_ok() && openable_payload(&dir).is_ok());
        for name in ["Movie.desktop", "Setup.EXE", "player.AppImage", "run.sh"] {
            let path = dir.join(name);
            std::fs::write(&path, b"[Desktop Entry]\nExec=touch /tmp/pwned\n").unwrap();
            assert!(openable_payload(&path).is_err(), "{name}");
        }
        let script = dir.join("trailer.mp4");
        std::fs::write(&script, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(openable_payload(&script).is_err());
        // An exec bit alone (exFAT/NTFS mounts) does not block ordinary media.
        std::fs::set_permissions(&movie, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(openable_payload(&movie).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn terminal_text_and_links_are_inert() {
        assert_eq!(terminal("name\u{1b}]52;c;cHduZWQ=\u{7}\u{1b}[2J\r\u{8}"), "name ]52;c;cHduZWQ=  [2J  ");
        assert_eq!(terminal_lines("a\u{1b}[31mred\u{9b}2J\r\n\tb"), "a [31mred 2J \n\tb");
        assert!(matches!(terminal_lines("plain\nline"), std::borrow::Cow::Borrowed(_)));
        // The crate-wide print macros route through the same filter.
        let shown = format!("{}", terminal_lines(&format!("{}", "x\u{1b}]52;c;AAAA\u{7}")));
        assert!(!shown.contains('\u{1b}') && !shown.contains('\u{7}'));
        assert_eq!(web_link("https://example.org/page"), Some("https://example.org/page"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "smb://host/share", "/usr/bin/xterm", "apt:foo", "http://",
                    "--help", "x-scheme-handler:evil", "mailto:a@b.invalid"] {
            assert_eq!(web_link(bad), None, "{bad}");
            assert!(!open_web(bad), "{bad} must never reach xdg-open");
        }
    }
}
