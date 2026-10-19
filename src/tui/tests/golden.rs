//! Baselines of the unsplit renderer; regenerate deliberately with FBT_UPDATE_GOLDEN=1.
use super::*;
use std::{fs, path::PathBuf};

#[path = "fixture.rs"]
mod fixture;

pub(super) fn render(app: &TuiApp, width: u16, height: u16) -> String {
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

pub(super) fn compare(path: &str, expected: &str, actual: &str) -> Result<(), String> {
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
goldens!(clipboard,torrents, search, shows, rss, files, providers, settings, prompt, help, confirm, done, details, poster, disk);

#[test]
fn done_row_offers_seed_instead_of_resume() {
    let mut app = fixture::app("torrents");
    app.on_torrents_key(KeyEvent::new(KeyCode::Down));
    let text = render(&app, 80, 24);
    let footer = text.lines().find(|line| line.contains("[Torrents]")).unwrap();
    println!("asserted footer: {footer}");
    assert!(footer.contains("(u) Seed"));
    assert!(!footer.contains("(p) Resume"));
    assert!(matches!(app.on_torrents_key(KeyEvent::new(KeyCode::Char('u'))), Cmd::Task(..)));
}

#[test]
fn resume_unknown_completion_timestamp_keeps_incomplete_controls() {
    for completion_on in [-1, 0] {
        let mut app = fixture::app("torrents");
        app.torrents[0].state = "stoppedDL".into();
        app.torrents[0].completion_on = completion_on;
        let text = render(&app, 80, 24);
        let footer = text.lines().find(|line| line.contains("[Torrents]")).unwrap();
        println!("completion_on={completion_on}, asserted footer: {footer}");
        assert!(footer.contains("(p) Resume"));
        assert!(matches!(app.on_torrents_key(KeyEvent::new(KeyCode::Char('p'))), Cmd::Task(..)));
    }
}

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

#[test]
fn footer_overflow_80x24() {
    let app=fixture::app("torrents");
    let enabled=app.footer_labels();
    assert!(enabled.len()>=9,"fixture must enable nine or more actions");
    let actual=render(&app,80,24);
    let footer=actual.lines().nth(22).unwrap();
    println!("nine-action overflow control: {footer}");
    assert!(footer.contains('…') && footer.ends_with("(?) Help"));
    let path="src/tui/tests/golden/footer-overflow-80x24.txt";
    if std::env::var("FBT_UPDATE_GOLDEN").as_deref()==Ok("1") { fs::write(path,&actual).unwrap(); }
    else { compare(path,&fs::read_to_string(path).unwrap(),&actual).unwrap_or_else(|diff|panic!("{diff}")); }
}

#[test]
fn watch_ready_and_may_stall_goldens() {
    for (name,stall,buffer,finish) in [("watch-ready",true,240,42),("watch-may-stall",false,30,1080)] {
        let mut app=fixture::app("torrents");
        let row:crate::api::Torrent=serde_json::from_value(serde_json::json!({"hash":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","name":"Watch fixture.mkv","state":"downloading","intent":"run","staged":false,"checked":true,"seed":false,"upload_policy":"default_cap","save_path":"/tmp/fbt-fixture","added_at":1792281600,"force":false,"size":67108864,"completed_bytes":41943040,"progress":0.625,"dl_bps":1048576,"playable":{"file":0,"ready":true,"prefix_bytes":41943040,"acked":false,"stall_free":stall,"buffered_s":buffer,"finishes_in_s":finish}})).unwrap();
        let legacy=super::format::torrent(&row);
        app.torrents[0]=legacy.clone();app.store.torrents.insert(row.hash.to_string(),row);
        let _=app.open_details(&legacy);
        let actual=render(&app,80,24);let path=format!("src/tui/tests/golden/{name}-80x24.txt");
        assert!(actual.contains(if stall {"Ready to watch"}else{"may stall"}));
        if std::env::var("FBT_UPDATE_GOLDEN").as_deref()==Ok("1") {fs::write(&path,&actual).unwrap();}
        else {let expected=fs::read_to_string(&path).unwrap();compare(&path,&expected,&actual).unwrap_or_else(|diff|panic!("{diff}"));}
        println!("{path}: truthful watch detail at 80x24; synthetic rendering only");
    }
}
