#[test]
fn completed_listing_detects_deleted_and_truncated_selected_files_without_hashing() {
    let (app, directory, records) = dormant_fixture("done-files-presence", 123, false, false);
    let dispatcher = app.dispatcher();
    assert!(dormant_rows(&dispatcher)
        .iter()
        .all(|r| r.state == TorrentState::Done));
    let first = Path::new(&records[0].output_folder).join(&records[0].name);
    let second = Path::new(&records[1].output_folder).join(&records[1].name);
    let third = Path::new(&records[2].output_folder).join(&records[2].name);
    std::fs::remove_file(&first).unwrap();
    std::fs::write(&second, b"hi").unwrap();
    // Same-size contents are deliberately not hashed by this cheap presence check.
    std::fs::write(&third, b"HELLO").unwrap();
    let rows = dormant_rows(&dispatcher);
    for record in &records[..2] {
        let row = rows.iter().find(|r| r.hash == record.hash).unwrap();
        assert_eq!(row.state, TorrentState::Error);
        assert_eq!(row.reason.as_deref(), Some("missing_files"));
        assert!(row.error.as_deref().unwrap().starts_with("Files missing:"));
        assert!(
            row.progress.is_none() && row.completed_bytes.is_none() && row.completed_at.is_none()
        );
        assert!(!row.checked && dispatcher.inner.handle(&record.hash).is_err());
        let value = wire(row);
        println!("completed selected file unavailable: {value}");
    }
    assert_eq!(
        rows.iter()
            .find(|r| r.hash == records[2].hash)
            .unwrap()
            .state,
        TorrentState::Done
    );
    assert_eq!(dispatcher.inner.session.with_torrents(|t| t.count()), 0);
    assert!(!first.exists(), "listing never creates missing payload");
    std::fs::write(&first, b"hello").unwrap();
    assert_eq!(
        dormant_rows(&dispatcher)
            .iter()
            .find(|r| r.hash == records[0].hash)
            .unwrap()
            .state,
        TorrentState::Done
    );
    drop(dispatcher);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
