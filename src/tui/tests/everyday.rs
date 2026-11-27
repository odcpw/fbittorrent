//! Everyday commands preserve row identity, confirmed state and payload safety.
use super::*;
use crate::api;
#[path="fixture.rs"] mod fixture;
fn key(app:&mut TuiApp,c:char)->Cmd<Msg> {app.on_key(KeyEvent::new(KeyCode::Char(c)))}
fn summary()->api::Summary {
    include_str!("../../../contracts/ipc/v1/subscribe.ndjson").lines()
        .map(|s|serde_json::from_str::<serde_json::Value>(s).unwrap())
        .find(|v|v["frame"]["topic"]=="summary")
        .map(|v|serde_json::from_value(v["frame"]["data"].clone()).unwrap()).unwrap()
}
#[test]
fn sort_and_reverse_keep_the_selected_search_identity_and_unknowns_last() {
    let mut app=fixture::app("search");app.search_focus_input=false;
    app.search_results[1].seeders=-1;
    app.search_list.borrow_mut().select(Some(4));
    let selected=app.selected_result().unwrap().file_url.clone();
    key(&mut app,'s');
    assert_eq!(app.search_sort,store::SearchSort::Name);
    assert_eq!(app.selected_result().unwrap().file_url,selected);
    key(&mut app,'S');
    assert!(app.search_descending);
    assert_eq!(app.selected_result().unwrap().file_url,selected);
    app.sort_search_column(Column::Seeds);
    assert_eq!(app.search_results.last().unwrap().seeders,-1);
    key(&mut app,'S');
    assert_eq!(app.search_results.last().unwrap().seeders,-1,"unknown is last in either direction");
    assert!(matches!(key(&mut app,'y'),Cmd::Task(..)));
    app.search_focus_input=true;app.search_input.set_focused(true);app.search_input.set_value("");
    for c in ['s','S','y'] {assert!(!matches!(key(&mut app,c),Cmd::Task(..)));}
    assert_eq!(app.search_input.value(),"sSy","editing never triggers sort or copy");
}
#[test]
fn release_sorts_preserve_parent_and_key_and_keep_unknowns_last() {
    let mut app=fixture::app("shows");app.shows_focus_eps=true;app.rebuild_inbox();
    let selected=app.selected_release_target().unwrap();
    key(&mut app,'s');
    assert_eq!(app.release_sort,release_sort::ReleaseSort::Name);
    assert_eq!(app.selected_release_target(),Some(selected.clone()));
    key(&mut app,'S');assert!(app.release_descending);
    assert_eq!(app.selected_release_target(),Some(selected));
    assert!(matches!(key(&mut app,'y'),Cmd::Task(..)));
    let mut known=app.shows.shows[0].episodes[0].clone();known.seeds=4;
    let mut unknown=known.clone();unknown.seeds=-1;
    for reverse in [false,true] {assert_eq!(release_sort::ReleaseSort::Seeds.compare(&known,&unknown,reverse),std::cmp::Ordering::Less);}
}
#[test]
fn settings_destination_idle_and_clipboard_are_cancelable_drafts() {
    let mut app=fixture::app("settings");
    let original=app.cfg.clone();
    app.settings.as_ref().unwrap().list.borrow_mut().select(Some(settings_fields::DESTINATION_ROW));
    app.on_key(KeyEvent::new(KeyCode::Right));
    assert_eq!(app.prompt.as_ref().unwrap().kind,PromptKind::SettingDestination);
    app.prompt.as_mut().unwrap().input.set_value("/tmp/fbt-settings-draft");
    app.on_key(KeyEvent::new(KeyCode::Enter));
    assert_eq!(app.settings.as_ref().unwrap().cfg.save_path.as_deref(),Some("/tmp/fbt-settings-draft"));
    app.apply_destination("relative");assert!(!app.settings.as_ref().unwrap().error.is_empty());
    app.apply_idle_exit("0");assert_eq!(app.settings.as_ref().unwrap().cfg.idle_exit_secs,0);
    app.apply_idle_exit("-1");assert_eq!(app.settings.as_ref().unwrap().cfg.idle_exit_secs,0);
    assert!(!app.settings.as_ref().unwrap().error.is_empty());
    app.settings.as_ref().unwrap().list.borrow_mut().select(Some(settings_fields::CLIPBOARD_ROW));
    app.on_key(KeyEvent::new(KeyCode::Right));
    let draft=&app.settings.as_ref().unwrap().cfg;
    assert_eq!(settings_fields::settings_patch(&original,draft),serde_json::json!({"save_path":"/tmp/fbt-settings-draft","idle_exit_secs":0,"clipboard_watch":!original.clipboard_watch}));
    app.edit_idle_exit();app.prompt.as_mut().unwrap().input.set_value("900");
    app.on_key(KeyEvent::new(KeyCode::Escape));assert_eq!(app.settings.as_ref().unwrap().cfg.idle_exit_secs,0);
    app.on_key(KeyEvent::new(KeyCode::Escape));assert!(app.settings.is_none());
    assert_eq!(serde_json::to_value(&app.cfg).unwrap(),serde_json::to_value(original).unwrap());
}
#[test]
fn open_settings_follows_remote_fields_and_preserves_only_local_edits() {
    let mut app=fixture::app("settings");
    let original=app.cfg.clone();
    let local=original.max_active_downloads+1;
    app.on_key(KeyEvent::new(KeyCode::Right));
    assert_eq!(app.settings.as_ref().unwrap().cfg.max_active_downloads,local);
    let mut remote=original.clone();
    remote.upload="on".into();
    remote.download_limit_bps=Some(131072);
    remote.max_active_downloads=local+3;
    app.sync_settings_snapshot(&api::ops::settings::view(&remote)).unwrap();
    let modal=app.settings.as_ref().unwrap();
    assert_eq!(app.cfg.max_active_downloads,remote.max_active_downloads);
    assert_eq!(modal.cfg.max_active_downloads,local,"keep the deliberate local edit");
    assert_eq!(modal.cfg.upload,"on","untouched upload follows the CLI");
    assert_eq!(modal.cfg.download_limit_bps,Some(131072));
    assert_eq!(settings_fields::settings_patch(&modal.original,&modal.cfg),serde_json::json!({"max_active_downloads":local}));
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert_eq!(app.cfg.max_active_downloads,remote.max_active_downloads,"Cancel discards the draft, never remote state");
    app.show_settings(app.cfg.clone());
    remote.max_active_downloads+=1;
    app.sync_settings_snapshot(&api::ops::settings::view(&remote)).unwrap();
    let modal=app.settings.as_ref().unwrap();
    assert_eq!(modal.cfg.max_active_downloads,remote.max_active_downloads);
    assert_eq!(settings_fields::settings_patch(&modal.original,&modal.cfg),serde_json::json!({}));
}
#[test]
fn removal_undo_requires_confirmed_keep_files_receipt_and_expires() {
    let mut app=fixture::app("torrents");
    assert!(!app.can_undo_removal());
    let hash=app.torrents[0].hash.to_string();
    let until=crate::shows::now() as u64+60;
    app.service_done(None,"torrents.remove",serde_json::json!({"hashes":[hash],"delete_files":false}),"Removed".into(),Ok(serde_json::json!({"state":"done","undo_until":until})));
    assert!(app.can_undo_removal());
    assert!(matches!(key(&mut app,'U'),Cmd::Task(..)));
    assert!(app.removal_undo.is_none(),"an outgoing restore cannot be retried implicitly");
    app.service_done(None,"torrents.remove",serde_json::json!({"hashes":[hash],"delete_files":true}),"Deleted".into(),Ok(serde_json::json!({"state":"pending","undo_until":until})));
    assert!(!app.can_undo_removal(),"deleting payloads can never advertise Undo");
    app.service_busy=false;app.removal_undo=Some((hash,crate::shows::now() as u64-1));
    assert!(!app.can_undo_removal());
    assert!(!matches!(key(&mut app,'U'),Cmd::Task(..)));
}
#[test]
fn top_bar_uses_idle_vpn_unread_and_errors_without_retired_limits_mode() {
    let mut app=fixture::app("torrents");
    let mut state=summary();let mut counts=serde_json::to_value(&state.counts).unwrap();
    for count in counts.as_object_mut().unwrap().values_mut() {*count=serde_json::json!(0);}
    state.counts=serde_json::from_value(counts).unwrap();
    state.dl_bps=Some(0);state.ul_bps=Some(0);state.daemon.busy_reasons=vec!["interactive".into()];
    state.daemon.idle_exit_in_s=Some(210);state.alt=false;
    app.store.summary=Some(state);app.sync_activity_label();
    for (width,height) in [(80,24),(120,30),(160,45)] {
        let pane=super::golden::render(&app,width,height);let header=pane.lines().take(3).collect::<Vec<_>>().join(" ");
        assert!(header.contains("Idle · UI open"),"{header}");
        assert!(header.contains("Shows (2)")&&header.contains("RSS (2)"),"{header}");
        assert!(!header.contains(&app.theme.name));
    }
    app.store.summary.as_mut().unwrap().network.state=api::NetworkReadiness::Waiting;
    app.sync_activity_label();assert_eq!(app.activity_text.as_deref(),Some("Waiting for VPN"));
    app.store.summary.as_mut().unwrap().alt=true;
    let pane=super::golden::render(&app,120,30);let header=pane.lines().take(3).collect::<Vec<_>>().join(" ");
    assert!(header.contains("Waiting for VPN")&&!header.contains("Turtle"),"{header}");
    app.store.summary.as_mut().unwrap().network.state=api::NetworkReadiness::Ready;
    app.store.summary.as_mut().unwrap().daemon.idle_exit_in_s=None;
    app.store.summary.as_mut().unwrap().counts.error=2;
    app.sync_activity_label();assert_eq!(app.activity_text.as_deref(),Some("! 2 errors"));
}
#[test]
fn follow_affordance_uses_tv_identity_and_does_not_offer_movies() {
    let mut app=fixture::app("torrents");
    assert!(app.followable_title(&app.torrents[0].name));
    assert!(!app.followable_title("Sintel.2010.1080p"));
    let name=app.torrents[0].name.clone();super::poster_tests::ready(&mut app,&name);
    assert!(app.followable_title(&name));
    if let Some(ImdbState::Ready(title,_))=app.imdb.get_mut("tt17491088") {title.kind="Movie".into();}
    assert!(!app.followable_title(&name),"resolved identity beats the filename hint");
}

mod commands {
use super::*;
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
    // Navigate the combined catalogue to Size; preserve the numeric-order assertions below.
    for _ in 0..7 {
        if app.search_sort==store::SearchSort::Size {break;}
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
    let hash = app.selected_torrent().unwrap().hash.to_string();
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
        Ok(serde_json::json!({"undo_until":crate::shows::now()+10})),
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
fn settings_destination_and_idle_are_validated_cancelable_minimal_drafts() {
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
        .select(Some(settings_fields::UPLOAD_ROW));
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
    assert_eq!(draft.upload,"on");
    let original = &app.settings.as_ref().unwrap().original;
    assert_eq!(
        settings_fields::settings_patch(original, draft),
        serde_json::json!({"save_path":"/private/new downloads","idle_exit_secs":0,"upload":"on","clipboard_watch":true})
    );
    for bad in ["", "-1", "29", "18446744073709551616"] {
        app.apply_idle_exit(bad);
        assert_eq!(app.settings.as_ref().unwrap().cfg.idle_exit_secs, 0);
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    for bad in ["", "relative", "/bad\npath"] {
        app.apply_destination(bad);
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    for (w, h) in [(80, 24), (120, 30), (160, 45)] {
        let text = crate::tui::golden::render(&app, w, h);
        for field in [
            "Download destination",
            "Upload: on",
            "Idle exit",
            "Clipboard offers",
        ] {
            assert!(text.contains(field), "{field} in {text}");
        }
        assert!(!text.contains("Seed by default"), "the obsolete seed-by-default setting stays absent");
    }
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert_eq!(app.cfg.upload,"off");
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
        let text = crate::tui::golden::render(&app, w, h);
        let bar = text.lines().take(3).collect::<Vec<_>>().join(" ");
        assert!(bar.contains("Idle · stops in 4 min"), "{bar}");
        assert!(bar.contains("Shows"));
        assert!(bar.contains("RSS (2)"));
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
    assert!(!app.followable_title(&name));
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
    assert!(app.followable_title(&name));
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
    assert!(!app.followable_title(&name));
    assert!(app.followable_title("A.Show.S01E02.1080p"));
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

}
