//! Limits remain drafts until a confirmed API write; the header mirrors pushes.
use super::*;
#[path = "fixture.rs"]
mod fixture;
fn summary() -> crate::api::Summary {
    let row = include_str!("../../../contracts/ipc/v1/subscribe.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["frame"]["topic"] == "summary")
        .unwrap();
    serde_json::from_value(row["frame"]["data"].clone()).unwrap()
}
#[test]
fn turtle_header_waits_for_confirmed_summary_at_three_sizes() {
    let mut app = fixture::app("torrents");
    assert_eq!(app.turtle_on(), None);
    for on in [false, true, false] {
        let mut state = summary();
        state.alt = on;
        app.store.summary = Some(state);
        for (w, h) in [(80, 24), (120, 30), (160, 45)] {
            let pane = super::golden::render(&app, w, h);
            assert_eq!(pane.lines().next().unwrap().contains("󰢃 Turtle"), on);
            println!(
                "{w}x{h} confirmed alt={on}: {}",
                pane.lines().next().unwrap()
            );
        }
    }
    app.store.summary.as_mut().unwrap().alt = true;
    assert!(matches!(app.toggle_alt(), Cmd::Task(..)));
    assert!(app.alt_busy);
    assert_eq!(
        app.turtle_on(),
        Some(true),
        "sending a mutation never fabricates a pushed state"
    );
}
#[test]
fn rate_fields_parse_suffixes_cancel_and_do_not_save_the_draft() {
    let mut app = fixture::app("settings");
    app.settings
        .as_ref()
        .unwrap()
        .list
        .borrow_mut()
        .select(Some(4));
    app.on_key(KeyEvent::new(KeyCode::Right));
    assert_eq!(actions::focused(&app), actions::Scope::Prompt);
    for c in "64K".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c)));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter));
    assert!(app.prompt.is_none());
    assert_eq!(
        app.settings.as_ref().unwrap().cfg.download_limit_bps,
        Some(65536)
    );
    assert_eq!(app.cfg.download_limit_bps, None);
    for (input, want) in [("2M", Some(2097152)), ("", None), ("0", None)] {
        app.apply_limit_input(1, input);
        assert_eq!(app.settings.as_ref().unwrap().cfg.upload_limit_bps, want);
    }
    for bad in ["15K", "-1", "4294967296", "999999999M", "1.2M", "nonsense"] {
        app.apply_limit_input(0, bad);
        assert_eq!(
            app.settings.as_ref().unwrap().cfg.download_limit_bps,
            Some(65536)
        );
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    app.on_key(KeyEvent::new(KeyCode::Right));
    app.prompt.as_mut().unwrap().input.set_value("128K");
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert_eq!(
        app.settings.as_ref().unwrap().cfg.download_limit_bps,
        Some(65536)
    );
    let patch = settings_fields::settings_patch(
        &app.settings.as_ref().unwrap().original,
        &app.settings.as_ref().unwrap().cfg,
    );
    assert_eq!(patch, serde_json::json!({"download_limit_bps":65536}));
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert!(app.settings.is_none());
    assert_eq!(app.cfg.download_limit_bps, None);
}
#[test]
fn limit_patch_omits_unedited_fields_and_can_clear_one() {
    let original = Config {
        download_limit_bps: Some(65536),
        ..Default::default()
    };
    let mut draft = original.clone();
    draft.download_limit_bps = None;
    draft.alt_upload_limit_bps = Some(32768);
    assert_eq!(
        settings_fields::settings_patch(&original, &draft),
        serde_json::json!({"download_limit_bps":null,"alt_upload_limit_bps":32768})
    );
    assert_eq!(
        settings_fields::settings_patch(&original, &original),
        serde_json::json!({})
    );
}
