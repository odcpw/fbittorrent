use super::*;
use crate::engine::tests::{hashes, review_directory, rows, test_app};

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
            upload_limit: 0,
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
    let listed = rows(&dispatcher);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, TorrentState::Paused);
    assert_eq!(listed[0].intent, Intent::Pause);
    assert_eq!(listed[0].size, Some(5));
    assert_eq!(listed[0].completed_bytes, Some(0));
    assert!(listed[0].dl_bps.is_none() && listed[0].eta_s.is_none() && listed[0].ratio.is_none());
    dispatcher
        .execute(EngineOp::Start {
            hashes: HashSelection(vec![hash.clone()]),
            skip_complete: false,
        })
        .unwrap();
    dispatcher
        .execute(EngineOp::Stop {
            hashes: HashSelection(vec![hash.clone()]),
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
            .execute(EngineOp::SetDownloadLimit { hashes: hashes(&[&hash]), bps: Some(1) })
            .is_err()
    );
    assert!(
        dispatcher
            .execute(EngineOp::SetUploadLimit { hashes: hashes(&[&hash]), limit: -1 })
            .is_err()
    );
    assert_eq!(
        dispatcher.ledger_generation().current(),
        before,
        "invalid typed mutations must not save"
    );
    dispatcher
        .execute(EngineOp::Delete {
            hashes: HashSelection(vec![hash.clone()]),
            files: false,
        })
        .unwrap();
    assert!(rows(&dispatcher).is_empty());
    println!(
        "typed add/list/files/start/stop/delete passed for {hash} in {}",
        directory.display()
    );
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
