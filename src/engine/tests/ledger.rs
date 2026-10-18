use super::*;
use crate::engine::tests::{review_directory, test_app};

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
