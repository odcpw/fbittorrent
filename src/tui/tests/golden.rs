//! Baselines of the unsplit renderer; regenerate deliberately with FBT_UPDATE_GOLDEN=1.
use super::*;
use std::{fs, path::PathBuf};

#[path = "fixture.rs"]
mod fixture;

fn render(app: &TuiApp, width: u16, height: u16) -> String {
    let mut pool = ftui::GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    app.view(&mut frame);
    frame_text(&frame)
}

fn frame_text(frame: &Frame) -> String {
    let Rect { width, height, .. } = frame.bounds();
    let mut text = String::new();
    for y in 0..height {
        let start = text.len();
        for x in 0..width {
            let content = frame.buffer.get(x, y).unwrap().content;
            if let Some(ch) = content.as_char() {
                text.push(ch);
            } else if let Some(id) = content.grapheme_id() {
                text.push_str(frame.pool.get(id).expect("valid frame grapheme"));
            } else if !content.is_continuation() {
                text.push(' ');
            }
        }
        assert_eq!(dw(&text[start..]), usize::from(width), "serialized row {y}");
        // Right padding carries no content and would trip git diff --check.
        text.truncate(text.trim_end_matches(' ').len());
        text.push('\n');
    }
    text
}

fn compare(path: &str, expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual { Ok(()) } else { Err(unified_diff(path, expected, actual)) }
}

/// One valid unified hunk, bounded by three unchanged context lines. Frames
/// have fixed row counts, but handling both lengths also explains missing rows.
fn unified_diff(path: &str, expected: &str, actual: &str) -> String {
    let old: Vec<_> = expected.lines().collect();
    let new: Vec<_> = actual.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b).count();
    let start = prefix.saturating_sub(3);
    let context = suffix.min(3);
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    let mut diff = format!("--- {path}\n+++ actual\n@@ -{},{} +{},{} @@\n",
        start + 1, old_end + context - start, start + 1, new_end + context - start);
    for (mark, lines) in [
        (' ', &old[start..prefix]), ('-', &old[prefix..old_end]),
        ('+', &new[prefix..new_end]), (' ', &old[old_end..old_end + context]),
    ] {
        for line in lines { diff.push(mark); diff.push_str(line); diff.push('\n'); }
    }
    diff
}

fn assert_render(surface: &str, width: u16, height: u16) {
    // Cargo runs tests in the package root; a runtime-relative path also lets
    // an rch-built test executable regenerate baselines on the local checkout.
    let path = PathBuf::from("src/tui/tests/golden")
        .join(format!("{surface}-{width}x{height}.txt"));
    let actual = render(&fixture::app(surface), width, height);
    let name = path.display().to_string();
    if std::env::var("FBT_UPDATE_GOLDEN").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).unwrap();
        eprintln!("updated {name}: {height} lines");
    } else {
        let expected = fs::read_to_string(&path).unwrap_or_else(|error|
            panic!("{name}: {error}; create/review with FBT_UPDATE_GOLDEN=1 cargo test tui::golden -- --nocapture"));
        compare(&name, &expected, &actual).unwrap_or_else(|diff|
            panic!("golden mismatch:\n{diff}\nReview deliberate changes with FBT_UPDATE_GOLDEN=1."));
        eprintln!("matched {name}: {height} lines");
    }
}

macro_rules! goldens {
    ($($surface:ident),+ $(,)?) => { $(mod $surface {
        use super::*;
        #[test] fn narrow() { assert_render(stringify!($surface), 80, 24); }
        #[test] fn medium() { assert_render(stringify!($surface), 120, 30); }
        #[test] fn wide() { assert_render(stringify!($surface), 160, 45); }
    })+ };
}
goldens!(torrents, search, shows, rss, files, providers, settings, prompt, help, confirm);

#[test]
fn all_surfaces_render_at_60x20_without_panics() {
    for surface in fixture::SURFACES {
        let text = render(&fixture::app(surface), 60, 20);
        assert_eq!(text.lines().count(), 20);
        eprintln!("rendered {surface} at 60x20 without panic");
    }
}

#[test]
fn cell_text_preserves_wide_and_combining_characters() {
    let mut pool = ftui::GraphemePool::new();
    let mut frame = Frame::new(20, 1, &mut pool);
    Paragraph::new("辛特尔 Cafe\u{301}").render(frame.bounds(), &mut frame);
    assert_eq!(frame_text(&frame), "辛特尔 Cafe\u{301}\n");
}

#[test]
fn one_character_label_change_produces_a_unified_diff() {
    let expected = "border\nSearch\nborder\n";
    let actual = "border\nSearci\nborder\n";
    let diff = compare("search-80x24.txt", expected, actual).unwrap_err();
    assert_eq!(diff, "--- search-80x24.txt\n+++ actual\n@@ -1,3 +1,3 @@\n border\n-Search\n+Searci\n border\n");
    assert!(compare("search-80x24.txt", expected, expected).is_ok());
}
