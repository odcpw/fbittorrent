use librqbit::{create_torrent, spawn_utils::BlockingSpawner, CreateTorrentOptions};
use std::time::Instant;
// Real wire requests deliberately ignore the choke: permission must hold even
// against a peer that requests pieces without waiting for our unchoke.
fn requested_payload(addr: std::net::SocketAddr, hash: &str, expected: &[u8]) -> usize {
    use std::io::{Read, Write};
    let mut peer = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    peer.set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut handshake = vec![19];
    handshake.extend_from_slice(b"BitTorrent protocol");
    handshake.extend_from_slice(&[0; 8]);
    handshake.extend(
        (0..40)
            .step_by(2)
            .map(|i| u8::from_str_radix(&hash[i..i + 2], 16).unwrap()),
    );
    handshake.extend_from_slice(b"-FB0001-wire-probe01");
    assert_eq!(handshake.len(), 68);
    peer.write_all(&handshake).unwrap();
    let mut response = [0; 68];
    peer.read_exact(&mut response).expect("a failed/slow handshake cannot prove zero upload");
    assert_eq!(&response[..20], &handshake[..20]);
    assert_eq!(&response[28..48], &handshake[28..48]);
    peer.write_all(&[0, 0, 0, 1, 2]).unwrap(); // interested
    let mut request = vec![0, 0, 0, 13, 6];
    request.extend_from_slice(&0u32.to_be_bytes());
    request.extend_from_slice(&0u32.to_be_bytes());
    request.extend_from_slice(&16384u32.to_be_bytes());
    peer.write_all(&request).expect("the verified-piece request must reach the socket");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut payload = 0;
    let mut closed = false;
    while Instant::now() < deadline {
        let mut prefix = [0; 4];
        match peer.read(&mut prefix[..1]) {
            Ok(0) => { closed = true; break; } // Explicit closure after our complete request.
            Ok(_) => peer.read_exact(&mut prefix[1..]).unwrap(),
            Err(error) => panic!("slow/failed request response cannot prove zero upload: {error}"),
        }
        let len = u32::from_be_bytes(prefix) as usize;
        assert!(len <= 65536, "bounded peer frame");
        if len == 0 {
            continue;
        }
        let mut frame = vec![0; len];
        peer.read_exact(&mut frame).unwrap();
        if frame[0] == 7 {
            assert!(len >= 9);
            assert_eq!(&frame[1..9], &[0; 8]);
            assert_eq!(&frame[9..], expected, "positive control serves the actual verified block");
            payload += len - 9;
            break;
        }
    }
    assert!(closed || payload > 0, "a response deadline cannot prove zero upload");
    payload
}

#[test]
fn upload_permission_blocks_payload_in_both_phases_and_allows_explicit_opt_in() {
    for complete in [false, true] {
        let directory = review_directory(if complete {
            "upload-complete"
        } else {
            "upload-downloading"
        });
        let source = directory.join("source");
        let downloads = directory.join("downloads");
        std::fs::create_dir_all(&source).unwrap();
        let payload: Vec<u8> = (0..32768).map(|i| (i % 251) as u8).collect();
        std::fs::write(source.join("payload.bin"), &payload).unwrap();
        let app = test_app(&directory, &downloads, Ledger::default(), Some(0));
        let dispatcher = app.dispatcher();
        let bytes = dispatcher
            .runtime
            .block_on(create_torrent(
                dispatcher.runtime.cx(),
                &source.join("payload.bin"),
                CreateTorrentOptions {
                    piece_length: Some(16384),
                    ..Default::default()
                },
                &BlockingSpawner::new(1),
            ))
            .unwrap()
            .as_bytes()
            .unwrap();
        std::fs::write(
            downloads.join("payload.bin"),
            if complete {
                &payload[..]
            } else {
                &payload[..16384]
            },
        )
        .unwrap();
        let handle = dispatcher.runtime.block_on(async {
            let h = dispatcher
                .inner
                .session
                .add_torrent(
                    dispatcher.runtime.cx(),
                    AddTorrent::from_bytes(bytes.clone()),
                    Some(AddTorrentOptions {
                        overwrite: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap()
                .into_handle()
                .unwrap();
            time::timeout(
                dispatcher.runtime.cx(),
                Duration::from_secs(5),
                h.wait_until_initialized(dispatcher.runtime.cx()),
            )
            .await
            .unwrap()
            .unwrap();
            h
        });
        assert_eq!(handle.stats().finished, complete);
        assert_eq!(
            handle.stats().progress_bytes,
            if complete { 32768 } else { 16384 }
        );
        let addr = dispatcher.inner.session.listen_addr().unwrap();
        let hash = handle.info_hash().as_string();
        let off = requested_payload(addr, &hash, &payload[..16384]);
        assert_eq!(off, 0, "default must refuse piece payload");
        let mut cfg = Config {
            upload: "on".into(),
            ..Default::default()
        };
        dispatcher.apply_settings(&cfg).unwrap();
        let on = requested_payload(addr, &hash, &payload[..16384]);
        assert_eq!(on, 16384, "on must serve verified data");
        handle.set_upload_enabled(Some(false));
        assert_eq!(
            requested_payload(addr, &hash, &payload[..16384]),
            0,
            "per-torrent off overrides global on"
        );
        cfg.upload = "off".into();
        dispatcher.apply_settings(&cfg).unwrap();
        handle.set_upload_enabled(Some(true));
        assert_eq!(
            requested_payload(addr, &hash, &payload[..16384]),
            16384,
            "explicit per-torrent on overrides global off"
        );
        handle.set_upload_enabled(None);
        assert_eq!(
            requested_payload(addr, &hash, &payload[..16384]),
            0,
            "removing override restores default off"
        );
        println!("upload phase={} default_payload_bytes={off} enabled_payload_bytes={on}; override off=0 on=16384",if complete {"completed"}else{"downloading"});
        drop(handle);
        drop(dispatcher);
        drop(app);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn upload_permission_is_independent_of_payload_cap_on_reload() {
    let directory = review_directory("upload-permission");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("torrents.json");
    for upload in [None, Some(false), Some(true)] {
        for cap in [0, 32768, 65536] {
            let mut record = review_record(&"a".repeat(40), &directory.join("downloads"));
            record.upload = upload;
            record.upload_limit = cap;
            record.only_files = Some(vec![0, 2]);
            let ledger = Ledger {records: vec![record], ..Default::default()};
            save_parked_ledger(&directory, &ledger).unwrap();
            let saved = read_ledger(&path).unwrap();
            let row = &saved.records[0];
            assert_eq!(row.upload, upload);
            assert_eq!(row.options(false).unwrap().upload_enabled, upload);
            assert_eq!(row.upload_limit, cap);
            assert_eq!(row.only_files, Some(vec![0, 2]));
            assert_eq!(row.stop_on_complete(), upload != Some(true));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
