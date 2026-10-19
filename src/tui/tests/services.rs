//! Confirmed API service projections and intent dispatch, without store-file I/O.
use super::*;
fn show() -> crate::api::Show {
    serde_json::from_value(serde_json::json!({"imdb":"tt1234001","title":"Fixture Show","unread":2,"added_at":1,"counts":{"episodes":3,"seen":1,"grabbed":0},"checking":false,"catalogue_complete":true})).unwrap()
}
fn release(key: &str, read: bool) -> crate::api::Release {
    serde_json::from_value(serde_json::json!({"show_imdb":"tt1234001","key":key,"title":format!("Fixture Show {key}"),"season_pack":false,"read":read,"transfer":"none"})).unwrap()
}
#[test]
fn services_are_confirmed_projections_and_unknown_release_metrics_stay_absent() {
    let mut app = TuiApp::new(Config::default(), Screen::Shows, Shows::default());
    app.store.shows.insert(show().imdb.clone(), show());
    app.store.releases_topic = Some("inbox".into());
    let row = release("S01E01", false);
    app.store
        .show_releases
        .insert(crate::api::release_key(true, &row), row);
    app.sync_native_shows();
    assert_eq!(app.inbox_rows.len(), 1);
    let (show, episode) = app.selected_episode().unwrap();
    assert_eq!(
        app.show_unread(show),
        2,
        "list count comes from metadata, not the loaded detail slice"
    );
    let metadata = app.release_metadata(show, episode, 100);
    assert_eq!(metadata.matches('—').count(), 4, "{metadata}");
    assert!(!metadata.contains("0 B"));
    assert!(matches!(
        app.on_shows_action(actions::Intent::Mark),
        Cmd::Task(..)
    ));
    assert_eq!(
        app.inbox_rows.len(),
        1,
        "an outgoing set_read may not remove its confirmed row"
    );
    assert!(
        app.inbox_last_read.is_none(),
        "Undo is available only after a confirmed success"
    );
    app.service_done(
        None,
        "shows.set_read",
        serde_json::json!({}),
        "ignored".into(),
        Err("fixture error".into()),
    );
    assert_eq!(app.inbox_rows.len(), 1);
    assert!(app.status_is_error);
    let row = release("S01E01", true);
    app.store.releases_topic = Some(show_id());
    app.store.show_releases.clear();
    app.store.show_releases.insert(row.key.clone(), row);
    app.select_show_parent(1);
    app.sync_native_shows();
    assert!(app.shows.shows[0].seen.contains("S01E01"));
    assert!(app.inbox_rows.is_empty());
}
fn show_id() -> String {
    "tt1234001".into()
}
#[test]
fn feed_identity_and_selected_item_survive_pushed_reordering() {
    let mut app = TuiApp::new(Config::default(), Screen::Rss, Shows::default());
    let feed:crate::api::Feed=serde_json::from_value(serde_json::json!({"id":"fixture","name":"Feed","url":"http://127.0.0.1/feed","unread":7,"loading":false})).unwrap();
    app.store.rss_feeds.insert(feed.id.clone(), feed);
    app.sync_native_feeds();
    assert_eq!(app.feed_counts(&app.feeds[0]), (7, None));
    assert!(
        app.feeds[0].articles.is_empty(),
        "feed metadata does not invent item rows"
    );
    let item = |id: &str| {
        serde_json::from_value::<crate::api::FeedItem>(
            serde_json::json!({"id":id,"title":id,"read":false,"grabbed":false}),
        )
        .unwrap()
    };
    app.store.rss_items.insert(
        "fixture".into(),
        [("a".into(), item("a")), ("b".into(), item("b"))].into(),
    );
    app.sync_native_feeds();
    app.item_list.borrow_mut().select(Some(1));
    app.store
        .rss_items
        .get_mut("fixture")
        .unwrap()
        .insert("c".into(), item("c"));
    app.sync_native_feeds();
    assert_eq!(
        app.feeds[0].articles[app.item_list.borrow().selected().unwrap()].id,
        "b"
    );
    assert_eq!(app.feed_id("Feed").as_deref(), Some("fixture"));
    assert!(app.feeds[0]
        .articles
        .iter()
        .all(|a| a.size.is_none() && a.date.is_empty()));
    let ids = Model::subscriptions(&app)
        .iter()
        .map(|s| s.id())
        .collect::<Vec<_>>();
    assert!(!ids.contains(&1), "no periodic data poll: {ids:?}");
}
