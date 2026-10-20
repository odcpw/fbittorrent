#[test]
fn handle_lookup_keeps_exact_hash_identity_and_refuses_session_ids() {
    let directory = review_directory("handle-lookup");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("hello.txt"), b"hello").unwrap();
    let mut bytes =
        b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
    let mut record = review_record(&hash, &downloads);
    record.paused = true;
    let app = test_app(
        &directory,
        &downloads,
        Ledger {
            records: vec![record],
            ..Ledger::default()
        },
        None,
    );
    let dispatcher = app.dispatcher();
    let handle = dispatcher.inner.handle(&hash).unwrap();
    let (id, expected) = dispatcher.inner.session.with_torrents(|torrents| {
        torrents
            .next()
            .map(|(id, handle)| (id, handle.clone()))
            .unwrap()
    });
    assert!(Arc::ptr_eq(&handle, &expected));
    for rejected in [
        id.to_string(),
        hash.to_uppercase(),
        "0".repeat(40),
        "g".repeat(40),
    ] {
        let error = dispatcher
            .inner
            .handle(&rejected)
            .err()
            .expect("invalid hash was accepted");
        assert_eq!(error.to_string(), format!("torrent not found: {rejected}"));
    }
    drop(handle);
    drop(expected);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
