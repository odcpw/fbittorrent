use super::*;
use crate::engine::tests::{review_directory, review_record};

#[test]
fn parked_batch_restore_appends_and_preserves_policy() {
    let directory = review_directory("undo-parked");
    std::fs::create_dir_all(&directory).unwrap();
    let mut a = review_record(&format!("{:040x}", 1), &directory);
    a.origin = "future-origin,exact".into();
    a.paused = true;
    a.completion_on = 123;
    let b = review_record(&format!("{:040x}", 2), &directory);
    let c = review_record(&format!("{:040x}", 3), &directory);
    std::fs::write(
        directory.join(format!("{}.torrent", a.hash)),
        b"metainfo fixture",
    )
    .unwrap();
    let mut ledger = Ledger {
        records: vec![a.clone(), b.clone(), c.clone()],
        ..Default::default()
    };
    save_parked_ledger(&directory, &ledger).unwrap();
    let history = Arc::new(History::start(&directory).unwrap());
    let owner =
        ParkedLedger::new(directory.clone(), Lifecycle::default()).with_undo(history.clone());
    let EngineResult::RemovedUndo { undo_until } = owner
        .execute(EngineOp::RemoveKeep {
            hashes: ops::HashSelection::Hashes(vec![a.hash.clone(), b.hash.clone()]),
        })
        .unwrap()
    else {
        panic!("removal")
    };
    assert!(undo_until >= now() as u64 + 9);
    assert!(!directory.join(format!("{}.torrent", a.hash)).exists());
    assert!(directory.join(format!("undo/{}.torrent", a.hash)).exists());
    assert_eq!(
        std::fs::metadata(directory.join("undo"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(directory.join(format!("undo/{}.json", a.hash)))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // Replacing a parked slot retains the daemon's authorization.
    drop(owner);
    let owner =
        ParkedLedger::new(directory.clone(), Lifecycle::default()).with_undo(history.clone());
    owner
        .execute(EngineOp::RestoreUndo {
            hash: a.hash.clone(),
        })
        .unwrap();
    ledger = read_ledger(&directory.join("torrents.json")).unwrap();
    assert_eq!(
        ledger.records.iter().map(|r| &r.hash).collect::<Vec<_>>(),
        [&c.hash, &a.hash]
    );
    assert!(ledger.records[1].paused && ledger.records[1].deferred);
    assert_eq!(
        ledger.records[1].completion_on, 0,
        "undo must actually recheck"
    );
    assert_eq!(ledger.records[1].origin, a.origin);
    let EngineResult::Added(added) = owner
        .execute(EngineOp::RestoreUndo {
            hash: a.hash.clone(),
        })
        .unwrap()
    else {
        panic!("duplicate")
    };
    assert!(added[0].duplicate);
    // Expiration is tested deterministically using a monotonic deadline.
    history
        .entries
        .lock()
        .unwrap()
        .insert(b.hash.clone(), Instant::now() - Duration::from_secs(1));
    let error = owner
        .execute(EngineOp::RestoreUndo {
            hash: b.hash.clone(),
        })
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<InvalidTorrentState>().unwrap().reason,
        "undo_expired"
    );
    history.expire(&directory);
    assert!(!directory.join(format!("undo/{}.json", b.hash)).exists());
    println!(
        "batch has one 10s deadline; saved intent/origin retained; tail append, fresh check, slot survival, duplicate and expiry verified"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn daemon_restart_loses_authorization_and_only_cleans_undo_cache() {
    let directory = review_directory("undo-restart");
    std::fs::create_dir_all(&directory).unwrap();
    let record = review_record(&"a".repeat(40), &directory);
    std::fs::write(
        directory.join(format!("{}.torrent", record.hash)),
        b"metainfo",
    )
    .unwrap();
    prepare(&directory, &[record.clone()]).unwrap();
    let history = History::default();
    history.authorize(&[record.clone()]);
    std::fs::write(directory.join("undo/unrelated.txt"), b"keep").unwrap();
    let next = History::start(&directory).unwrap();
    assert!(
        next.record(&directory, &record.hash)
            .unwrap_err()
            .is::<InvalidTorrentState>()
    );
    assert!(
        directory.join(format!("{}.torrent", record.hash)).exists(),
        "precommit original survives crash cleanup"
    );
    assert!(directory.join("undo/unrelated.txt").exists());
    assert!(
        !directory
            .join(format!("undo/{}.json", record.hash))
            .exists()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_restore_save_retains_authorization_for_retry() {
    let directory = review_directory("undo-failed-save");
    std::fs::create_dir_all(&directory).unwrap();
    let record = review_record(&"b".repeat(40), &directory);
    std::fs::write(
        directory.join(format!("{}.torrent", record.hash)),
        b"metainfo",
    )
    .unwrap();
    save_parked_ledger(
        &directory,
        &Ledger {
            records: vec![record.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    let history = Arc::new(History::default());
    let owner =
        ParkedLedger::new(directory.clone(), Lifecycle::default()).with_undo(history.clone());
    owner
        .execute(EngineOp::RemoveKeep {
            hashes: ops::HashSelection::Hashes(vec![record.hash.clone()]),
        })
        .unwrap();
    std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
    assert!(
        owner
            .execute(EngineOp::RestoreUndo {
                hash: record.hash.clone()
            })
            .is_err()
    );
    assert!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .is_empty()
    );
    assert!(history.record(&directory, &record.hash).is_ok());
    std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
    owner
        .execute(EngineOp::RestoreUndo {
            hash: record.hash.clone(),
        })
        .unwrap();
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .len(),
        1
    );
    println!(
        "restore save failure preserves undo authorization and disk record; same-metainfo retry commits once"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
