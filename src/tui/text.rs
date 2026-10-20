//! Existing text behavior.
use super::*;
use ftui::text::wrap::graphemes;

/// Display width in terminal cells.
pub(super) fn dw(s: &str) -> usize { unicode_width::UnicodeWidthStr::width(s) }

/// Drop controls and standalone invisible clusters, keeping accents, variation
/// selectors and joiners attached to their visible grapheme. Collapse spaces.
pub(super) fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for cluster in graphemes(s) {
        if cluster.chars().any(char::is_control) || dw(cluster) == 0 { continue; }
        if cluster.chars().all(char::is_whitespace) {
            if !last_space { out.push(' '); }
            last_space = true;
        } else {
            out.push_str(cluster);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// Fit a string into `max` cells: keep the head and the tail, ellipsis in between.
pub(super) fn short(s: &str, max: usize) -> String {
    let s = clean(s);
    if max == 0 { return String::new(); }
    if dw(&s) <= max { return s; }
    if max <= 1 { return "…".into(); }
    let keep = max - 1;
    let head_w = keep * 2 / 3;
    let tail_w = keep - head_w;
    let mut head_end = 0;
    let mut w = 0;
    for cluster in graphemes(&s) {
        let cw = dw(cluster);
        if w + cw > head_w { break; }
        head_end += cluster.len(); w += cw;
    }
    let tail_column = dw(&s).saturating_sub(tail_w);
    let mut tail_start = 0;
    let mut column = 0;
    for cluster in graphemes(&s) {
        if column >= tail_column { break; }
        column += dw(cluster);
        tail_start += cluster.len();
    }
    let out = format!("{}…{}", &s[..head_end], &s[tail_start..]);
    // Pad if wide chars left us a cell short so columns stay aligned.
    let pad = max.saturating_sub(dw(&out));
    format!("{out}{}", " ".repeat(pad))
}

/// Left-align `s` in exactly `w` cells.
pub(super) fn cell(s: &str, w: usize) -> String {
    let t = short(s, w);
    format!("{t}{}", " ".repeat(w.saturating_sub(dw(&t))))
}

/// Only an unselected hovered row receives a clock. Keyboard selection is static.
pub(super) fn hovered_for(state: &RefCell<ListState>, index: usize, elapsed: Option<Duration>) -> Option<Duration> {
    let state=state.borrow();
    if state.hovered==Some(index) && state.selected()!=Some(index) { elapsed } else { None }
}

/// The existing 300 ms clock advances at eight terminal cells per second.
/// One final-window tick is guaranteed before returning to the elided start.
pub(super) fn marquee_offset(name_width: usize, width: usize, elapsed: Duration) -> Option<usize> {
    let travel=name_width.checked_sub(width)?;
    if width<2 || travel==0 || elapsed<Duration::from_millis(900) { return None; }
    let moving=elapsed-Duration::from_millis(900);
    let end_ms=(travel as u128*1000).div_ceil(8);
    if moving.as_millis()>=end_ms+300 { return None; }
    Some((moving.as_millis()*8/1000).min(travel as u128) as usize)
}

/// A hovered name travels to the end once, resets, and stays still. Never wraps.
pub(super) fn readable_cell(s: &str, w: usize, hovered_for: Option<Duration>) -> String {
    let Some(elapsed)=hovered_for else { return cell(s,w) };
    let name=clean(s);
    let Some(offset)=marquee_offset(dw(&name),w,elapsed) else { return cell(&name,w) };
    let mut out=String::with_capacity(w);
    let mut pos=0;
    let mut used=0;
    for cluster in graphemes(&name) {
        let cw=dw(cluster);
        if pos+cw<=offset { pos+=cw;continue; }
        if pos<offset { let gap=pos+cw-offset;out.push_str(&" ".repeat(gap));used+=gap;pos+=cw;continue; }
        if used+cw>w { break; }
        out.push_str(cluster);used+=cw;pos+=cw;
    }
    out.push_str(&" ".repeat(w.saturating_sub(used)));
    out
}

pub(super) fn human_count(n: u32) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) } else if n >= 1000 { format!("{}k", n / 1000) } else { n.to_string() }
}

pub(super) fn ago(unix: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let d = (now - unix).max(0);
    if d < 60 { "just now".into() }
    else if d < 3600 { format!("{}m ago", d / 60) }
    else if d < 86400 { format!("{}h ago", d / 3600) }
    else { format!("{}d ago", d / 86400) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_and_marquee_never_split_visible_graphemes() {
        let name = "Cafe\u{301}👩🏽‍💻日本語👨‍👩‍👧‍👦.mkv";
        let clusters: Vec<_> = graphemes(name).collect();
        assert_eq!(clean(name), name);
        for width in 1..32 {
            let fitted = cell(name, width);
            assert_eq!(dw(&fitted), width);
            for text in std::iter::once(fitted).chain((0..40).map(|tick|
                readable_cell(name, width, Some(Duration::from_millis(tick * 300))))) {
                assert_eq!(dw(&text), width, "{width}: {text:?}");
                for cluster in graphemes(text.trim()) {
                    assert!(cluster == "…" || cluster == " " || clusters.contains(&cluster),
                        "split grapheme at {width} columns: {text:?}");
                }
            }
        }
    }

    #[test]
    fn visible_accents_do_not_reintroduce_terminal_controls_or_invisible_prefixes() {
        assert_eq!(clean("\u{301}  Cafe\u{301}\u{200b}\u{202e}\u{2066}\0\x1b\n\t  日本語  "),
            "Cafe\u{301} 日本語");
    }
}
