use super::*;
use crate::api;
#[path="fixture.rs"] mod fixture;

pub(super) fn prepare(app: &mut TuiApp) {
    let frame:serde_json::Value=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v|v["frame"]["result"]["caps"].is_array()).unwrap();
    app.store.hello(serde_json::from_value(frame["frame"]["result"].clone()).unwrap());
    app.store.stale=false;app.online=true;
    let torrent=app.torrents[0].clone();app.open_details(&torrent);
    app.details.as_mut().unwrap().tab=DetailsTab::Trackers;
    let row=|url: &str,status,message|api::Tracker{url:url.into(),status,tier:None,last_announce_at:Some(crate::shows::now().max(0) as u64),next_announce_at:None,peers:None,message};
    app.trackers_loaded(None,torrent.hash,0,false,Ok(vec![
        row("http://127.0.0.1:42001/announce?passkey=private-token",api::TrackerStatus::Error,Some("fixture: not yet http://127.0.0.1:42001/announce?passkey=private-token".into())),
        api::Tracker{peers:Some(1),..row("https://long-tracker.invalid/a-very-long-tracker-path/announce",api::TrackerStatus::Ok,None)},
        row("udp://unsupported.invalid/announce",api::TrackerStatus::NotContacted,None),
    ]));
}
#[test]
fn tracker_caps_labels_privacy_and_modal_action_scope() {
    let mut app=fixture::app("torrents");
    assert!(!app.details_tabs().contains(&DetailsTab::Trackers));
    prepare(&mut app);
    assert_eq!(app.details_tabs(),[DetailsTab::Overview,DetailsTab::Files,DetailsTab::Peers,DetailsTab::Trackers,DetailsTab::Log]);
    for (width,height) in [(80,24),(120,30),(160,45)] {
        let text=golden::render(&app,width,height);
        for expected in ["[Trackers]","! Error","✓ OK","…Not contacted","fixture: not yet","(e) Reannounce"] {assert!(text.contains(expected),"{expected} missing:\n{text}");}
        assert!(!text.contains("private-token")&&!text.contains("passkey=")&&!text.contains("/announce?"));
    }
    let active=app.footer_signature(120);
    app.details_tab(true);assert_eq!(app.details.as_ref().unwrap().tab,DetailsTab::Log);
    assert_ne!(app.footer_signature(120),active,"modal tab must invalidate the footer's action cache");
    let action=actions::all().find(|a|a.id=="trackers.reannounce").unwrap();assert!(!(action.enabled)(&app));
}
#[test]
fn stale_tracker_reads_and_late_closed_tab_results_are_ignored() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    let hash=app.details.as_ref().unwrap().hash.clone();
    let saved=app.details.as_ref().unwrap().trackers.rows.clone();
    app.trackers_loaded(Some(123),hash.clone(),0,false,Ok(vec![]));
    app.trackers_loaded(None,hash.clone(),42,false,Ok(vec![]));
    assert_eq!(app.details.as_ref().unwrap().trackers.rows,saved);
    assert!(matches!(app.load_trackers(false),Cmd::Task(..)));
    assert!(app.details.as_ref().unwrap().trackers.busy);
    let request=app.details.as_ref().unwrap().trackers.request;
    app.details_tab(true);assert!(!app.details.as_ref().unwrap().trackers.busy);
    app.trackers_loaded(None,hash,request,false,Ok(vec![]));
    assert_eq!(app.details.as_ref().unwrap().trackers.rows,saved);
    assert!(matches!(app.details_tab(false),Cmd::Task(..)),"returning to the tab can read even if an old read was in flight");
}
#[test]
fn statuses_use_shared_words_and_messages_remove_queries_and_echoed_values() {
    use api::TrackerStatus::*;
    for (status,expected) in [(Ok,"✓ OK"),(Error,"! Error"),(Updating,"⟳ Updating"),(NotContacted,"…Not contacted")] {
        assert_eq!(format::state::tracker_label(&status),expected);
    }
    let row=modals_trackers::public_tracker(api::Tracker{url:"https://tracker.invalid/announce?passkey=echo-me&token=another".into(),tier:None,status:Error,last_announce_at:None,next_announce_at:None,peers:None,message:Some("echo-me another https://other.invalid/error?key=secret".into())});
    assert_eq!(row.url,"https://tracker.invalid/announce");
    assert_eq!(row.message.as_deref(),Some("… … https://other.invalid/error"));
}

#[test]
fn tracker_url_elision_respects_display_width_and_keeps_both_ends() {
    for width in [14,20,30] {
        let value=modals_trackers::middle_url("http://例子.invalid/a-very-long-path/announce",width);
        assert_eq!(dw(&value),width);
        assert!(value.starts_with("http"));assert!(value.contains('…'));assert!(value.ends_with("announce".chars().rev().take((width-1)/2).collect::<Vec<_>>().into_iter().rev().collect::<String>().as_str()));
    }
}

#[test]
fn tracker_error_elision_keeps_the_failure_reason_before_the_url() {
    let text=modals_trackers::message_cell("tracker returned failure. Failure reason: fixture: not yet http://127.0.0.1:42001/announce",76);
    assert_eq!(dw(&text),76);
    assert!(text.contains("fixture: not yet"),"{text}");
    assert!(text.ends_with('…'));
}

#[test]
fn live_tracker_snapshot_is_atomic_and_a_late_read_cannot_replace_a_push() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    let hash=api::Hash::new(app.details.as_ref().unwrap().hash.clone()).unwrap();
    let topic=api::Topic::Trackers(hash.clone());assert_eq!(app.tracker_topic(),Some(topic.clone()));
    app.load_trackers(false);let request=app.details.as_ref().unwrap().trackers.request;
    let stale=app.details.as_ref().unwrap().trackers.rows.clone();
    let row=api::Tracker{url:"https://credential-user:credential-pass@tracker.invalid/announce?passkey=private-token".into(),tier:None,status:api::TrackerStatus::Ok,last_announce_at:Some(10),next_announce_at:None,peers:Some(1),message:None};
    app.store.event(&api::Event::SnapshotBegin{sub:123,topic:topic.clone(),rev:1,count:1,reason:api::SnapshotReason::Subscribe,oldest_seq:None}).unwrap();
    app.store.event(&api::Event::SnapshotChunk{sub:123,topic:topic.clone(),rev:2,items:vec![api::Item::Tracker(row.clone())]}).unwrap();
    app.sync_tracker_panel();assert_eq!(app.details.as_ref().unwrap().trackers.rows,stale,"partial snapshots cannot replace confirmed status");
    app.store.event(&api::Event::SnapshotEnd{sub:123,topic:topic.clone(),rev:3}).unwrap();app.sync_tracker_panel();
    app.trackers_loaded(None,hash.to_string(),request,false,Ok(stale));
    let panel=&app.details.as_ref().unwrap().trackers;
    assert_eq!(panel.rows.len(),1);assert_eq!(panel.rows[0].status,api::TrackerStatus::Ok);
    assert_eq!(panel.urls,[row.url.clone()],"private identity stays available only for explicit socket operations");
    let pane=golden::render(&app,80,24);
    for secret in ["credential-user","credential-pass","private-token","passkey="] {assert!(!pane.contains(secret));}
    app.details_tab(true);assert!(app.tracker_topic().is_none());assert!(app.store.trackers.is_empty());
}

#[test]
fn tracker_edit_prompts_mask_input_and_cancel_removal_by_default() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    app.store.hello.as_mut().unwrap().caps.push("tracker_edit".into());
    let key=|code|KeyEvent::new(code);
    app.on_key(key(KeyCode::Char('+')));
    assert!(matches!(app.prompt.as_ref().unwrap().kind,PromptKind::AddTracker{..}));
    app.prompt.as_mut().unwrap().input.set_value("http://private-user:private-password@tracker.invalid/a?key=private-token");
    let text=golden::render(&app,80,24);
    for secret in ["private-user","private-password","private-token","key="] {assert!(!text.contains(secret),"{text}");}
    assert!(text.contains("input hidden"));
    app.on_key(key(KeyCode::Escape));assert!(!app.service_busy);
    for cancel in [KeyCode::Enter,KeyCode::Escape,KeyCode::Char('n')] {
        app.on_key(key(KeyCode::Char('x')));
        assert!(matches!(app.prompt.as_ref().unwrap().kind,PromptKind::ConfirmRemoveTracker{..}));
        let text=golden::render(&app,80,24);assert!(text.contains("[Cancel]"));assert!(!text.contains("private-token"));
        assert!(matches!(app.on_key(key(cancel)),Cmd::None));assert!(!app.service_busy);
    }
    app.on_key(key(KeyCode::Char('x')));
    assert!(matches!(app.on_key(key(KeyCode::Char('y'))),Cmd::Task(..)));
    assert!(app.service_busy);assert!(app.prompt.is_none());
}

#[test]
fn unsupported_tracker_input_never_dispatches_or_echoes_credentials() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    app.store.hello.as_mut().unwrap().caps.push("tracker_edit".into());
    for value in ["file:///tmp/secret-passkey","ftp://user:secret-password@host/a","udp://host/a"] {
        app.prompt_add_tracker();app.prompt.as_mut().unwrap().input.set_value(value);
        assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Enter)),Cmd::None));assert!(!app.service_busy);
        let text=golden::render(&app,80,24);assert!(!text.contains(value));assert!(!text.contains("secret-password"));
    }
}

#[test]
fn tracker_push_keeps_the_selected_private_url_for_removal() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    app.store.hello.as_mut().unwrap().caps.push("tracker_edit".into());
    let details=app.details.as_mut().unwrap();details.trackers.offset=1;
    let selected=details.trackers.urls[1].clone();let hash=details.hash.clone();
    let mut row=details.trackers.rows[1].clone();row.url="http://before.invalid/a".into();
    let mut rows=details.trackers.rows.clone();rows.push(row);
    for row in &mut rows {if row.url==details.trackers.rows[1].url {row.url=selected.clone();}}
    app.store.trackers.insert(api::Hash::new(&hash).unwrap(),rows.into_iter().map(|r|(r.url.clone(),r)).collect());
    app.sync_tracker_panel();app.prompt_remove_tracker();
    assert!(matches!(&app.prompt.as_ref().unwrap().kind,PromptKind::ConfirmRemoveTracker{url,..} if url==&selected));
}
