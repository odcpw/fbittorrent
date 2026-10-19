use super::*;
#[test]
fn history_state_waits_for_its_tick() {
    let old=json!({"history":{"download":[1],"upload":[0],"interval_s":1}});
    let new=json!({"history":{"download":[1,2],"upload":[0,0],"interval_s":1}});
    assert_eq!(structural_projection(&Topic::Stats,&old,&new),old);
    let old=json!({"history":[1],"counts":{},"top":[]});
    let new=json!({"history":[1,2],"counts":{},"top":[]});
    assert_eq!(structural_projection(&Topic::Summary,&old,&new),old);
}
#[test]
fn rev_monotonic() {
    let sink = EventSink::testing_with_hello();
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
        history:vec![],stats:json!({"history":{"download":[],"upload":[],"interval_s":1}}),
        epoch: 1,
        state: "starting".into(),
        rows: vec![],
        completions: vec![],
        logs: json!([]),
        searches: BTreeMap::new(),
        files: BTreeMap::new(),
        peers: BTreeMap::new(),
        pieces:BTreeMap::new(),
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
fn merged_search_and_show_inbox_keep_distinct_removal_keys() {
    let cases = [
        (Topic::Search(1), json!([{"key":"first"},{"key":"second"}]),
            json!([{"key":"second"}]), "first"),
        (Topic::ShowReleases("inbox".into()),
            json!([{"show_imdb":"tt1","key":"S01E01"},{"show_imdb":"tt2","key":"S01E01"}]),
            json!([{"show_imdb":"tt2","key":"S01E01"}]), "tt1/S01E01"),
    ];
    for (topic, before, after, removed_key) in cases {
        let (changed, added, removed) = diff(&topic, &before, &after);
        assert!(changed.is_empty() && added.is_empty());
        assert_eq!(removed, vec![json!(removed_key)]);
    }
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

#[test]
fn pieces_are_singleton_state_and_never_structural_updates() {
    let hash=Hash::new("a".repeat(40)).unwrap();let topic=Topic::Pieces(hash);
    let old=json!({"count":8,"piece_size":16384,"have":"AA==","priority":[0,7]});
    let new=json!({"count":8,"piece_size":16384,"have":"gA==","priority":[0,7]});
    assert_eq!(structural_projection(&topic,&old,&new),old);
    let event=state_event(1,&topic,1,new).unwrap();
    use validation::Validate;
    event.validate().unwrap();
    assert!(matches!(event,Event::State {data:StateData::Pieces(_),..}));
}

#[test]
fn pieces_missing_between_samples_does_not_postpone_the_deadline_and_restart_resends() {
    let hash=Hash::new("a".repeat(40)).unwrap();let topic=Topic::Pieces(hash.clone());
    let data=json!({"count":8,"piece_size":16384,"have":"gA==","priority":[0,7]});
    let row=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines().filter_map(|line|serde_json::from_str::<Value>(line).ok())
        .find_map(|line|line.pointer("/frame/result/torrents/0").cloned()).unwrap();
    let mut row:Torrent=serde_json::from_value(row).unwrap();row.hash=hash.clone();
    let mut sample=Sample {history:vec![],stats:Value::Null,epoch:1,state:"running".into(),rows:vec![row],completions:vec![],logs:json!([]),searches:BTreeMap::new(),files:BTreeMap::new(),peers:BTreeMap::new(),pieces:BTreeMap::new(),shows:json!([]),show_releases:BTreeMap::new(),rss_feeds:json!([]),rss_items:BTreeMap::new(),network:json!({}),settings:json!({}),disk:Disk {needed_bytes:0,low:false,free_bytes:None},daemon:json!({})};
    let due=Instant::now()+Duration::from_secs(2);let sink=EventSink::testing();
    let hello:Value=serde_json::from_str(include_str!("../../../contracts/ipc/v1/pieces.ndjson").lines().nth(1).unwrap()).unwrap();
    sink.testing_negotiate(&serde_json::from_value(hello["frame"]["result"].clone()).unwrap());
    let mut sub=Subscription {sub:1,sink:sink.clone(),rev:1,topics:vec![TopicState {topic,interval:Duration::from_secs(2),due,sent:Some(data.clone())}],since:None,epoch:Some((1,"running".into())),active:true};
    for _ in 0..10 {sub.update(&sample,&[],1).unwrap();}
    assert_eq!(sub.topics[0].due,due);assert!(sink.take_frames().is_empty());
    sample.epoch=2;sub.update(&sample,&[],1).unwrap();assert!(sub.topics[0].sent.is_none());
    sample.pieces.insert(hash.to_string(),data);sub.topics[0].due=Instant::now();
    sub.update(&sample,&[],1).unwrap();assert_eq!(sink.take_frames().len(),1,"new engine must republish even an identical bitfield");
}

#[test]
fn stats_subscription_waits_for_a_sample_of_its_new_topic() {
    let sink=EventSink::testing_with_hello();
    let mut subscription=Subscription {sub:1,sink:sink.clone(),rev:0,
        topics:vec![TopicState {topic:Topic::Stats,interval:Duration::from_secs(1),due:Instant::now(),sent:None}],
        since:None,epoch:None,active:true};
    // The worker collected its topic set before this subscription arrived.
    let mut sample=Sample {history:vec![],stats:Value::Null,epoch:1,state:"running".into(),
        rows:vec![],completions:vec![],logs:json!([]),searches:BTreeMap::new(),files:BTreeMap::new(),peers:BTreeMap::new(),pieces:BTreeMap::new(),
        network:json!({"ready":true,"message":"ready"}),settings:json!({}),disk:Disk {needed_bytes:0,low:false,free_bytes:None},
        shows:json!([]),show_releases:BTreeMap::new(),rss_feeds:json!([]),rss_items:BTreeMap::new(),
        daemon:json!({"state":"running","busy_reasons":[],"upgrade_pending":false})};
    subscription.update(&sample,&[],1).unwrap();
    assert!(sink.take_frames().is_empty());assert_eq!(subscription.rev,0);
    assert!(subscription.topics[0].sent.is_none() && subscription.topics[0].due>Instant::now());
    sample.stats=json!({"history":{"download":[0],"upload":[0],"interval_s":1}});
    subscription.update(&sample,&[],1).unwrap();
    let frames=sink.take_frames();assert_eq!(frames.len(),1);
    assert_eq!(frames[0]["ev"],"state");assert_eq!(frames[0]["rev"],1);
    assert_eq!(frames[0]["data"]["history"]["download"],json!([0]));
    assert!(!sink.closed());
}
