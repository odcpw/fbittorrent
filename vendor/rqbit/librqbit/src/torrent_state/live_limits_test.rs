use super::*;

#[test]
fn live_limits_update() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("hello.txt"), b"hello").unwrap();
    let mut bytes =
        b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
    bytes.extend_from_slice(&[
        0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c,
        0xd9, 0xae, 0xa9, 0x43, 0x4d,
    ]);
    bytes.extend_from_slice(b"7:privatei1eee");
    let runtime = rqbit_runtime::RuntimeBuilder::new()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let _ambient = Cx::set_current(Some(cx.clone()));
    runtime.block_on(async {
        let session = Session::new_with_opts(&cx, directory.path().into(), crate::SessionOptions {
            dht: None, disable_trackers: true, disable_local_service_discovery: true,
            ..Default::default()
        }).await.unwrap();
        let handle = session.add_torrent(&cx, crate::AddTorrent::from_bytes(bytes), Some(crate::AddTorrentOptions {
            paused: true, overwrite: true, ..Default::default()
        })).await.unwrap().into_handle().unwrap();
        handle.wait_until_initialized(&cx).await.unwrap();
        assert!(handle.stats().finished);
        session.unpause(&cx, &handle).await.unwrap();
        let original = handle.live().unwrap();
        for cap in [Some(16384), Some(131072), None, Some(u32::MAX as u64), Some(32768)] {
            handle.set_upload_bps(cap).unwrap();
            assert_eq!(handle.shared.options.ratelimits.get_upload_bps().map(|v|v.get() as u64), cap);
            assert!(Arc::ptr_eq(&original, &handle.live().unwrap()));
            assert!(handle.stats().finished);
            assert!(matches!(handle.stats().state, TorrentStatsState::Live));
        }
        for cap in [0, 1, 16383, u32::MAX as u64 + 1, u64::MAX] {
            assert!(handle.set_upload_bps(Some(cap)).is_err());
            assert_eq!(handle.shared.options.ratelimits.get_upload_bps().unwrap().get(), 32768);
        }
        session.pause(&cx, &handle).await.unwrap();
        handle.set_upload_bps(Some(65536)).unwrap();
        session.unpause(&cx, &handle).await.unwrap();
        assert_eq!(handle.shared.options.ratelimits.get_upload_bps().unwrap().get(), 65536);
        assert!(matches!(handle.stats().state, TorrentStatsState::Live));
        assert!(handle.stats().finished);
        // Exercise the actual limiter, including its burst reservation.
        let limits = &handle.shared.options.ratelimits;
        limits.prepare_for_upload(&cx, std::num::NonZeroU32::new(65536).unwrap()).await.unwrap();
        let reservation = limits.prepare_for_upload(&cx, std::num::NonZeroU32::new(16384).unwrap());
        let mut reservation = std::pin::pin!(reservation);
        assert!(futures::poll!(reservation.as_mut()).is_pending());
        handle.set_upload_bps(None).unwrap();
        timeout(&cx, Duration::from_secs(1), reservation).await.unwrap().unwrap();
        println!("live_limits_update: one live state retained for all cap changes; invalid caps rejected; pause/resume retained 65536; finite bucket enforced; pending reservation completed after unlimited edit");
        session.stop(&cx).await.unwrap();
    });
}
