//! Clipboard offers are live, bounded and isolated from text editing/history.
use super::*;
#[path="fixture.rs"] mod fixture;
use crate::api::{self, Event as ApiEvent, Hash};
fn offered(seq:u64, hash:&str, name:&str)->ApiEvent {
    ApiEvent::ClipboardMagnet {seq,ts:1,hash:Hash::new(hash.to_owned()).unwrap(),name:Some(name.into()),already_added:false}
}
#[test]
fn offer_keys_are_contextual_and_dismissal_deduplicates() {
    let mut app=fixture::app("torrents");
    let hash="c7".repeat(20);
    app.clipboard_event(&offered(1,&hash,"private name"));
    assert!(app.clipboard_visible());
    assert_eq!(app.footer_context(),"Clipboard");
    assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Escape)),Cmd::Task(..)));
    assert!(app.clipboard.offer.is_none());
    app.clipboard_event(&offered(2,&hash,"same selection"));
    assert!(app.clipboard.offer.is_none());
    app.service_busy=false;
    app.clipboard_event(&offered(3,&"d8".repeat(20),"another selection"));
    assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('y'))),Cmd::Task(..)));
    assert!(app.clipboard.offer.is_none());
    assert!(!app.status.contains("selection"));
    assert!(app.feedback.history.is_empty(),"clipboard names never enter UI message history");
}
#[test]
fn text_and_confirmation_dialogs_retain_their_keys() {
    let mut app=fixture::app("search");
    app.search_focus_input=true;app.search_input.set_value("");
    app.clipboard_event(&offered(1,&"c7".repeat(20),"private name"));
    app.on_key(KeyEvent::new(KeyCode::Char('y')));
    assert_eq!(app.search_input.value(),"y");
    assert!(app.clipboard.offer.is_some());
    app.open_prompt(PromptKind::AddMagnet,"Add",false);
    app.on_key(KeyEvent::new(KeyCode::Char('y')));
    assert_eq!(app.prompt.as_ref().unwrap().input.value(),"y");
    assert!(app.clipboard.offer.is_some());
}
#[test]
fn offer_is_bounded_static_and_preserves_keys_at_every_width() {
    let mut app=fixture::app("torrents");
    let title="中\n\x1b".repeat(1000);
    app.clipboard_event(&offered(1,&"c7".repeat(20),&title));
    for (width,height) in [(80,24),(120,30),(160,45)] {
        let label=app.clipboard_label(width).unwrap();
        assert!(dw(&label)<=width);assert!(label.ends_with("(y/Esc)"));
        assert!(!label.chars().any(char::is_control));
        let before=app.footer_signature(width as u16);
        let rendered=golden::render(&app,width as u16,height);
        assert!(rendered.contains("[Clipboard]"));assert!(rendered.contains("(y/Esc)"));
        for _ in 0..8 {assert_eq!(golden::render(&app,width as u16,height),rendered);}
        assert_eq!(app.footer_signature(width as u16),before);
    }
    assert!(app.feedback.history.is_empty());
    app.connection_lost(false);assert!(app.clipboard.offer.is_none());
}
#[test]
fn already_added_and_out_of_order_offers_are_ignored() {
    let mut app=fixture::app("torrents");
    let hash="c7".repeat(20);
    let mut event=offered(2,&hash,"already added");
    if let ApiEvent::ClipboardMagnet{already_added,..}=&mut event {*already_added=true;}
    app.clipboard_event(&event);assert!(app.clipboard.offer.is_none());
    app.clipboard_event(&offered(1,&"d8".repeat(20),"older"));assert!(app.clipboard.offer.is_none());
    app.clipboard_event(&offered(3,&"e9".repeat(20),"current"));assert!(app.clipboard_visible());
}
#[test]
fn live_event_delta_offers_but_snapshot_history_does_not() {
    let mut app=fixture::app("torrents");
    let event=offered(1,&"c7".repeat(20),"history");
    let historical=ApiEvent::SnapshotChunk {sub:1,topic:api::Topic::Events,rev:1,
        items:vec![api::Item::Event(Box::new(event.clone()))]};
    app.clipboard_event(&historical);assert!(app.clipboard.offer.is_none());
    let live:ApiEvent=serde_json::from_value(serde_json::json!({"ev":"delta","sub":1,"topic":"events","rev":2,"added":[event]})).unwrap();
    app.clipboard_event(&live);assert!(app.clipboard_visible());
}

#[test]
fn clipboard_frames_never_enter_retained_event_or_message_history() {
    let mut app=fixture::app("torrents");
    let event=offered(1,&"c7".repeat(20),"never retain this clipboard name");
    app.store.event(&event).unwrap();
    assert!(app.store.events.is_empty());assert!(app.feedback.history.is_empty());
    let begin:ApiEvent=serde_json::from_value(serde_json::json!({"ev":"snapshot.begin","sub":1,"topic":"events","rev":1,"count":1,"reason":"subscribe"})).unwrap();
    let chunk=ApiEvent::SnapshotChunk{sub:1,topic:api::Topic::Events,rev:1,items:vec![api::Item::Event(Box::new(event))]};
    let end=ApiEvent::SnapshotEnd{sub:1,topic:api::Topic::Events,rev:1};
    for event in [begin,chunk,end] {app.store.event(&event).unwrap();}
    assert!(app.store.events.is_empty());
}
