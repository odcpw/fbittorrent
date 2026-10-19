//! Everyday keys operate on identities, drafts, and confirmed optional values.
use super::*;
#[path = "fixture.rs"]
mod fixture;
fn key(app: &mut TuiApp, c: char) -> Cmd<Msg> {
    app.on_key(KeyEvent::new(KeyCode::Char(c)))
}
fn summary() -> crate::api::Summary {
    let row = include_str!("../../../contracts/ipc/v1/subscribe.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["frame"]["topic"] == "summary")
        .unwrap();
    serde_json::from_value(row["frame"]["data"].clone()).unwrap()
}
#[test]
fn keyboard_sorts_preserve_identity_and_reverse_numeric_values_with_unknown_last() {
    let mut app = fixture::app("torrents");
    let selected = app.selected_torrent().unwrap().hash.clone();
    key(&mut app, 'S');
    assert!(!app.torrent_descending);
    assert_eq!(app.torrent_sort, store::TorrentSort::Added);
    key(&mut app, 's');
    assert_eq!(app.torrent_sort, store::TorrentSort::Name);
    assert_eq!(app.selected_torrent().unwrap().hash, selected);
    let mut app = fixture::app("search");
    app.search_results.truncate(3);
    for (r, size) in app.search_results.iter_mut().zip([900, 1100, -1]) {
        r.file_size = size;
    }
    let selected = app.selected_result().unwrap().file_url.clone();
    // Seeds -> rating -> uploaded -> provider -> peers -> name -> size.
    for _ in 0..6 {
        key(&mut app, 's');
    }
    assert_eq!(app.search_sort, store::SearchSort::Size);
    assert_eq!(
        app.search_results
            .iter()
            .map(|r| r.file_size)
            .collect::<Vec<_>>(),
        [1100, 900, -1]
    );
    key(&mut app, 'S');
    assert_eq!(
        app.search_results
            .iter()
            .map(|r| r.file_size)
            .collect::<Vec<_>>(),
        [900, 1100, -1]
    );
    assert_eq!(app.selected_result().unwrap().file_url, selected);
    key(&mut app, '/');
    app.search_input.set_value("");
    key(&mut app, 's');
    key(&mut app, 'S');
    key(&mut app, 'y');
    assert_eq!(
        app.search_input.value(),
        "sSy",
        "query editing keeps printable commands literal"
    );
}
#[test]
fn copy_handles_missing_provider_magnet_without_claiming_success() {
    let mut app = fixture::app("search");
    assert!(matches!(key(&mut app, 'y'), Cmd::Task(..)));
    let i = app.search_list.borrow().selected().unwrap();
    app.search_results[i].file_url = "https://provider.invalid/details".into();
    assert!(matches!(key(&mut app, 'y'), Cmd::None));
    assert!(app.status_is_error && app.status.contains("No magnet"));
}
#[test]
fn removal_undo_is_granted_by_confirmation_never_by_delete_or_failure() {
    let mut app = fixture::app("torrents");
    let hash = app.selected_torrent().unwrap().hash.clone();
    key(&mut app, 'd');
    assert!(app.removal_undo.is_none());
    app.on_key(KeyEvent::new(KeyCode::Enter));
    assert!(app.prompt.is_none(), "default remains No");
    key(&mut app, 'd');
    assert!(matches!(key(&mut app, 'y'), Cmd::Task(..)));
    assert!(app.removal_undo.is_none());
    let args = serde_json::json!({"hashes":[hash],"delete_files":false});
    app.service_done(
        None,
        "torrents.remove",
        args.clone(),
        "removed".into(),
        Err("refused".into()),
    );
    assert!(app.removal_undo.is_none());
    app.service_done(
        None,
        "torrents.remove",
        args,
        "removed".into(),
        Ok(serde_json::json!({"undo_until":shows::now()+10})),
    );
    assert_eq!(app.removal_undo.as_ref().unwrap().0, hash);
    assert!(matches!(key(&mut app, 'U'), Cmd::Task(..)));
    app.service_done(
        None,
        "torrents.add",
        serde_json::json!({"source":{"restore":hash}}),
        "restored".into(),
        Ok(serde_json::json!({"state":"done"})),
    );
    assert!(app.removal_undo.is_none());
    app.service_done(
        None,
        "torrents.remove",
        serde_json::json!({"hashes":[hash],"delete_files":true}),
        "deleted".into(),
        Ok(serde_json::json!({"state":"done"})),
    );
    assert!(app.removal_undo.is_none());
    app.removal_undo = Some((hash, 1));
    key(&mut app, 'U');
    assert!(app.removal_undo.is_none());
    assert!(app.status.contains("expired"));
}
#[test]
fn releases_sort_without_retargeting_and_quality_captures_the_show() {
    let mut app = fixture::app("shows");
    app.select_show_parent(1);
    app.shows_focus_eps = true;
    let mut a = app.shows.shows[0].episodes[0].clone();
    a.key = "S01E02".into();
    a.size = 1100;
    let mut b = a.clone();
    b.key = "S01E03".into();
    b.size = -1;
    app.shows.shows[0].episodes[0].size = 900;
    app.shows.shows[0].episodes.extend([a, b]);
    app.ep_list.borrow_mut().select(Some(0));
    let target = app.selected_release_target().unwrap();
    key(&mut app, 's');
    key(&mut app, 's');
    assert_eq!(app.release_sort, release_sort::ReleaseSort::Size);
    assert_eq!(
        app.shows.shows[0]
            .episodes
            .iter()
            .map(|e| e.size)
            .collect::<Vec<_>>(),
        [1100, 900, -1]
    );
    key(&mut app, 'S');
    assert_eq!(
        app.shows.shows[0]
            .episodes
            .iter()
            .map(|e| e.size)
            .collect::<Vec<_>>(),
        [900, 1100, -1]
    );
    assert_eq!(app.selected_release_target(), Some(target));
    assert!(matches!(key(&mut app, 'y'), Cmd::Task(..)));
    key(&mut app, '=');
    let expected = app.release_show().unwrap().imdb.clone();
    assert_eq!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::ShowQuality { imdb: expected }
    );
    app.prompt.as_mut().unwrap().input.set_value("4k");
    app.on_key(KeyEvent::new(KeyCode::Enter));
    assert!(app.status_is_error && !app.service_busy);
    key(&mut app, '=');
    app.prompt.as_mut().unwrap().input.set_value("2160p");
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Enter)),
        Cmd::Task(..)
    ));
    assert!(
        app.shows.shows[0].quality.is_none(),
        "only the push changes confirmed quality"
    );
}
#[test]
fn settings_destination_seed_and_idle_are_validated_cancelable_minimal_drafts() {
    let mut app = fixture::app("settings");
    for (index, value) in [
        (settings_fields::DESTINATION_ROW, "/private/new downloads"),
        (settings_fields::IDLE_ROW, "0"),
    ] {
        app.settings
            .as_ref()
            .unwrap()
            .list
            .borrow_mut()
            .select(Some(index));
        app.on_key(KeyEvent::new(KeyCode::Right));
        app.prompt.as_mut().unwrap().input.set_value(value);
        app.on_key(KeyEvent::new(KeyCode::Enter));
    }
    app.settings
        .as_ref()
        .unwrap()
        .list
        .borrow_mut()
        .select(Some(settings_fields::SEED_ROW));
    app.on_key(KeyEvent::new(KeyCode::Right));
    app.settings
        .as_ref()
        .unwrap()
        .list
        .borrow_mut()
        .select(Some(settings_fields::CLIPBOARD_ROW));
    app.on_key(KeyEvent::new(KeyCode::Right));
    let draft = &app.settings.as_ref().unwrap().cfg;
    assert_eq!(draft.save_path.as_deref(), Some("/private/new downloads"));
    assert_eq!(draft.idle_exit_secs, 0);
    assert!(draft.seed_by_default);
    let original = &app.settings.as_ref().unwrap().original;
    assert_eq!(
        settings_fields::settings_patch(original, draft),
        serde_json::json!({"save_path":"/private/new downloads","idle_exit_secs":0,"seed_by_default":true,"clipboard_watch":true})
    );
    for bad in ["", "-1", "29", "18446744073709551616"] {
        app.apply_idle_input(bad);
        assert_eq!(app.settings.as_ref().unwrap().cfg.idle_exit_secs, 0);
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    for bad in ["", "relative", "/bad\npath"] {
        app.apply_destination_input(bad);
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    for (w, h) in [(80, 24), (120, 30), (160, 45)] {
        let text = super::golden::render(&app, w, h);
        for field in [
            "Destination",
            "Seed new torrents",
            "Idle exit",
            "Clipboard offers",
        ] {
            assert!(text.contains(field), "{field} in {text}");
        }
    }
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert!(!app.cfg.seed_by_default);
    assert_eq!(app.cfg.idle_exit_secs, 300);
}
#[test]
fn status_uses_confirmed_idle_vpn_and_rates_and_narrow_badges() {
    let mut app = fixture::app("torrents");
    let mut s = summary();
    s.network.state = crate::api::NetworkReadiness::Ready;
    s.dl_bps = Some(0);
    s.ul_bps = Some(0);
    s.daemon.busy_reasons.clear();
    s.daemon.idle_exit_in_s = Some(240);
    s.alt = false;
    app.store.summary = Some(s);
    for (w, h) in [(80, 24), (120, 30), (160, 45)] {
        let text = super::golden::render(&app, w, h);
        let bar = text.lines().next().unwrap();
        assert!(bar.contains("Idle · stops in 4 min"), "{bar}");
        assert!(bar.contains("Shows"));
        assert!(bar.contains("RSS(2)"));
        assert!(!bar.contains(&format!(" {} ", app.theme.name)));
    }
    app.store.summary.as_mut().unwrap().daemon.busy_reasons = vec!["interactive".into()];
    assert_eq!(app.header_status(80), "Idle · UI open");
    app.store.summary.as_mut().unwrap().network.state = crate::api::NetworkReadiness::Waiting;
    app.store.summary.as_mut().unwrap().dl_bps = Some(999);
    assert_eq!(app.header_status(80), "Waiting for VPN");
    app.store.summary.as_mut().unwrap().network.state = crate::api::NetworkReadiness::Ready;
    app.store.summary.as_mut().unwrap().dl_bps = None;
    app.store.summary.as_mut().unwrap().ul_bps = None;
    assert_eq!(app.header_status(80), "↓ —  ↑ —");
}
#[test]
fn follow_is_offered_for_tv_titles_and_never_known_films() {
    let mut app = fixture::app("search");
    let name = app.selected_result().unwrap().file_name.clone();
    assert!(!app.can_follow_title(&name));
    app.cache_imdb(
        name.clone(),
        ImdbState::Ready(
            Box::new(Title {
                kind: "tvSeries".into(),
                ..Default::default()
            }),
            None,
        ),
    );
    assert!(app.can_follow_title(&name));
    app.cache_imdb(
        name.clone(),
        ImdbState::Ready(
            Box::new(Title {
                kind: "movie".into(),
                ..Default::default()
            }),
            None,
        ),
    );
    assert!(!app.can_follow_title(&name));
    assert!(app.can_follow_title("A.Show.S01E02.1080p"));
}

#[test]
fn rss_sort_preserves_article_and_unknown_size_stays_last_after_reverse() {
    let mut app = fixture::app("rss");
    for (a, size) in app.feeds[0]
        .articles
        .iter_mut()
        .zip([Some(900), Some(1100), None])
    {
        a.size = size;
    }
    app.item_list.borrow_mut().select(Some(0));
    let selected = app.feeds[0].articles[0].id.clone();
    key(&mut app, 's');
    key(&mut app, 's');
    assert_eq!(
        app.feeds[0]
            .articles
            .iter()
            .map(|a| a.size)
            .collect::<Vec<_>>(),
        [Some(1100), Some(900), None]
    );
    key(&mut app, 'S');
    assert_eq!(
        app.feeds[0]
            .articles
            .iter()
            .map(|a| a.size)
            .collect::<Vec<_>>(),
        [Some(900), Some(1100), None]
    );
    assert_eq!(
        app.feeds[0].articles[app.item_list.borrow().selected().unwrap()].id,
        selected
    );
}
