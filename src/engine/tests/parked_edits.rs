#[test]
fn parked_record_edits_are_saved_for_the_next_engine_and_refusals_name_the_network() {
    let directory = review_directory("parked-edits");
    std::fs::create_dir_all(&directory).unwrap();
    let bytes = b"d4:infod6:lengthi1e4:name11:offline.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
    let hash = librqbit::torrent_from_bytes(bytes).unwrap().info_hash.as_string();
    std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
    let mut record = review_record(&hash, &directory.join("downloads"));
    record.completion_on = 10;
    record.paused = true;
    record.upload_limit = 0;
    save_parked_ledger(&directory, &Ledger { generation: Default::default(), migrated_legacy: false, records: vec![record] }).unwrap();
    let network = crate::network_app::NetworkState {
        selection: Some("wg0".into()), interface: Some("wg0".into()), index: None, ready: false, message: "Waiting for wg0".into(),
    };
    let parked = ParkedLedger::new(directory.clone()).with_network(network);
    let one = || HashSelection(vec![hash.clone()]);
    parked.execute(EngineOp::SetUploadLimit { hashes: one(), limit: 49152 }).unwrap();
    parked.execute(EngineOp::SetDownloadLimit { hashes: one(), bps: Some(65536) }).unwrap();
    parked.execute(EngineOp::SetSeed { hashes: one(), on: true }).unwrap();
    parked.execute(EngineOp::SetFiles { hash: hash.clone(), include: vec![0] }).unwrap();
    let saved = || read_ledger(&directory.join("torrents.json")).unwrap().records.remove(0);
    let record = saved();
    assert_eq!((record.upload_limit, record.download_limit, record.upload, record.only_files.clone()),
        (49152, Some(65536), Some(true), Some(vec![0])));
    assert!(!record.stop_on_complete(), "typed upload intent permits seeding");
    assert!(!record.paused, "seeding a completed record is run intent, as live SetSeed");
    assert_eq!(record.completion_on, 0, "a changed selection is checked by the next engine");
    let EngineResult::Torrents(rows) = parked.execute(EngineOp::Info).unwrap() else { panic!("rows") };
    assert_eq!((rows[0].upload_limit_bps, rows[0].download_limit_bps, rows[0].seed), (Some(49152), Some(65536), true));
    assert_eq!(rows[0].state, TorrentState::WaitingVpn);
    // Invalid edits are refused before anything is saved.
    assert!(parked.execute(EngineOp::SetFiles { hash: hash.clone(), include: vec![3] }).is_err());
    assert!(parked.execute(EngineOp::SetUploadLimit { hashes: one(), limit: 100 }).is_err());
    assert_eq!((saved().upload_limit, saved().only_files), (49152, Some(vec![0])));
    // Transfer actions still need the engine; the refusal names the configured network.
    let error = parked.execute(EngineOp::Move { hashes: vec![hash.clone()], save_path: directory.join("elsewhere") }).unwrap_err();
    let refusal = error.downcast_ref::<ParkedRefusal>().expect("typed parked refusal");
    assert!(refusal.0.contains("connect wg0") && !refusal.0.contains("ProtonVPN"), "{refusal}");
    let failure = crate::api::ops::engine_error(error);
    assert_eq!(failure.code, crate::api::ErrorCode::EngineUnavailable);
    assert_eq!(failure.details.unwrap()["state"], "parked");
    std::fs::remove_dir_all(directory).unwrap();
}
