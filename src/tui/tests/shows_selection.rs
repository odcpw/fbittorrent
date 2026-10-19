//! Metadata and detail subscriptions arrive independently; only complete rows act.
use super::*;
use serde_json::{json, Value};

const IMDB: &str = "tt1234001";

fn app() -> TuiApp {
    let mut app = TuiApp::new(Config::default(), Screen::Shows, Shows::default());
    app.kitty = None;
    let hello = include_str!("../../../contracts/ipc/v1/subscribe.ndjson")
        .lines().map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|line| line["frame"]["result"]["caps"].is_array()).unwrap();
    app.store.hello(serde_json::from_value(hello["frame"]["result"].clone()).unwrap());
    let show = serde_json::from_value(json!({"imdb":IMDB,"title":"Local Fixture",
        "unread":1,"added_at":1,"last_check_at":1792274400,
        "counts":{"episodes":1,"seen":0,"grabbed":0},"checking":false,
        "catalogue_complete":true})).unwrap();
    app.store.shows.insert(IMDB.into(), show);
    app.sync_native_shows();
    app
}

fn row(key: &str) -> Value {
    json!({"show_imdb":IMDB,"key":key,"title":format!("Local.Fixture.{key}.1080p"),
        "season_pack":false,"read":false,"transfer":"none"})
}

fn event(app: &mut TuiApp, value: Value) -> store::Effect {
    let effect = app.store.event(&serde_json::from_value(value).unwrap()).unwrap();
    if effect == store::Effect::Changed { app.sync_native_shows(); }
    effect
}

fn begin(app: &mut TuiApp, sub: u64, topic: &str, count: usize) {
    event(app, json!({"ev":"snapshot.begin","sub":sub,"topic":topic,
        "rev":1,"count":count,"reason":"subscribe"}));
}

fn snapshot(app: &mut TuiApp, sub: u64, topic: &str, rows: Vec<Value>) {
    begin(app, sub, topic, rows.len());
    event(app, json!({"ev":"snapshot.chunk","sub":sub,"topic":topic,"rev":2,"items":rows}));
    event(app, json!({"ev":"snapshot.end","sub":sub,"topic":topic,"rev":3}));
}

fn key(app: &mut TuiApp, code: KeyCode) -> Cmd<Msg> {
    app.on_shows_key(KeyEvent::new(code))
}

fn assert_loading(app: &TuiApp) {
    let pane = golden::render(app, 80, 24);
    assert!(pane.contains("Loading releases…"), "{pane}");
    assert!(!pane.contains("All caught up"), "{pane}");
    assert!(app.selected_episode().is_none());
}

#[test]
fn unread_count_before_snapshot_shows_loading_then_selects_the_arriving_release() {
    let mut app = app();
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Enter);
    assert_loading(&app);
    assert!(golden::render(&app, 80, 24).contains("All unread · 1 releases"));
    assert!(matches!(key(&mut app, KeyCode::Char('m')), Cmd::None));
    assert!(app.status.contains("Loading releases"));
    assert!(!app.service_busy, "an early mark must not be queued for an unseen row");
    begin(&mut app, 10, "shows.releases:inbox", 1);
    event(&mut app, json!({"ev":"snapshot.chunk","sub":10,"topic":"shows.releases:inbox",
        "rev":2,"items":[row("S01E01")]}));
    assert_loading(&app);
    event(&mut app, json!({"ev":"snapshot.end","sub":10,"topic":"shows.releases:inbox","rev":3}));
    assert_eq!(app.selected_release_target(), Some(ShowReleaseTarget {imdb:IMDB.into(),key:"S01E01".into()}));
    let pane = golden::render(&app, 80, 24);
    assert!(pane.contains("S01E01") && pane.contains("(m) Mark read"), "{pane}");
    assert!(matches!(key(&mut app, KeyCode::Char('m')), Cmd::Task(..)));
    assert_eq!(app.episode_count(), 1, "mark waits for the confirmed pushed removal");
}

#[test]
fn inbox_metadata_ahead_of_its_delta_does_not_claim_all_caught_up() {
    let mut app = app();
    app.store.shows.get_mut(IMDB).unwrap().unread = 0;
    snapshot(&mut app, 10, "shows.releases:inbox", vec![]);
    assert!(golden::render(&app, 80, 24).contains("All caught up"));
    app.store.shows.get_mut(IMDB).unwrap().unread = 1;
    app.sync_native_shows();
    assert_loading(&app);
    event(&mut app, json!({"ev":"delta","sub":10,"topic":"shows.releases:inbox",
        "rev":4,"added":[row("S01E01")]}));
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E01");
}

#[test]
fn switching_detail_topics_hides_cached_actions_and_selects_the_first_complete_row() {
    let mut app = app();
    snapshot(&mut app, 10, "shows.releases:inbox", vec![row("S01E01")]);
    app.select_show_parent(1);
    key(&mut app, KeyCode::Enter);
    assert_loading(&app);
    assert_eq!(app.episode_count(), 0, "the inbox slice is not a full show history");
    snapshot(&mut app, 11, &format!("shows.releases:{IMDB}"), vec![row("S01E01")]);
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E01");
    app.select_show_parent(0);
    key(&mut app, KeyCode::Enter);
    assert_loading(&app);
    // A late full-show update must not overwrite the user's chosen inbox parent.
    event(&mut app, json!({"ev":"delta","sub":11,"topic":format!("shows.releases:{IMDB}"),
        "rev":4,"added":[row("S01E02")]}));
    assert!(app.shows_inbox() && app.shows_focus_eps);
    assert_loading(&app);
    app.store.unsubscribe(11);
    snapshot(&mut app, 12, "shows.releases:inbox", vec![row("S01E01"),row("S01E02")]);
    assert!(app.shows_inbox() && app.shows_focus_eps);
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E01");
    app.ep_list.borrow_mut().select(Some(1));
    app.sync_native_shows();
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E02");
    event(&mut app, json!({"ev":"delta","sub":12,"topic":"shows.releases:inbox",
        "rev":4,"removed":[format!("{IMDB}/S01E02")]}));
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E01", "removing the selected row keeps a valid next target");
    assert!(matches!(key(&mut app, KeyCode::Char('m')), Cmd::Task(..)));
}

#[test]
fn retired_gapped_and_reconnected_topics_wait_for_a_fresh_complete_snapshot() {
    let mut app = app();
    snapshot(&mut app, 10, "shows.releases:inbox", vec![row("S01E01")]);
    app.store.unsubscribe(10);
    assert_loading(&app);
    snapshot(&mut app, 11, "shows.releases:inbox", vec![row("S01E01")]);
    let effect = event(&mut app, json!({"ev":"delta","sub":11,"topic":"shows.releases:inbox",
        "rev":5,"added":[row("S01E02")]}));
    assert!(matches!(effect, store::Effect::Resubscribe {sub:11,..}));
    assert_loading(&app);
    snapshot(&mut app, 12, "shows.releases:inbox", vec![row("S01E01")]);
    app.store.disconnected();
    assert_loading(&app);
    app.store.hello(app.store.hello.clone().unwrap());
    assert_loading(&app);
    snapshot(&mut app, 13, "shows.releases:inbox", vec![row("S01E01")]);
    assert_eq!(app.selected_episode().unwrap().1.key, "S01E01");
}

#[test]
fn filtered_inbox_reads_are_actionable_without_an_unread_subscription() {
    let mut app=app();
    app.show_controls.read=ReadFilter::All;
    assert!(!app.releases_ready());
    app.show_controls.rows=Some(vec![serde_json::from_value(row("S01E01")).unwrap()]);
    app.sync_native_shows();
    assert!(app.store.releases_topic.is_none());
    assert!(app.releases_ready());
    assert_eq!(app.selected_episode().unwrap().1.key,"S01E01");
    app.show_controls.quality=Some("720p".into());
    app.rebuild_inbox();
    assert_eq!(app.episode_count(),0);
    assert!(!golden::render(&app,80,24).contains("Loading releases…"),"a completed filter with no matches is not loading");
    app.store.disconnected();
    assert!(!app.releases_ready(),"cached full-inbox rows cannot remain actionable after disconnect");
}
