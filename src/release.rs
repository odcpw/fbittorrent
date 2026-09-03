//! Turn a torrent/release name into a searchable title (+ year, + episode tag).

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Release {
    pub title: String,
    pub year: Option<u16>,
    pub episode: Option<String>, // "S01E02"
}

const STOP: &[&str] = &[
    "1080p", "720p", "2160p", "480p", "4k", "uhd", "hdr", "hdr10", "dv", "x264", "x265", "h264", "h265",
    "hevc", "avc", "av1", "web", "webrip", "web-dl", "webdl", "bluray", "blu-ray", "bdrip", "brrip", "dvdrip",
    "hdtv", "hdrip", "remux", "proper", "repack", "internal", "limited", "extended", "unrated", "directors",
    "multi", "dual", "aac", "aac2", "ac3", "dts", "ddp", "ddp5", "dd5", "atmos", "truehd", "10bit", "8bit",
    "yify", "yts", "rarbg", "amzn", "nf", "hmax", "dsnp", "atvp", "complete", "season", "xvid", "divx",
];

pub fn parse(name: &str) -> Release {
    // Anime releases lead with the group tag: "[Ironclad] Title - S01E09", "[SubsPlease] Title - 1176".
    let mut name = name.trim();
    while name.starts_with('[') {
        match name.find(']') { Some(end) => name = name[end + 1..].trim_start(), None => break }
    }
    let mut title = Vec::new();
    let mut year = None;
    let mut episode = None;
    // "Title - 1176 (1080p)": a bare number after " - " is an absolute episode number.
    if let Some(i) = name.find(" - ") {
        let tail = &name[i + 3..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        let ends = tail[digits.len()..].chars().next().map(|c| !c.is_alphanumeric()).unwrap_or(true);
        if !digits.is_empty() && ends && as_year(&digits).is_none() {
            episode = Some(digits);
            name = &name[..i];
        }
    }
    // Split on the usual separators, keep apostrophes inside words.
    let cleaned = name.replace(['.', '_', '[', ']', '(', ')', '{', '}'], " ");
    let toks: Vec<&str> = cleaned.split_whitespace()
        .map(|raw| raw.trim_matches(|c: char| c == '-' || c == ','))
        .filter(|t| !t.is_empty())
        .collect();
    for (i, tok) in toks.iter().enumerate() {
        let low = tok.to_ascii_lowercase();
        if let Some(y) = as_year(&low) {
            // "Blade Runner 2049 2017": a year followed by another year is part of the title.
            let next_is_year = toks.get(i + 1).map(|n| as_year(&n.to_ascii_lowercase()).is_some()).unwrap_or(false);
            if !title.is_empty() && !next_is_year { year = Some(y); break; }
            if next_is_year { title.push(tok.to_string()); continue; }
        }
        if is_episode(&low) { episode = Some(low.to_uppercase()); break; }
        // "0001-1071+Movies", "1176": an episode range or a 4+ digit non-year number ends the
        // title. Short numbers stay ("District 9", "Apollo 13").
        if !title.is_empty() && low.starts_with(|c: char| c.is_ascii_digit()) && as_year(&low).is_none()
            && (low.contains('-') || low.contains('+') || low.chars().all(|c| c.is_ascii_digit()) && low.len() >= 4) { break; }
        if STOP.contains(&low.as_str()) { break; }
        if low.starts_with("s0") || low.starts_with("s1") || low.starts_with("s2") {
            if low.len() == 3 && low[1..].chars().all(|c| c.is_ascii_digit()) { break; } // "S01"
        }
        title.push(tok.to_string());
    }
    let mut title = title.join(" ");
    if let Some(i) = title.find(" - ") { title.truncate(i); }
    Release { title: title.trim().to_string(), year, episode }
}

fn as_year(t: &str) -> Option<u16> {
    if t.len() != 4 || !t.chars().all(|c| c.is_ascii_digit()) { return None; }
    let y: u16 = t.parse().ok()?;
    (1900..=2100).contains(&y).then_some(y)
}

fn is_episode(t: &str) -> bool {
    // s01e02, s1e2, 1x02
    let b = t.as_bytes();
    if b.len() >= 4 && b[0] == b's' {
        let rest = &t[1..];
        if let Some(pos) = rest.find('e') {
            let (s, e) = (&rest[..pos], &rest[pos + 1..]);
            return !s.is_empty() && !e.is_empty() && s.chars().all(|c| c.is_ascii_digit()) && e.chars().all(|c| c.is_ascii_digit());
        }
    }
    if let Some(pos) = t.find('x') {
        let (s, e) = (&t[..pos], &t[pos + 1..]);
        return !s.is_empty() && e.len() >= 2 && s.chars().all(|c| c.is_ascii_digit()) && e.chars().all(|c| c.is_ascii_digit());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn movies_and_shows() {
        let r = parse("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.H.264-FLUX");
        assert_eq!(r.title, "Dune Part Two"); assert_eq!(r.year, Some(2024));
        let r = parse("The.Bear.S03E04.1080p.WEB.h264-ETHEL[TGx]");
        assert_eq!(r.title, "The Bear"); assert_eq!(r.episode.as_deref(), Some("S03E04"));
        let r = parse("ubuntu-24.04.2-desktop-amd64.iso");
        assert_eq!(r.title, "ubuntu-24 04"); // not media; only matters that it does not panic
        let r = parse("Blade Runner 2049 (2017) [1080p] [YTS]");
        assert_eq!(r.title, "Blade Runner 2049"); assert_eq!(r.year, Some(2017));
    }
    #[test]
    fn anime_group_tags_and_absolute_episodes() {
        let r = parse("[Ironclad] THE GHOST IN THE SHELL - S01E09 [WEB.1080p.AV1] [Multi-Audio, Multi-Subs] (Weekly)");
        assert_eq!(r.title, "THE GHOST IN THE SHELL"); assert_eq!(r.episode.as_deref(), Some("S01E09"));
        let r = parse("[SubsPlease] One Piece - 1176 (1080p) [8855DEBC].mkv");
        assert_eq!(r.title, "One Piece"); assert_eq!(r.episode.as_deref(), Some("1176"));
        let r = parse("[Anime Time] One Piece (0001-1071+Movies+Specials) [BD+CR]");
        assert_eq!(r.title, "One Piece");
    }
}
