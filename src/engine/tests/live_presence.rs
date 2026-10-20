#[test]
fn live_handle_listing_reports_deleted_payload_as_files_missing() {
    let (app, directory, records) = dormant_fixture("live-files-presence", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = &records[0].hash;
    dispatcher
        .execute(EngineOp::SetSeed { hashes: HashSelection(vec![hash.clone()]), on: true })
        .unwrap();
    wait_for_check(&dispatcher, hash);
    let row = |dispatcher: &EngineDispatcher| dormant_rows(dispatcher).into_iter().find(|r| &r.hash == hash).unwrap();
    let sampled = |dispatcher: &EngineDispatcher| dispatcher.runtime.block_on(async {
        let ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), dispatcher.runtime.cx()).await.unwrap();
        dispatcher.info_observed(&ledger, false).into_iter().find(|r| &r.hash == hash).unwrap()
    });
    assert_eq!(row(&dispatcher).state, TorrentState::Seeding);
    assert_eq!(sampled(&dispatcher).state, TorrentState::Seeding);
    let payload = Path::new(&records[0].output_folder).join(&records[0].name);
    std::fs::remove_file(&payload).unwrap();
    // The sampler reuses its recent observation; an explicit read stats now.
    assert_eq!(sampled(&dispatcher).state, TorrentState::Seeding, "sampled presence is bounded, not per tick");
    let missing = row(&dispatcher);
    assert_eq!(missing.state, TorrentState::Error);
    assert_eq!(missing.reason.as_deref(), Some("missing_files"));
    assert!(missing.error.as_deref().unwrap().starts_with("Files missing:"));
    assert!(missing.progress.is_none() && missing.completed_at.is_none() && !missing.checked);
    assert_eq!(sampled(&dispatcher).state, TorrentState::Error, "the explicit read refreshed the sampler");
    assert!(dispatcher.inner.handle(hash).is_ok(), "observing presence leaves the handle alone");
    assert!(!payload.exists(), "a listing never recreates payload");
    std::fs::write(&payload, b"hello").unwrap();
    assert_eq!(row(&dispatcher).state, TorrentState::Seeding, "a restored file clears the report");
    // The dormant rows of the same fixture are untouched by the live check.
    assert!(dormant_rows(&dispatcher).iter().filter(|r| &r.hash != hash).all(|r| r.state == TorrentState::Done));
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
