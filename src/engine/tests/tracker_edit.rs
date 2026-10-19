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
    assert_eq!(dormant_rows(&dispatcher)[0].state,TorrentState::Done);
    assert!(std::fs::read_dir(directory.join("downloads")).unwrap().next().is_none());
}
