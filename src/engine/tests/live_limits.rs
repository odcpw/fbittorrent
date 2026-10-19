#[test]
fn upload_cap_edit_retains_handle_and_checked_data() {
    use ops::{AddRequest, EngineOp, EngineResult, HashSelection};
    let directory = review_directory("live-upload-cap");
    let downloads = directory.join("downloads");
    let app = test_app(&directory, &downloads, Ledger::default(), None);
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
    let dispatcher = app.dispatcher();
    dispatcher
        .execute(EngineOp::Add(AddRequest {
            sources: vec![],
            torrent: Some(("hello.torrent".into(), bytes)),
            save_path: Some(downloads.to_string_lossy().into()),
            upload_limit: 0,
            ratio_limit: 0.0,
            stopped: true,
            category: String::new(),
            tags: String::new(), origin: None,
        }))
        .unwrap();
    let original = dispatcher.inner.handle(&hash).unwrap();
    dispatcher
        .runtime
        .block_on(original.wait_until_initialized(dispatcher.runtime.cx()))
        .unwrap();
    let cap = |limit| EngineOp::SetUploadLimit {
        hashes: HashSelection(vec![hash.clone()]),
        limit,
    };
    for limit in [131072, 0, 0, 0] {
        assert!(matches!(
            dispatcher.execute(cap(limit)).unwrap(),
            EngineResult::Acknowledged
        ));
        assert!(Arc::ptr_eq(
            &original,
            &dispatcher.inner.handle(&hash).unwrap()
        ));
        assert!(original.stats().finished);
        let EngineResult::Torrents(rows) = dispatcher.execute(EngineOp::Info).unwrap() else {
            panic!()
        };
        assert!(rows[0].checked);
        assert_ne!(rows[0].state, view::TorrentState::Checking);
        assert_eq!(rows[0].upload_limit_bps, Some(limit as u64));
        assert_eq!(
            read_ledger(&directory.join("torrents.json"))
                .unwrap()
                .records[0]
                .upload_limit,
            limit
        );
        println!(
            "live cap {limit}: id={}, checked={}, state={:?}",
            original.id(),
            rows[0].checked,
            rows[0].state
        );
    }
    for limit in [-1, 1024, i64::MAX] {
        assert!(dispatcher.execute(cap(limit)).is_err());
    }
    std::fs::create_dir(directory.join("torrents.json.tmp")).unwrap();
    assert!(dispatcher.execute(cap(131072)).is_err());
    assert!(Arc::ptr_eq(
        &original,
        &dispatcher.inner.handle(&hash).unwrap()
    ));
    assert_eq!(
        read_ledger(&directory.join("torrents.json"))
            .unwrap()
            .records[0]
            .upload_limit,
        0
    );
    std::fs::remove_dir(directory.join("torrents.json.tmp")).unwrap();
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
