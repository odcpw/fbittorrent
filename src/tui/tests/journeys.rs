//! Controls for defects found by the real M3 journeys.
use super::*;
#[path="fixture.rs"] mod fixture;
fn rows()->Vec<crate::api::File> {
    [(0,"Episode.mkv"),(1,"English.srt"),(2,"sample.mkv")].into_iter().map(|(index,path)|crate::api::File{index,path:path.into(),sort_key:path.to_ascii_lowercase(),size:16384,progress:0.25,included:true,ready:None}).collect()
}
#[test]
fn file_push_and_read_use_the_same_order_and_keep_selection_by_identity() {
    let mut app=fixture::app("files");
    let hash=app.files.as_ref().unwrap().hash.clone();
    let input=rows();let ordered=format::files(&input);
    assert_eq!(ordered.iter().map(|f|f.name.as_str()).collect::<Vec<_>>(),["English.srt","Episode.mkv","sample.mkv"]);
    app.files.as_ref().unwrap().list.borrow_mut().select(Some(1)); // existing file index 1
    app.update(Msg::Files(hash.clone(),Ok(ordered)));
    let panel=app.files.as_ref().unwrap();assert_eq!(panel.files[panel.list.borrow().selected().unwrap()].index,1);
    let map=input.into_iter().map(|row|(row.index.to_string(),row)).collect::<std::collections::BTreeMap<_,_>>();
    let pushed=format::files(map.values());
    assert_eq!(panel.files.iter().map(|f|f.name.as_str()).collect::<Vec<_>>(),pushed.iter().map(|f|f.name.as_str()).collect::<Vec<_>>());
    app.update(Msg::Files(hash,Ok(pushed)));
    let panel=app.files.as_ref().unwrap();assert_eq!(panel.files[panel.list.borrow().selected().unwrap()].index,1);
}
#[test]
fn file_save_blocks_stale_toggle_and_clears_errors_after_a_confirmed_read() {
    let mut app=fixture::app("files");let hash=app.files.as_ref().unwrap().hash.clone();
    app.files.as_ref().unwrap().list.borrow_mut().select(Some(2));
    assert!(matches!(app.on_files_action(actions::Intent::Toggle),Cmd::Task(..)));
    assert!(app.files.as_ref().unwrap().saving);
    assert!(matches!(app.on_files_action(actions::Intent::Toggle),Cmd::None));
    app.update(Msg::Files(hash.clone(),Ok(vec![])));assert_eq!(app.files.as_ref().unwrap().files.len(),3,"a pushed/read result cannot reenable an unfinished edit");
    app.update(Msg::FilesSaved{connection:None,hash:hash.clone(),result:Ok(format::files(&rows()))});
    assert!(app.files_editable());assert_eq!(app.files.as_ref().unwrap().list.borrow().selected(),Some(2));
    app.update(Msg::Files(hash.clone(),Err("temporary read failure".into())));
    assert!(!app.files.as_ref().unwrap().error.is_empty());
    app.update(Msg::Files(hash,Ok(format::files(&rows()))));assert!(app.files.as_ref().unwrap().error.is_empty());
}
#[test]
fn rss_enter_opens_a_selected_feed_while_its_first_snapshot_is_pending() {
    let mut app=fixture::app("rss");app.feeds[0].articles.clear();app.rss_focus_items=false;
    let frame=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines().map(|line|serde_json::from_str::<serde_json::Value>(line).unwrap()).find(|v|v["frame"]["result"]["caps"].is_array()).unwrap();
    app.store.hello(serde_json::from_value(frame["frame"]["result"].clone()).unwrap());
    app.feeds[0].uid="pending-feed".into();app.on_key(KeyEvent::new(KeyCode::Enter));
    assert!(app.rss_focus_items);let text=golden::render(&app,80,24);
    assert!(text.contains("Loading…"),"{text}");assert!(!text.contains("No items."));
    app.store.rss_items.insert("pending-feed".into(),Default::default());
    assert!(golden::render(&app,80,24).contains("No items."));
}
#[test]
fn add_prompt_explains_local_torrent_paths() {
    let mut app=fixture::app("torrents");app.on_key(KeyEvent::new(KeyCode::Char('a')));
    let text=golden::render(&app,80,24);assert!(text.contains(".torrent path"),"{text}");assert!(text.contains("seed:off"),"{text}");
}
