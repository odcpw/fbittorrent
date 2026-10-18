//! Snapshot regressions through real initialization and isolated native sessions.
use crate::{
    AddTorrent, AddTorrentOptions, BitV, ManagedTorrent, ManagedTorrentState, Session,
    SessionOptions,
};
use rqbit_runtime::{Budget, Cx, RuntimeBuilder, time::timeout};
use sha1w::{ISha1, Sha1};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const BOUND: Duration = Duration::from_secs(5);

async fn session(cx: &Cx, directory: &Path) -> Arc<Session> {
    Session::new_with_opts(
        cx,
        directory.to_owned(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            concurrent_init_limit: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

async fn add(
    cx: &Cx,
    session: &Arc<Session>,
    directory: &Path,
    pieces: usize,
    paused: bool,
) -> Arc<ManagedTorrent> {
    // One-byte pieces keep the 10,000-piece fixture small. Every third piece
    // deliberately fails its real SHA-1 check, so an all-ones/zero stub fails.
    let payload: Vec<_> = (0..pieces)
        .map(|index| if index % 3 == 0 { b'y' } else { b'x' })
        .collect();
    std::fs::write(directory.join("payload.bin"), payload).unwrap();
    let mut torrent = format!(
        "d4:infod6:lengthi{pieces}e4:name11:payload.bin12:piece lengthi1e6:pieces{}:",
        pieces * 20
    )
    .into_bytes();
    let mut hash = Sha1::new();
    hash.update(b"x");
    let digest = hash.finish();
    for _ in 0..pieces {
        torrent.extend_from_slice(&digest);
    }
    torrent.extend_from_slice(b"7:privatei1eee");
    timeout(
        cx,
        BOUND,
        session.add_torrent(
            cx,
            AddTorrent::from_bytes(torrent),
            Some(AddTorrentOptions {
                paused,
                overwrite: true,
                disable_trackers: true,
                ..Default::default()
            }),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .into_handle()
    .unwrap()
}

fn measure(
    handle: &ManagedTorrent,
    case: &str,
    phase: &str,
    pieces: usize,
) -> (Option<Box<dyn BitV>>, Duration) {
    let started = Instant::now();
    let snapshot = handle.have_pieces_snapshot();
    let elapsed = started.elapsed();
    println!(
        "FBT-P506-SNAPSHOT {}",
        serde_json::json!({
            "case": case, "phase": phase, "pieces": pieces,
            "clone_ns": elapsed.as_nanos() as u64,
            "set_bits": snapshot.as_ref().map(|bits| bits.as_slice().count_ones()),
            "bytes": snapshot.as_ref().map(|bits| bits.as_bytes().len()),
            "present": snapshot.is_some(),
        })
    );
    (snapshot, elapsed)
}

#[test]
fn have_pieces_snapshot_live() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    runtime.block_on(async {
        let session = session(&cx, directory.path()).await;
        let handle = add(&cx, &session, directory.path(), 10, false).await;
        timeout(&cx, BOUND, handle.wait_until_initialized(&cx))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            handle.stats().state,
            crate::TorrentStatsState::Live
        ));
        let (snapshot, _) = measure(&handle, "have_pieces_snapshot_live", "live", 10);
        let mut snapshot = snapshot.unwrap();
        assert_eq!(
            snapshot.as_bytes(),
            [0x6d, 0x80],
            "verified pieces in MSB-first order, with six padding zeros"
        );
        assert_eq!(snapshot.as_slice().len(), 16);
        snapshot.as_slice_mut().set(0, true);
        snapshot.as_slice_mut().set(1, false);
        snapshot.flush(false).unwrap();
        assert_eq!(
            handle.have_pieces_snapshot().unwrap().as_bytes(),
            [0x6d, 0x80],
            "snapshot mutation/flush cannot change the tracker"
        );
        timeout(&cx, BOUND, session.pause(&cx, &handle))
            .await
            .unwrap()
            .unwrap();
        timeout(&cx, BOUND, session.stop(&cx))
            .await
            .unwrap()
            .unwrap();
        drop(handle);
        drop(session);
        assert_eq!(
            snapshot.as_bytes(),
            [0xad, 0x80],
            "snapshot survives tracker/session teardown"
        );
    });
}

#[test]
fn have_pieces_snapshot_paused() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    runtime.block_on(async {
        let session = session(&cx, directory.path()).await;
        // Hold initialization admission so the no-tracker observation is
        // deterministic even on a fast filesystem or under a busy scheduler.
        let permit = session
            .concurrent_initialize_semaphore
            .acquire(&cx, 1)
            .await
            .unwrap();
        let handle = add(&cx, &session, directory.path(), 10, true).await;
        assert!(matches!(
            handle.stats().state,
            crate::TorrentStatsState::Initializing { paused: true }
        ));
        assert!(
            measure(
                &handle,
                "have_pieces_snapshot_paused",
                "paused_initializing",
                10
            )
            .0
            .is_none()
        );
        drop(permit);
        timeout(&cx, BOUND, handle.wait_until_initialized(&cx))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            handle.stats().state,
            crate::TorrentStatsState::Paused
        ));
        assert_eq!(
            measure(
                &handle,
                "have_pieces_snapshot_paused",
                "paused_with_tracker",
                10
            )
            .0
            .unwrap()
            .as_bytes(),
            [0x6d, 0x80]
        );
        // Paused state normally owns a tracker. Explicitly exercise the
        // tracker-less state used while swapping states, retaining pause intent.
        let paused = handle.with_state_mut(ManagedTorrentState::take);
        assert!(handle.is_paused());
        assert!(
            measure(
                &handle,
                "have_pieces_snapshot_paused",
                "paused_without_state",
                10
            )
            .0
            .is_none()
        );
        handle.with_state_mut(|state| {
            *state = ManagedTorrentState::Error(anyhow::anyhow!("fixture error"))
        });
        assert!(
            measure(
                &handle,
                "have_pieces_snapshot_paused",
                "error_without_tracker",
                10
            )
            .0
            .is_none()
        );
        handle.with_state_mut(|state| *state = paused);
        timeout(&cx, BOUND, session.stop(&cx))
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn have_pieces_snapshot_timing() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    runtime.block_on(async {
        let session = session(&cx, directory.path()).await;
        let handle = add(&cx, &session, directory.path(), 10_000, false).await;
        timeout(&cx, BOUND, handle.wait_until_initialized(&cx))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            handle.stats().state,
            crate::TorrentStatsState::Live
        ));
        assert_eq!(
            handle
                .with_metadata(|metadata| metadata.lengths().total_pieces())
                .unwrap(),
            10_000
        );
        let (snapshot, elapsed) = measure(&handle, "have_pieces_snapshot_timing", "live", 10_000);
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.as_slice().len(), 10_000);
        assert_eq!(snapshot.as_bytes().len(), 1_250);
        assert_eq!(snapshot.as_slice().count_ones(), 6_666);
        for (index, bit) in snapshot.as_slice().iter().by_vals().enumerate() {
            assert_eq!(bit, index % 3 != 0, "piece {index}");
        }
        assert!(
            elapsed < Duration::from_millis(1),
            "10,000-piece snapshot took {elapsed:?}; limit is strictly below 1 ms"
        );
        timeout(&cx, BOUND, session.stop(&cx))
            .await
            .unwrap()
            .unwrap();
    });
}
