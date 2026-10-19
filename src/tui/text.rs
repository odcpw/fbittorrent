//! Existing text behavior.
use super::*;

/// List row with a blank 2-column marker, so unselected rows sit at the same
/// x as the highlighted one (the widget only indents the selected row otherwise).
pub(super) fn li<'t>(content: impl Into<Text<'t>>) -> ListItem<'static> {
    ListItem::new(content).marker(" ")
}

/// Display width in terminal cells.
pub(super) fn dw(s: &str) -> usize { unicode_width::UnicodeWidthStr::width(s) }

/// Drop control and zero-width characters and collapse whitespace so that
/// measured width matches what the terminal paints.
pub(super) fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if c.is_control() || w == 0 { continue; }
        if c.is_whitespace() {
            if !last_space { out.push(' '); }
            last_space = true;
        } else {
            out.push(c);
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
    let mut head = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > head_w { break; }
        head.push(c); w += cw;
    }
    let mut tail_chars = Vec::new();
    let mut w = 0;
    for c in s.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > tail_w { break; }
        tail_chars.push(c); w += cw;
    }
    let tail: String = tail_chars.into_iter().rev().collect();
    let out = format!("{head}…{tail}");
    // Pad if wide chars left us a cell short so columns stay aligned.
    let pad = max.saturating_sub(dw(&out));
    format!("{out}{}", " ".repeat(pad))
}

/// Left-align `s` in exactly `w` cells.
pub(super) fn cell(s: &str, w: usize) -> String {
    let t = short(s, w);
    format!("{t}{}", " ".repeat(w.saturating_sub(dw(&t))))
}

/// After a pause, slide a fixed-width window across the entire selected name.
pub(super) fn readable_cell(s: &str, w: usize, selected_for: Option<Duration>) -> String {
    let Some(elapsed) = selected_for else { return cell(s, w) };
    let name = clean(s);
    if w < 2 || dw(&name) <= w || elapsed < Duration::from_millis(900) { return cell(&name, w); }
    let offset = ((elapsed - Duration::from_millis(900)).as_millis() / 160) as usize % (dw(&name) + 3);
    let looped = format!("{name}   {name}");
    let mut out = String::new();
    let mut pos = 0;
    let mut used = 0;
    for c in looped.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if pos + cw <= offset { pos += cw; continue; }
        if pos < offset { out.push(' '); used += 1; pos += cw; continue; }
        if used + cw > w { break; }
        out.push(c); used += cw; pos += cw;
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
