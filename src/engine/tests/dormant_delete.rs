#[test]
fn dormant_delete_files_needs_no_session_handle() {
    let (app, directory, records) = dormant_fixture("dormant-delete", 123, false, false);
    let dispatcher = app.dispatcher();
    let hash = records[0].hash.clone();
    dispatcher
        .execute(EngineOp::Delete {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
            files: true,
        })
        .unwrap();
    assert!(!Path::new(&records[0].output_folder)
        .join(&records[0].name)
        .exists());
    assert!(!directory.join(format!("{hash}.torrent")).exists());
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 0);
    let rows = dormant_rows(&dispatcher);
    assert_eq!(rows.len(), 4);
    assert!(rows
        .iter()
        .all(|row| row.state == TorrentState::Done && !row.checked));
    assert!(records[1..]
        .iter()
        .all(|r| Path::new(&r.output_folder).join(&r.name).exists()));
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn dormant_delete_failed_save_keeps_payloads() {
    let (app, directory, records) = dormant_fixture("dormant-delete-save", 123, false, false);
    let dispatcher = app.dispatcher();
    std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
    assert!(dispatcher
        .execute(EngineOp::Delete {
            hashes: HashSelection::Hashes(vec![records[0].hash.clone()]),
            files: true
        })
        .is_err());
    assert!(records
        .iter()
        .all(|r| Path::new(&r.output_folder).join(&r.name).exists()));
    assert_eq!(dormant_rows(&dispatcher).len(), 5);
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records
            .len(),
        5
    );
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 0);
    std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
