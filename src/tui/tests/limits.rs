//! Ordinary limits remain drafts; legacy alternate settings stay API-compatible.
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
fn retired_limits_mode_has_no_header_hint_or_key_at_three_sizes() {
    let mut app = fixture::app("torrents");
    for on in [false, true, false] {
        let mut state = summary();
        state.alt = on;
        app.store.summary = Some(state);
        for (w, h) in [(80, 24), (120, 30), (160, 45)] {
            let pane = super::golden::render(&app, w, h);
            assert!(!pane.contains("Turtle") && !pane.contains("(z)"));
            println!(
                "{w}x{h} confirmed alt={on}: {}",
                pane.lines().next().unwrap()
            );
        }
    }
    assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('z'))), Cmd::None));
    assert!(!actions::all().any(|action|action.id=="limits.toggle"));
    assert!(!actions::help_text().contains("Turtle"));
    assert_eq!(app.store.summary.as_ref().unwrap().alt, false);
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
    assert_eq!(
        settings_fields::settings_patch(&original, &draft),
        serde_json::json!({"download_limit_bps":null})
    );
    assert_eq!(
        settings_fields::settings_patch(&original, &original),
        serde_json::json!({})
    );
}

#[test]
fn ordinary_settings_preserve_legacy_limits_and_follow_shared_updates() {
    let mut app = fixture::app("settings");
    let remote = Config {
        save_path: Some("/fixture/downloads".into()),
        network_interface: None,
        alt_limits_on: true,
        download_limit_bps: Some(65536), upload_limit_bps: Some(32768),
        alt_download_limit_bps: Some(131072), alt_upload_limit_bps: Some(65536),
        ..Default::default()
    };
    let mut settings=crate::api::ops::settings::view(&remote);
    app.sync_settings_snapshot(&settings).unwrap();
    app.apply_limit_input(0, "256K");
    settings.upload_limit_bps = Some(98304);
    settings.alt_download_limit_bps = Some(196608);
    app.sync_settings_snapshot(&settings).unwrap();
    let modal = app.settings.as_ref().unwrap();
    assert_eq!(modal.cfg.download_limit_bps, Some(262144));
    assert_eq!(modal.cfg.upload_limit_bps, Some(98304));
    assert_eq!(modal.cfg.alt_download_limit_bps, Some(196608));
    assert!(modal.cfg.alt_limits_on);
    assert_eq!(settings_fields::settings_patch(&modal.original, &modal.cfg),
        serde_json::json!({"download_limit_bps":262144}));
    let pane=super::golden::render(&app,80,24);
    assert!(pane.contains("Download limit") && pane.contains("Upload limit"));
    assert!(!pane.contains("Turtle"));
    app.apply_limit_input(2, "512K");
    assert_eq!(app.settings.as_ref().unwrap().cfg.alt_download_limit_bps, Some(196608));
}
