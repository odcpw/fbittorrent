use super::*;

fn fixture_events() -> Vec<api::Event> {
    include_str!("../../../../contracts/ipc/v1/search.ndjson")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|line| {
            (line["dir"] == "s2c" && line["frame"].get("ev").is_some())
                .then(|| serde_json::from_value(line["frame"].clone()).unwrap())
        })
        .collect()
}
#[test]
fn native_search_reduces_pushed_items_without_a_polling_subscription() {
    let mut app = TuiApp::new(Config::default(), Screen::Search, Shows::default());
    app.search_seq = 42;
    app.search_sub = Some((1042, "Sintel".into()));
    app.search_running = true;
    app.native_search = Some(Session {
        id: 1,
        instance: api::Instance::new("0".repeat(32)).unwrap(),
        subscriptions: vec![1, 2],
    });
    assert!(
        !app.subscriptions().iter().any(|s| s.id() == 1042),
        "native search must not spawn the legacy polling worker"
    );
    for event in fixture_events() {
        app.store.event(&event).unwrap();
        if matches!(event, api::Event::SnapshotChunk { .. }) {
            assert!(
                app.search_results.is_empty(),
                "partial snapshot is not visible"
            );
        }
        if matches!(
            event,
            api::Event::SnapshotEnd { .. }
                | api::Event::Delta { .. }
                | api::Event::SearchDone { .. }
        ) {
            app.sync_native_search();
        }
    }
    assert_eq!(app.search_results.len(), 1);
    let result = &app.search_results[0];
    assert_eq!(result.file_name, "辛特尔");
    assert_eq!(result.engine, "alpha, beta");
    assert_eq!(result.leechers, -1);
    assert_eq!(
        app.wire_result(result)
            .unwrap()
            .added
            .as_ref()
            .unwrap()
            .as_str(),
        "a".repeat(40)
    );
    assert!(!app.search_running);
    assert!(app.status.contains("1 result"));
    assert!(
        app.native_search.is_some(),
        "completed results retain their owned session until a deliberate replacement or disconnect"
    );
}
#[test]
fn wire_unknowns_and_provider_names_remain_truthful() {
    let item: api::SearchItem = serde_json::from_value(
        json!({"key":"title:unknown|size:?","name":"Straße","providers":["alpha","beta"]}),
    )
    .unwrap();
    let shown = display(&item);
    assert_eq!(shown.file_size, -1);
    assert_eq!(shown.seeders, -1);
    assert_eq!(shown.pub_date, 0);
    assert_eq!(shown.file_name, "Straße");
    assert_eq!(shown.engine, "alpha, beta");
    assert!(!serde_json::to_value(&item)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("size"));
}
