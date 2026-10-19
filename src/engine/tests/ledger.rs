use super::*;
use crate::engine::tests::{review_directory, test_app};

#[test]
fn deferred_record_from_magnet() {
    use crate::engine::ops::{AddRequest, EngineOp, EngineResult};
    let directory = review_directory("deferred-magnet");
    let source = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Deferred%20fixture&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce";
    let options = AddRequest {
        sources: vec![source.into()], torrent: None, save_path: Some(directory.join("downloads").to_string_lossy().into()),
        upload_limit: 32768, ratio_limit: 0.0, stopped: false,
        category: "fixture".into(), tags: "fbt-origin:agent".into(), origin: None,
    };
    let record = Record::deferred_magnet(&options, &directory).unwrap();
    assert_eq!(record.hash, "0123456789abcdef0123456789abcdef01234567");
    assert_eq!(record.name, "Deferred fixture");
    assert_eq!(record.source, source);
    assert!(record.deferred);
    assert!(!record.paused, "the absent session is paused; durable intent remains run");
    let prior = crate::engine::tests::review_record("ab00000000000000000000000000000000000000", &directory);
    let mut ledger = Ledger::default();ledger.records.push(prior.clone());
    save_parked_ledger(&directory, &ledger).unwrap();
    let parked = crate::engine::ParkedLedger::new(directory.clone(), Lifecycle::default());
    let generation = parked.ledger_generation().current();
    let EngineResult::Added(added) = parked.execute(EngineOp::AddMagnet(options.clone())).unwrap() else { panic!("add result") };
    assert!(added[0].deferred && !added[0].duplicate);
    let saved = read_ledger(&directory.join("torrents.json")).unwrap();
    assert_eq!(saved.records.len(), 2);
    assert_eq!(saved.records[0].hash, prior.hash);
    assert!(saved.records[1].deferred && !saved.records[1].paused);
    let before = std::fs::read(directory.join("torrents.json")).unwrap();
    let EngineResult::Added(duplicate) = parked.execute(EngineOp::AddMagnet(AddRequest { stopped: true, ..options })).unwrap() else { panic!("duplicate") };
    assert!(duplicate[0].duplicate && duplicate[0].deferred);
    assert_eq!(before, std::fs::read(directory.join("torrents.json")).unwrap());
    assert!(parked.ledger_generation().current() > generation);
    let EngineResult::Torrents(rows) = parked.execute(EngineOp::Info).unwrap() else { panic!("rows") };
    let row = rows.iter().find(|row| row.hash == record.hash).unwrap();
    assert_eq!(row.state, crate::engine::view::TorrentState::WaitingVpn);
    assert_eq!(row.intent, crate::engine::view::Intent::Run);
    assert!(!row.checked);
    assert!(row.progress.is_none());
    println!("deferred_record_from_magnet: saved btih/dn/tracker, private durable marker, run intent, end-of-ledger order, duplicate preserves bytes, no live session; row=waiting_vpn");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn generation_bumps_on_save() {
    let directory = review_directory("ledger-generation");
    let app = test_app(
        &directory,
        &directory.join("downloads"),
        Ledger::default(),
        None,
    );
    let dispatcher = app.dispatcher();
    let generation = dispatcher.ledger_generation();
    assert_eq!(generation.current(), 0);
    let (ready, waiting) = std::sync::mpsc::channel();
    let observer = generation.clone();
    let thread = std::thread::spawn(move || {
        ready.send(()).unwrap();
        observer.wait_after(0, Duration::from_secs(3))
    });
    waiting.recv().unwrap();
    let began = std::time::Instant::now();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx)
            .await
            .unwrap();
        dispatcher.inner.save(cx, &ledger).await.unwrap();
        assert_eq!(ledger.generation.current(), 1);
        assert!(
            serde_json::to_value(&*ledger)
                .unwrap()
                .get("generation")
                .is_none()
        );
        // The async records mutex remains locked throughout this standard-thread
        // wait; the waiter only touches the shared atomic/Condvar handle.
        assert_eq!(generation.wait_after(1, Duration::from_millis(10)), None);
    });
    let observed = thread.join().unwrap();
    println!(
        "generation_bumps_on_save: before=0 after={} waiter={observed:?} elapsed={:?}",
        generation.current(),
        began.elapsed()
    );
    assert_eq!(observed, Some(1));
    assert!(began.elapsed() < Duration::from_secs(2));
    std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx)
            .await
            .unwrap();
        assert!(dispatcher.inner.save(cx, &ledger).await.is_err());
    });
    assert_eq!(
        generation.current(),
        1,
        "failed save must not announce a successful commit"
    );
    std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parked_ledger_generations_survive_reads_and_each_durable_save() {
    use crate::engine::ops::{EngineOp, HashSelection};
    let directory = review_directory("parked-generations");
    save_parked_ledger(&directory, &Ledger::default()).unwrap();
    let parked = crate::engine::ParkedLedger::new(directory.clone(), Lifecycle::default());
    let generation = parked.ledger_generation();
    for expected in 1..=2 {
        parked
            .execute(EngineOp::Stop {
                hashes: HashSelection::All,
            })
            .unwrap();
        assert_eq!(generation.current(), expected);
        parked.execute(EngineOp::Info).unwrap();
        assert_eq!(generation.current(), expected);
    }
    println!(
        "parked generation after two saves and reads: {}",
        generation.current()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn save_preserves_order() {
    let directory=review_directory("queue-order");
    std::fs::create_dir_all(&directory).unwrap();
    let records:Vec<_>=[3,1,2].into_iter().map(|n| {
        let mut r=crate::engine::tests::review_record(&format!("{n:040x}"),&directory);
        r.added_on=n;r
    }).collect();
    let expected:Vec<_>=records.iter().map(|r|r.hash.clone()).collect();
    let ledger=Ledger{records,..Default::default()};
    save_parked_ledger(&directory,&ledger).unwrap();
    let saved=read_ledger(&directory.join("torrents.json")).unwrap();
    assert_eq!(saved.records.iter().map(|r|r.hash.clone()).collect::<Vec<_>>(),expected);
    let parked=crate::engine::ParkedLedger::new(directory.clone(),Lifecycle::default());
    let crate::engine::ops::EngineResult::Torrents(rows)=parked.execute(crate::engine::ops::EngineOp::Info).unwrap() else {panic!("rows");};
    assert_eq!(rows.iter().map(|r|r.queue_pos.unwrap()).collect::<Vec<_>>(),[1,3,2],"display sort cannot change ledger positions");
    assert_eq!(read_ledger(&directory.join("torrents.json")).unwrap().records.iter().map(|r|r.hash.clone()).collect::<Vec<_>>(),expected);
    println!("saved ledger order stays 3,1,2 independently of added_on and display sort");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn origin_default_manual() {
    let directory=review_directory("origin-migration");
    let mut record=crate::engine::tests::review_record(&"a".repeat(40),&directory);
    let mut value=serde_json::to_value(&record).unwrap();
    value.as_object_mut().unwrap().remove("origin");
    assert_eq!(serde_json::from_value::<Record>(value.clone()).unwrap().origin,"manual");
    value["tags"]=serde_json::json!("watching,fbt-origin:rss");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("torrents.json"),serde_json::to_vec(&serde_json::json!({"migrated_legacy":false,"records":[value]})).unwrap()).unwrap();
    assert_eq!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].origin,"rss");
    record.origin="future-origin,exact".into();record.tags="fbt-origin:handler".into();
    save_parked_ledger(&directory,&Ledger {records:vec![record.clone()],..Default::default()}).unwrap();
    assert_eq!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].origin,record.origin);
    record.origin="manual".into();
    save_parked_ledger(&directory,&Ledger {records:vec![record],..Default::default()}).unwrap();
    assert_eq!(read_ledger(&directory.join("torrents.json")).unwrap().records[0].origin,"manual");
    println!("old field defaults manual; legacy rss tag migrates; explicit manual and unknown comma-bearing open strings remain authoritative");
    std::fs::remove_dir_all(directory).unwrap();
}


#[test]
fn staged_excluded_from_counts_and_busy() {
    use crate::api::validation::Validate;
    let directory=review_directory("staged-counts-busy");
    let mut record=crate::engine::tests::review_record(&"a".repeat(40),&directory);
    record.staged=true;
    // Even inconsistent old run intent cannot admit a staged record to counts.
    record.paused=false;
    let app=test_app(&directory,&directory.join("downloads"),Ledger::default(),None);
    let dispatcher=app.dispatcher();
    dispatcher.inner.set_restore_outcome(&record.hash,RestoreOutcome::Metadata);
    let row=crate::api::ops::torrents::torrent(view::TorrentView::unrestored(&record,None)).unwrap();
    row.validate().unwrap();
    let counts=serde_json::to_value(crate::api::bus::summary_counts(&[row.clone()])).unwrap();
    assert!(counts.as_object().unwrap().iter().all(|(key,value)|value.as_u64()==Some(u64::from(key=="staged"))));
    let mut ledger=Ledger {records:vec![record],..Default::default()};
    assert!(!dispatcher.has_pending_restore(&ledger),"staged metadata resolver must not count as restore busy work");
    let activity=crate::daemon::policy::Transfers::from_torrents(&[row],"running",dispatcher.has_pending_restore(&ledger));
    let at=std::time::Instant::now();
    let mut policy=crate::daemon::policy::Policy::new(3,None,at);
    policy.transfers(activity,at);
    let snapshot=policy.snapshot(at);
    assert!(snapshot.busy_reasons.is_empty());
    assert_eq!(snapshot.idle_exit_in_s,Some(3));
    ledger.records[0].staged=false;
    assert!(dispatcher.has_pending_restore(&ledger),"ordinary pending metadata must still keep the daemon alive");
    println!("staged counts={counts}; policy={snapshot:?}; ordinary pending restore stays busy");
    drop(dispatcher);drop(app);std::fs::remove_dir_all(directory).unwrap();
}
