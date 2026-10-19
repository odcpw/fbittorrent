use super::*;
use crate::engine::tests::fixture::{Request, Route, LEGACY_ROUTES};
use crate::engine::tests::{request, review_directory, test_app};

#[test]
fn every_route_named_once() {
    let expected: Vec<String> = serde_json::from_str(include_str!("legacy-routes.json")).unwrap();
    let names: HashSet<_> = LEGACY_ROUTES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), LEGACY_ROUTES.len());
    assert_eq!(names.len(), expected.len());
    let body = request(
        "unused",
        &[
            ("hashes", "all"),
            ("hash", "0000000000000000000000000000000000000001"),
            (
                "urls",
                "magnet:?xt=urn:btih:0000000000000000000000000000000000000001",
            ),
            ("max_active_downloads", "3"),
            ("id", "0|1"),
            ("priority", "1"),
            ("limit", "32768"),
            ("ratioLimit", "0"),
            ("shareLimitAction", "Stop"),
            ("tags", "alpha, beta"),
        ],
    );
    for route in expected {
        assert!(
            names.contains(route.as_str()),
            "old socket route {route} missing"
        );
        let kind = Route::from_name(&route).unwrap();
        assert_eq!(kind.name(), route);
        assert!(
            EngineOp::parse(&route, &body).unwrap().is_some(),
            "route {route} does not parse"
        );
        println!("route {route}: one definition; parsed {kind:?}");
    }
    assert!(
        EngineOp::parse("not-an-engine-route", &body)
            .unwrap()
            .is_none()
    );
}

#[test]
fn parser_retains_typed_values_and_rejects_invalid_arguments() {
    let body = request("unused", &[("hashes", "a|b"), ("skip_complete", "true")]);
    let Some(EngineOp::Start {
        hashes,
        skip_complete,
    }) = EngineOp::parse(Route::Start.name(), &body).unwrap()
    else {
        panic!("typed Start")
    };
    assert_eq!(hashes, HashSelection::Hashes(vec!["a".into(), "b".into()]));
    assert!(skip_complete);
    for (route, fields) in [
        (
            Route::FilePriority,
            vec![("hash", "a"), ("id", "0"), ("priority", "7")],
        ),
        (
            Route::SetUploadLimit,
            vec![("hashes", "a"), ("limit", "-1")],
        ),
        (
            Route::SetShareLimits,
            vec![
                ("hashes", "a"),
                ("ratioLimit", "0"),
                ("shareLimitAction", "Remove"),
            ],
        ),
    ] {
        assert!(
            EngineOp::parse(route.name(), &request("unused", &fields)).is_err(),
            "invalid {route:?} must fail"
        );
    }
    let body = request(
        "unused",
        &[
            ("urls", "first\nsecond"),
            ("tags", SEED_TAG),
            ("stopped", "true"),
        ],
    );
    let Some(EngineOp::Add(add)) = EngineOp::parse(Route::Add.name(), &body).unwrap() else {
        panic!("typed Add")
    };
    assert_eq!(add.sources, ["first", "second"]);
    assert_eq!(add.upload_limit, 0);
    assert_eq!(add.ratio_limit, -2.0);
    assert!(add.stopped);
}

#[test]
fn typed_operations_execute_without_legacy_frames() {
    use crate::engine::view::{Intent, TorrentState};
    let directory = review_directory("typed-operations");
    let downloads = directory.join("downloads");
    let app = test_app(&directory, &downloads, Ledger::default(), None);
    let dispatcher = app.dispatcher();
    let mut bytes =
        b"d4:infod6:lengthi5e4:name10:typed.file12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    dispatcher
        .execute(EngineOp::Add(AddRequest {
            sources: Vec::new(),
            torrent: Some(("typed.torrent".into(), bytes)),
            save_path: Some(downloads.to_string_lossy().into()),
            upload_limit: NO_UPLOAD_LIMIT,
            ratio_limit: 0.0,
            stopped: true,
            category: "typed".into(),
            tags: String::new(), origin: None,
        }))
        .unwrap();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        time::timeout(
            cx,
            Duration::from_secs(5),
            dispatcher
                .inner
                .handle(&hash)
                .unwrap()
                .wait_until_initialized(cx),
        )
        .await
        .unwrap()
        .unwrap();
    });
    let EngineResult::Torrents(rows) = dispatcher.execute(EngineOp::Info).unwrap() else {
        panic!("typed torrent result")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, TorrentState::Paused);
    assert_eq!(rows[0].intent, Intent::Pause);
    assert_eq!(rows[0].size, Some(5));
    assert_eq!(rows[0].completed_bytes, Some(0));
    assert!(rows[0].dl_bps.is_none() && rows[0].eta_s.is_none() && rows[0].ratio.is_none());
    dispatcher
        .execute(EngineOp::Start {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
            skip_complete: false,
        })
        .unwrap();
    dispatcher
        .execute(EngineOp::Stop {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
        })
        .unwrap();
    let EngineResult::Files(files) = dispatcher
        .execute(EngineOp::Files { hash: hash.clone() })
        .unwrap()
    else {
        panic!("typed files")
    };
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "typed.file");
    let before = dispatcher.ledger_generation().current();
    assert!(
        dispatcher
            .execute(EngineOp::FilePriority {
                hash: hash.clone(),
                ids: vec![0],
                priority: 9
            })
            .is_err()
    );
    assert!(
        dispatcher
            .execute(EngineOp::SetShareLimits {
                hashes: HashSelection::All,
                ratio: 5.0
            })
            .is_err()
    );
    assert_eq!(
        dispatcher.ledger_generation().current(),
        before,
        "invalid typed mutations must not save"
    );
    dispatcher
        .execute(EngineOp::Delete {
            hashes: HashSelection::Hashes(vec![hash.clone()]),
            files: false,
        })
        .unwrap();
    let EngineResult::Torrents(rows) = dispatcher.execute(EngineOp::Info).unwrap() else {
        panic!("typed torrent result")
    };
    assert!(rows.is_empty());
    println!(
        "typed add/list/files/start/stop/delete passed for {hash} in {}",
        directory.display()
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
