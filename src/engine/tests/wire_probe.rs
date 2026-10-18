// Shared verbatim by the old-source capture and the current characterization.
// The caller supplies a private XDG root and runs only this ignored test.
#[test]
#[ignore = "subprocess probe; requires isolated XDG dirs and FBT_P119_WIRE_ROOT"]
fn legacy_wire_fixture_probe() {
    let directory = PathBuf::from(std::env::var_os("FBT_P119_WIRE_ROOT").unwrap());
    for (key, suffix) in [
        ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
        ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CACHE_HOME", "cache"),
        ("XDG_STATE_HOME", "state"),
    ] {
        assert_eq!(PathBuf::from(std::env::var_os(key).unwrap()), directory.join(suffix));
    }
    let output = directory.join("actual");
    std::fs::create_dir_all(&output).unwrap();
    let capture = |name: &str, raw: String| {
        let normalized = raw.replace(directory.to_str().unwrap(), "<FIXTURE>");
        std::fs::write(output.join(format!("{name}.json")), normalized).unwrap();
    };
    let mut cfg = Config::default();
    cfg.save().unwrap();
    let parked = directory.join("parked");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&parked).unwrap();
    std::fs::create_dir_all(&downloads).unwrap();
    let mut records = Vec::new();
    for (index, info) in [
        b"d5:filesld6:lengthi2e4:pathl5:a.txteed6:lengthi3e4:pathl5:b.txteee4:name6:bundle12:piece lengthi16384e6:pieces20:".as_slice(),
        b"d6:lengthi5e4:name10:single.txt12:piece lengthi16384e6:pieces20:".as_slice(),
    ].into_iter().enumerate() {
        let mut bytes = b"d4:info".to_vec();
        bytes.extend_from_slice(info);
        bytes.extend_from_slice(&[0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
        std::fs::write(parked.join(format!("{hash}.torrent")), bytes).unwrap();
        let mut record = review_record(&hash, &downloads);
        record.name = format!("fixture \"{index}\" \\ café");
        record.category = "wire\nfixture".into();
        record.tags = "alpha,beta".into();
        record.added_on = 10 + index as i64;
        record.only_files = (index == 0).then_some(vec![0]);
        record.paused = index == 1;
        record.completion_on = if index == 1 { 7 } else { 0 };
        records.push(record);
    }
    for index in 2..4 {
        let mut record = review_record(&format!("{index:040x}"), &downloads);
        record.name = format!("metadata unavailable {index}");
        record.added_on = 10 + index as i64;
        record.paused = index == 2;
        record.completion_on = if index == 2 { -1 } else { 0 };
        records.push(record);
    }
    let hashes: Vec<_> = records.iter().map(|record| record.hash.clone()).collect();
    save_parked_ledger(&parked, &Ledger { migrated_legacy: true, records, ..Default::default() }).unwrap();
    for (name, route, args) in [
        ("parked-info", "torrents/info", vec![]),
        ("parked-files-multi", "torrents/files", vec![("hash", hashes[0].as_str())]),
        ("parked-files-single", "torrents/files", vec![("hash", hashes[1].as_str())]),
        ("parked-queue", "app/queue", vec![]),
    ] {
        capture(name, dispatch_parked_in(&request(route, &args), &parked).unwrap().unwrap());
    }
    let empty = directory.join("empty");
    save_parked_ledger(&empty, &Ledger::default()).unwrap();
    for (name, route) in [("parked-empty-info", "torrents/info"), ("parked-empty-queue", "app/queue")] {
        capture(name, dispatch_parked_in(&request(route, &[]), &empty).unwrap().unwrap());
    }
    let app = test_app(&directory.join("live"), &downloads, Ledger::default(), None);
    let dispatcher = app.dispatcher();
    let hashes: Vec<_> = (80..84).map(|index| add_queue_fixture(&dispatcher, index, false)).collect();
    capture("live-queue", dispatcher.dispatch(&request("app/queue", &[])).unwrap().unwrap());
    capture("live-files", dispatcher.dispatch(&request("torrents/files", &[("hash", &hashes[0])])).unwrap().unwrap());
    let lifecycle = Lifecycle::default();
    capture("settings-unbound", crate::network_app::controls(&request("app/settings", &[]), &crate::network_app::state(&cfg), &lifecycle).unwrap().unwrap());
    cfg.max_active_downloads = 7;
    cfg.peer_encryption = "require".into();
    cfg.network_interface = Some("fbmissing0".into());
    cfg.save().unwrap();
    capture("settings-bound", crate::network_app::controls(&request("app/settings", &[]), &crate::network_app::state(&cfg), &lifecycle).unwrap().unwrap());
    drop(dispatcher);
    drop(app);
}
