use super::*;
#[test]
fn rev_monotonic() {
    let sink = EventSink::testing();
    let mut subscription = Subscription {
        sub: 1,
        sink: sink.clone(),
        rev: 0,
        topics: vec![Topic::Summary, Topic::Torrents, Topic::Events]
            .into_iter()
            .map(|topic| TopicState {
                topic,
                interval: Duration::ZERO,
                due: Instant::now(),
                sent: None,
            })
            .collect(),
        since: None,
        epoch: None,
        active: true,
    };
    let mut sample = Sample {
        epoch: 1,
        state: "starting".into(),
        rows: vec![],
        completions: vec![],
        logs: json!([]),
        files: BTreeMap::new(),
        peers: BTreeMap::new(),
        network: json!({"ready":true,"message":"ready"}),
        settings: json!({}),
        disk: Disk {needed_bytes:0,low:false,free_bytes:None},
        shows:json!([]),show_releases:BTreeMap::new(),
        rss_feeds:json!([]),rss_items:BTreeMap::new(),
        daemon: json!({"state":"running","busy_reasons":[],"upgrade_pending":false}),
    };
    subscription.update(&sample, &[], 1).unwrap();
    assert!(sink.take_frames().is_empty());
    assert!(subscription.topics.iter().all(|topic| topic.sent.is_none()
        && topic.due > Instant::now()));
    assert_eq!(subscription.rev, 0);
    sample.state = "running".into();
    let mut last = 0;
    for i in 0..100 {
        sample.network["message"] = json!(format!("observation {i}"));
        subscription.update(&sample, &[], 1).unwrap();
        for frame in sink.take_frames() {
            let rev = frame["rev"].as_u64().unwrap();
            assert_eq!(rev, last + 1);
            last = rev;
        }
    }
    assert_eq!(last, subscription.rev);
    assert!(last > 100);
    println!("consecutive per-subscription revisions across state and keyed snapshots: {last}");
}
#[test]
fn queue_full_marks_slow() {
    let sink = EventSink::testing();
    for _ in 0..=512 {
        sink.send(Event::Heartbeat { ts: 1 }).unwrap();
    }
    assert!(sink.closed());
    assert_eq!(sink.terminal().unwrap()["reason"], "slow_client");
    assert!(sink.take_frames().is_empty());
}
#[test]
fn diff_changed_added_removed() {
    let old = json!([{"hash":"a","name":"A","eta_s":2},{"hash":"b"}]);
    let new = json!([{"hash":"a","name":"B"},{"hash":"c"}]);
    let (c, a, r) = diff(&Topic::Torrents, &old, &new);
    assert_eq!(c["a"]["eta_s"], Value::Null);
    assert_eq!(c["a"]["name"], "B");
    assert_eq!(a, json!([{"hash":"c"}]).as_array().unwrap().clone());
    assert_eq!(r, vec![json!("b")]);
    println!("changed={c:?} added={a:?} removed={r:?}");
}
#[test]
fn coalesce_rates_not_states() {
    let old = json!([{"hash":"a","state":"downloading","progress":0.2,"playable":{"ready":false,"buffered_s":1}}]);
    let new = json!([{"hash":"a","state":"paused","progress":0.3,"playable":{"ready":true,"buffered_s":2}}]);
    let actual = structural_projection(&Topic::Torrents, &old, &new);
    assert_eq!(actual[0]["state"], "paused");
    assert_eq!(actual[0]["progress"], 0.2);
    assert_eq!(actual[0]["playable"]["ready"], true);
    assert_eq!(actual[0]["playable"]["buffered_s"], 1);
}
#[test]
fn events_ring_resync() {
    let mut r = EventRing::default();
    for _ in 0..1002 {
        r.append(json!({"ev":"engine.changed"}));
    }
    let (resync, oldest, items) = r.replay(Some(1));
    assert!(resync);
    assert_eq!(oldest, 3);
    assert_eq!(items.len(), 1000);
    assert!(!r.replay(Some(2)).0);
    r.entries.front_mut().unwrap().0 = Instant::now() - Duration::from_secs(601);
    assert_eq!(r.oldest(), 4);
    assert_eq!(EventRing::default().append(json!({}))["seq"], 1);
}

#[test]
fn peer_counters_coalesce_but_seed_and_direction_are_structural(){
    let topic=Topic::Peers(Hash::new("12".repeat(20)).unwrap());
    let old=json!([{"addr":"127.0.0.1:1","seed":false,"downloaded":1,"uploaded":2,"dl_bps":3,"ul_bps":4}]);
    let new=json!([{"addr":"127.0.0.1:1","seed":true,"downloaded":9,"uploaded":10,"dl_bps":11,"ul_bps":12}]);
    let projected=diff::structural_projection(&topic,&old,&new);
    assert_eq!(projected[0]["downloaded"],1);assert_eq!(projected[0]["uploaded"],2);
    assert_eq!(projected[0]["dl_bps"],3);assert_eq!(projected[0]["ul_bps"],4);assert_eq!(projected[0]["seed"],true);
    let (changed,_,_)=diff::diff(&topic,&old,&new);assert!(changed.contains_key("127.0.0.1:1"));
}
