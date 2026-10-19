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
        category: "fixture".into(), tags: "fbt-origin:agent".into(),
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
