#[test]
fn dormant_tracker_edits_are_durable_without_admitting_payloads() {
    let (app,directory,records)=dormant_fixture("dormant-tracker-edits",123,false,true);
    let dispatcher=app.dispatcher();let hash=records[0].hash.clone();
    let a="http://127.0.0.1:1/a?passkey=private".to_string();let b="udp://127.0.0.1:2/b".to_string();
    for (url,add) in [(&a,true),(&b,true),(&a,false)] {
        dispatcher.execute(EngineOp::EditTracker{hash:hash.clone(),url:url.clone(),add}).unwrap();
        assert!(dispatcher.inner.handle(&hash).is_err());
        assert_eq!(dispatcher.restore_outcomes()[&hash],RestoreOutcome::Dormant);
    }
    let EngineResult::Trackers(rows)=dispatcher.execute(EngineOp::Trackers{hash:hash.clone()}).unwrap()else{panic!()};
    assert_eq!(rows.len(),1);assert_eq!(rows[0].url.as_str(),b);
    assert_eq!(rows[0].status,librqbit::TrackerStatus::NotContacted);assert!(rows[0].last_announce_at.is_none());
    let saved=read_ledger(&directory.join("torrents.json")).unwrap();
    let record=saved.records.iter().find(|r|r.hash==hash).unwrap();
    assert_eq!(record.trackers,[b]);assert_eq!(record.removed_trackers,[a]);
    let row=&dormant_rows(&dispatcher)[0];
    assert_eq!(row.state,TorrentState::Error);
    assert_eq!(row.reason.as_deref(),Some("missing_files"));
    assert!(row.progress.is_none());
    assert!(std::fs::read_dir(directory.join("downloads")).unwrap().next().is_none());
}

#[test]
fn redacted_alias_removes_only_its_tracker_and_cannot_be_added_or_retargeted() {
    let (app,directory,records)=dormant_fixture("tracker-alias-removal",123,false,true);
    let dispatcher=app.dispatcher();let hash=records[0].hash.clone();
    let a="http://127.0.0.1:1/0123456789abcdef0123456789abcdef/announce?key=short";
    let b="http://127.0.0.1:1/FEDCBA9876543210FEDCBA9876543210/announce?key=other";
    let edit=|url:&str,add|dispatcher.execute(EngineOp::EditTracker{hash:hash.clone(),url:url.into(),add});
    edit(a,true).unwrap();edit(b,true).unwrap();
    let alias=crate::api::trackers::alias(&url::Url::parse(a).unwrap()).unwrap();
    assert!(edit(&alias,true).is_err(),"display aliases must never become announces");
    for fragment in ["FBT-TRACKER=stale","fbt-tracker%3Dstale","%66bt-tracker=stale"] {
        let malformed=format!("http://127.0.0.1:1/REDACTED/announce#{fragment}");
        assert!(edit(&malformed,true).is_err());edit(&malformed,false).unwrap();
    }
    edit(&alias,false).unwrap();edit(&alias,false).unwrap();
    let EngineResult::Trackers(rows)=dispatcher.execute(EngineOp::Trackers{hash:hash.clone()}).unwrap()else{panic!()};
    assert_eq!(rows.len(),1);assert_eq!(rows[0].url.as_str(),b);
    let saved=read_ledger(&directory.join("torrents.json")).unwrap();
    let row=saved.records.iter().find(|r|r.hash==hash).unwrap();
    assert_eq!(row.trackers,[b]);assert_eq!(row.removed_trackers,[a]);
    assert!(dispatcher.inner.handle(&hash).is_err(),"configuration remains dormant");
}
