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
    assert_eq!(app.release_sort,release_sort::ReleaseSort::Title);
    assert_eq!(app.selected_release_target(),Some(selected.clone()));
    key(&mut app,'S');assert!(app.release_descending);
    assert_eq!(app.selected_release_target(),Some(selected));
    assert!(matches!(key(&mut app,'y'),Cmd::Task(..)));
    let mut known=app.shows.shows[0].episodes[0].clone();known.seeds=4;
    let mut unknown=known.clone();unknown.seeds=-1;
    for reverse in [false,true] {assert_eq!(release_sort::episode_order(&known,&unknown,release_sort::ReleaseSort::Seeds,reverse),std::cmp::Ordering::Less);}
}
#[test]
fn settings_destination_seed_idle_and_clipboard_are_cancelable_drafts() {
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
fn removal_undo_requires_confirmed_keep_files_receipt_and_expires() {
    let mut app=fixture::app("torrents");
    assert!(!app.can_undo_removal());
    let hash=app.torrents[0].hash.clone();
    let until=shows::now() as u64+60;
    app.service_done(None,"torrents.remove",serde_json::json!({"hashes":[hash],"delete_files":false}),"Removed".into(),Ok(serde_json::json!({"state":"done","undo_until":until})));
    assert!(app.can_undo_removal());
    assert!(matches!(key(&mut app,'U'),Cmd::Task(..)));
    assert!(app.removal_undo.is_none(),"an outgoing restore cannot be retried implicitly");
    app.service_done(None,"torrents.remove",serde_json::json!({"hashes":[hash],"delete_files":true}),"Deleted".into(),Ok(serde_json::json!({"state":"pending","undo_until":until})));
    assert!(!app.can_undo_removal(),"deleting payloads can never advertise Undo");
    app.service_busy=false;app.removal_undo=Some((hash,shows::now() as u64-1));
    assert!(!app.can_undo_removal());
    assert!(!matches!(key(&mut app,'U'),Cmd::Task(..)));
}
#[test]
fn top_bar_uses_idle_vpn_unread_and_errors_without_hiding_turtle() {
    let mut app=fixture::app("torrents");
    let mut state=summary();let mut counts=serde_json::to_value(&state.counts).unwrap();
    for count in counts.as_object_mut().unwrap().values_mut() {*count=serde_json::json!(0);}
    state.counts=serde_json::from_value(counts).unwrap();
    state.dl_bps=Some(0);state.ul_bps=Some(0);state.daemon.busy_reasons=vec!["interactive".into()];
    state.daemon.idle_exit_in_s=Some(210);state.alt=false;
    app.store.summary=Some(state);app.sync_activity_label();
    for (width,height) in [(80,24),(120,30),(160,45)] {
        let pane=super::golden::render(&app,width,height);let header=pane.lines().next().unwrap();
        assert!(header.contains("Idle · stops in"),"{header}");
        assert!(header.contains("Shows (2)")&&header.contains("RSS (2)"),"{header}");
        assert!(!header.contains(&app.theme.name));
    }
    app.store.summary.as_mut().unwrap().network.state=api::NetworkReadiness::Waiting;
    app.sync_activity_label();assert_eq!(app.activity_label(),Some("Waiting for VPN"));
    app.store.summary.as_mut().unwrap().alt=true;
    let pane=super::golden::render(&app,120,30);let header=pane.lines().next().unwrap();
    assert!(header.contains("Waiting for VPN")&&header.contains("Turtle"),"{header}");
    app.store.summary.as_mut().unwrap().network.state=api::NetworkReadiness::Ready;
    app.store.summary.as_mut().unwrap().daemon.idle_exit_in_s=None;
    app.store.summary.as_mut().unwrap().counts.error=2;
    app.sync_activity_label();assert_eq!(app.activity_label(),Some("! 2 errors"));
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
