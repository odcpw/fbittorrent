//! Behavioural surface checks at useful terminal sizes; historical captures stay as references.
use super::*;

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
    let app=fixture::app(surface);
    let actual=render(&app,width,height);
    let required:&[&str]=match surface {
        "torrents"|"done"=>&["STATE","NAME","↓ Down","✓ Done","◌ Meta","The Diplomat","Sintel","Other.Movie","42%","100%"],
        "search"=>&["Search","Results","NAME","SEEDS","Sintel","124"],
        "shows"=>&["All unread","Cached releases","RELEASE","SHOW","S05E16","S04E01","Unread"],
        "rss"=>&["RSS","Blender releases","Unread","Read","Sintel","Big.Buck.Bunny","Elephants.Dream"],
        "files"=>&["Files","FILE","SIZE","DONE","PRIO","Subtitles/English.srt","42%","include","skip"],
        "providers"=>&["Search providers","EZTV","YTS","The Pirate Bay","nova"],
        "settings"=>&["Download settings","Simultaneous downloads","Peer encryption","VPN connection","Reduce motion","Peer port","Download destination","Seed by default","Idle exit","Clipboard offers","Enter save","Esc cancel"],
        "prompt"=>&["Add magnet link or URL","streaming automatic","seed:off","magnet:?xt=urn:btih:"],
        "help"=>&["GLOBAL","Torrents","Search","Shows","RSS","Settings","Log"],
        "confirm"=>&["DELETE","AND ITS FILES? y/N"],
        "details"=>&["[Overview]","Files","state","progress","speed","peers","upload","path","downloading","42.0%"],
        "poster"=>&["The Diplomat (2023)","TV series","7.9","Keri Russell"],
        "disk"=>&["! Disk","Not enough space","needed","free"],
        "clipboard"=>&["Add 'Sintel.2010.1080p'? (y/Esc)","(y) Add","(Esc) Dismiss"],
        "pieces"=>&["[Pieces]","1000 verified pieces","first/last","verified","missing","playable prefix"],
        "peers"=>&["[Peers]","ADDRESS","CLIENT","KIND","↓/s","↑/s","SEED","127.0.0.1:42001","127.0.0.1:42002","yes","no"],
        "details-trackers"=>&["[Trackers]","URL","STATUS","LAST","PEERS","! Error","✓ OK","Not contacted","fixture: not yet"],
        _=>panic!("missing surface behaviour: {surface}"),
    };
    for text in required {assert!(actual.contains(text),"{surface} {width}x{height} lost {text:?}:\n{actual}");}
    // frame_text independently checks every display-cell width, including CJK.
    assert_eq!(actual.lines().count(),height as usize);
    println!("verified {surface} at {width}x{height}: readable content, truthful states and controls");
}

macro_rules! goldens {
    ($($surface:ident),+ $(,)?) => { $(mod $surface {
        use super::*;
        #[test] fn narrow() { assert_render(stringify!($surface), 80, 24); }
        #[test] fn medium() { assert_render(stringify!($surface), 120, 30); }
        #[test] fn wide() { assert_render(stringify!($surface), 160, 45); }
    })+ };
}
goldens!(peers,clipboard,torrents, search, shows, rss, files, providers, settings, prompt, help, confirm, done, details, poster, disk, pieces);

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
        assert!(actual.contains(if stall {"4m00s buffered"}else{"30s buffered"}));
        assert!(actual.contains(if stall {"finishes in 42s"}else{"finishes in 18m00s"}));
        println!("{path}: truthful watch detail at 80x24; synthetic rendering only");
    }
}

#[test]
fn private_overview_explains_networking_only_for_known_private_metainfo() {
    for (private,known) in [(true,true),(false,true),(true,false)] {
        let mut app=fixture::app("details");
        let mut value=serde_json::json!({"hash":app.torrents[0].hash,"name":"Private fixture","state":"paused","intent":"pause","staged":false,"checked":false,"seed":false,"upload_policy":"default_cap","save_path":"/fixture/downloads","added_at":1792281600,"force":false,"private":private});
        if known {value["size"]=serde_json::json!(1024);}
        let detail:crate::api::TorrentDetail=serde_json::from_value(value).unwrap();
        let hash=detail.torrent.hash.clone();
        app.store.reply(&crate::api::Call::TorrentsGet {args:crate::api::HashArgs {hash}},crate::api::Reply::TorrentsGet(crate::api::GetResult {torrent:detail})).unwrap();
        for width in [80,120,160] {
            let text=render(&app,width,30);
            assert_eq!(text.contains("no DHT, no PeX"),private && known,"private={private}; known={known}; width={width}\n{text}");
            assert_eq!(text.contains("Public torrent"),!private && known);
        }
        let _=app.close_details();assert!(app.store.details.is_empty(),"closing releases immutable detail state");
    }
}

#[test]
fn details_trackers_medium() {assert_render("details-trackers",120,30);}

#[test]
fn shows_quality_key_and_independent_inbox_filters() {
    let mut app=fixture::app("shows");
    app.select_show_parent(1);
    let original=app.selected_show().unwrap().quality.clone();
    let _=app.on_key(KeyEvent::new(KeyCode::Char('=')));
    assert!(matches!(app.prompt.as_ref().unwrap().kind,PromptKind::ShowQuality {..}));
    app.prompt.as_mut().unwrap().input.set_value("4k");
    let _=app.on_prompt_key(KeyEvent::new(KeyCode::Enter));
    assert!(!app.service_busy && app.selected_show().unwrap().quality==original,"invalid preference never sends or edits a show");
    let _=app.on_key(KeyEvent::new(KeyCode::Char('=')));
    app.prompt.as_mut().unwrap().input.set_value("720p");
    let command=app.on_prompt_key(KeyEvent::new(KeyCode::Enter));
    assert!(matches!(command,Cmd::Task(..)) && app.service_busy);
    assert_eq!(app.selected_show().unwrap().quality,original,"preference waits for confirmed daemon state");
    app.service_busy=false;
    app.shows.shows[0].episodes[0].quality="720p".into();
    let key=app.shows.shows[0].episodes[0].key.clone();app.shows.shows[0].seen.insert(key);
    for (filter,count) in [("unread all",1),("all all",2),("read 720p",1),("unread 720p",0),("all 1080p",1)] {
        let _=app.apply_show_filters(filter);assert_eq!(app.inbox_rows.len(),count,"{filter}");
        assert!(app.shows_inbox());
    }
    let _=app.apply_show_filters("read 720p");
    let rendered=render(&app,120,30);assert!(rendered.contains("read 720p"));
    let _=app.apply_show_filters("all 4k");assert!(app.show_controls.read==ReadFilter::Read);
}
