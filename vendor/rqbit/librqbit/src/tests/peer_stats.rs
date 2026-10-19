//! Real seed/leecher sessions, including six unused padding bits in the bitfield.
use crate::{
    AddTorrent, AddTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions,
};
use rqbit_runtime::{Budget, RuntimeBuilder, time};
use sha1w::{ISha1, Sha1};
use std::{num::NonZeroU32, time::Duration};
#[test]
fn peer_stats_seed_flag() {
    let root = tempfile::tempdir().unwrap();
    let seed_dir = root.path().join("seed");
    let leecher_dir = root.path().join("leecher");
    std::fs::create_dir_all(&seed_dir).unwrap();
    std::fs::create_dir_all(&leecher_dir).unwrap();
    let payload = vec![b'x'; 16384 * 10];
    std::fs::write(seed_dir.join("payload.bin"), &payload).unwrap();
    let mut hash = Sha1::new();
    hash.update(&payload[..16384]);
    let digest = hash.finish();
    let mut metainfo =
        b"d4:infod6:lengthi163840e4:name11:payload.bin12:piece lengthi16384e6:pieces200:".to_vec();
    for _ in 0..10 {
        metainfo.extend_from_slice(&digest);
    }
    metainfo.extend_from_slice(b"ee");
    let rt = RuntimeBuilder::current_thread().build().unwrap();
    let cx = rt.request_cx_with_budget(Budget::INFINITE);
    rt.block_on(async {
        let options = || SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let seed = Session::new_with_opts(&cx, seed_dir.clone(), options())
            .await
            .unwrap();
        let leecher = Session::new_with_opts(&cx, leecher_dir, options())
            .await
            .unwrap();
        let seed_handle = seed
            .add_torrent(
                &cx,
                AddTorrent::from_bytes(metainfo.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    ratelimits: crate::limits::LimitsConfig {
                        upload_bps: NonZeroU32::new(16384),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        seed_handle.wait_until_initialized(&cx).await.unwrap();
        let client = leecher
            .add_torrent(
                &cx,
                AddTorrent::from_bytes(metainfo),
                Some(AddTorrentOptions {
                    initial_peers: Some(vec![seed.listen_addr().unwrap()]),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        let result = time::timeout(&cx, Duration::from_secs(10), async {
            loop {
                let from_client = client
                    .live()
                    .map(|l| l.per_peer_stats_snapshot(Default::default()));
                let from_seed = seed_handle
                    .live()
                    .map(|l| l.per_peer_stats_snapshot(Default::default()));
                if let (Some(c), Some(s)) = (from_client, from_seed) {
                    if c.peers.values().any(|p| p.seed) && s.peers.values().any(|p| !p.seed) {
                        println!(
                            "peer_stats_seed_flag {}",
                            serde_json::json!({"from_leecher":c,"from_seed":s})
                        );
                        assert_eq!(client.live().unwrap().connected_seed_count(), 1);
                        assert_eq!(seed_handle.live().unwrap().connected_seed_count(), 0);
                        assert!(client.stats().progress_bytes < client.stats().total_bytes);
                        break;
                    }
                }
                time::sleep(&cx, Duration::from_millis(25)).await;
            }
        })
        .await;
        seed.stop(&cx).await.unwrap();
        leecher.stop(&cx).await.unwrap();
        result.expect("actual local seed and incomplete leecher must exchange bitfields");
    });
}
