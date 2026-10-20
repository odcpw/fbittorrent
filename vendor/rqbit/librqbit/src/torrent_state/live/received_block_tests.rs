use super::*;

const RUN_PIECE_BYTES: usize = 4 * CHUNK_SIZE as usize;

fn run_fixture_metainfo(multifile: bool) -> (Vec<u8>, Vec<u8>) {
    let payload: Vec<u8> = (0..2 * RUN_PIECE_BYTES)
        .map(|index| ((index * 71 + index / 251) % 256) as u8)
        .collect();
    let mut torrent = b"d4:infod".to_vec();
    if multifile {
        torrent.extend_from_slice(b"5:filesl");
        for (name, size) in [("a.bin", 24577), ("empty.bin", 0), ("b.bin", 106495)] {
            torrent.extend_from_slice(
                format!("d6:lengthi{size}e4:pathl{}:{name}ee", name.len()).as_bytes(),
            );
        }
        torrent.extend_from_slice(b"e4:name4:root");
    } else {
        torrent.extend_from_slice(b"6:lengthi131072e4:name11:payload.bin");
    }
    torrent.extend_from_slice(b"12:piece lengthi65536e6:pieces40:");
    for piece in payload.chunks(RUN_PIECE_BYTES) {
        let mut hash = sha1w::Sha1::new();
        hash.update(piece);
        torrent.extend_from_slice(&hash.finish());
    }
    torrent.extend_from_slice(b"ee");
    (payload, torrent)
}

fn run_fixture_live(
    fixture: &NativeSession,
    metainfo: &[u8],
) -> (
    crate::torrent_state::ManagedTorrentHandle,
    Arc<TorrentStateLive>,
) {
    let handle = fixture
        .block_on(fixture.session.add_torrent(
            &fixture.cleanup,
            AddTorrent::from_bytes(metainfo.to_vec()),
            Some(AddTorrentOptions {
                paused: true,
                overwrite: true,
                disable_trackers: true,
                ..Default::default()
            }),
        ))
        .unwrap()
        .into_handle()
        .unwrap();
    fixture
        .block_on(handle.wait_until_initialized(&fixture.cleanup))
        .unwrap();
    let run_cx = handle.run_region.lock().as_ref().unwrap().cx().clone();
    let paused = handle.with_state_mut(|state| match state.take() {
        ManagedTorrentState::Paused(paused) => paused,
        other => panic!("expected checked paused storage, got {}", other.name()),
    });
    assert_eq!(
        paused
            .chunk_tracker
            .get_have_pieces()
            .as_slice()
            .count_ones(),
        0,
        "partial/unverified data must not be restored as Have"
    );
    let (fatal_tx, _fatal_rx) = rqbit_runtime::channel::oneshot::channel();
    let live = TorrentStateLive::new(
        &run_cx,
        paused,
        fatal_tx,
        fixture.session.cancellation_token().child_token(),
    )
    .unwrap();
    {
        let mut managed = handle.locked.write();
        managed.paused = false;
        managed.state = ManagedTorrentState::Live(live.clone());
    }
    (handle, live)
}

#[derive(Clone, Copy)]
enum ReceiveFault {
    None,
    Corrupt,
    CancelBefore,
    Unsolicited,
}

fn receive_run(
    fixture: &NativeSession,
    live: &Arc<TorrentStateLive>,
    payload: &[u8],
    order: &[usize],
    fault: ReceiveFault,
) -> anyhow::Result<()> {
    let addr: SocketAddr = "127.0.0.1:45124".parse().unwrap();
    let (tx, _rx) = unbounded_channel();
    let peer = Peer::new_live_for_incoming_connection(
        addr,
        Id20::new([0x62; 20]),
        tx.clone(),
        &live.peers,
        ConnectionKind::Tcp,
    );
    let counters = peer.stats.counters.clone();
    assert!(live.peers.states.insert(addr, peer).is_none());
    let piece = live.lengths.validate_piece_index(0).unwrap();
    let chunks: Vec<_> = (0..4)
        .map(|index| {
            live.lengths
                .chunk_info_from_received_data(piece, index * CHUNK_SIZE, CHUNK_SIZE)
                .unwrap()
        })
        .collect();
    {
        let mut guard = live.lock_write("received blocks test reserve");
        let priorities = guard.file_priorities.clone();
        let acquired = guard
            .get_pieces_mut()
            .unwrap()
            .acquire_piece(AcquireRequest {
                peer: addr,
                peer_avg_time: None,
                priority_pieces: std::iter::once(piece),
                file_priorities: &priorities,
                file_infos: &live.metadata.file_infos,
                peer_has_piece: |candidate| candidate == piece,
                can_steal: |_| false,
            });
        assert!(matches!(acquired, AcquireResult::Reserved(actual) if actual == piece));
    }
    live.peers
        .with_live_mut(addr, "received blocks test requests", |peer| {
            for (index, chunk) in chunks.iter().enumerate() {
                if !matches!(fault, ReceiveFault::Unsolicited) || index != 2 {
                    assert!(peer.add_inflight_request(*chunk));
                }
            }
        })
        .unwrap();
    let mut wire = Vec::new();
    for &index in order {
        let begin = index * CHUNK_SIZE as usize;
        let mut block = payload[begin..begin + CHUNK_SIZE as usize].to_vec();
        if matches!(fault, ReceiveFault::Corrupt) && index == 2 {
            block[77] ^= 1;
        }
        let mut frame = vec![0; 13 + block.len()];
        Message::Piece(Piece::from_data(0, begin as u32, &block))
            .serialize(&mut frame, &Default::default)
            .unwrap();
        wire.extend(frame);
    }
    wire.extend([0, 0, 0, 0]);
    let run_cx = live.cx.clone();
    let live = live.clone();
    let task = rqbit_runtime::task::spawn_with(&run_cx, move |origin| async move {
        let storage = live.shared.spawner.lease(&origin).await?;
        storage.run(&origin, || ()).await?;
        assert!(storage.batch_domain(&origin).is_some());
        let handler = PeerHandler {
            storage,
            state: live.clone(),
            counters,
            flow_control: Mutex::new(PeerFlowControl::default()),
            on_bitfield_notify: Notify::new(),
            addr,
            incoming: true,
            tx,
            first_message_received: AtomicBool::new(false),
            cancel_token: live.cancellation_token.child_token(),
            client_name_and_version: live.shared.client_name_and_version().to_owned(),
        };
        let mut input: BoxAsyncReadVectored = Box::new(WireInput {
            wire,
            consumed: Arc::new(AtomicUsize::new(0)),
            polls: Arc::new(AtomicUsize::new(0)),
        });
        let mut receive = ReadBuf::new();
        receive.grow_for_piece_batch();
        let meta = {
            let Message::Piece(piece) = receive
                .read_message(&origin, &mut input, Duration::from_secs(6))
                .await?
            else {
                anyhow::bail!("expected first Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let frame = receive.into_piece_frame(meta.0, meta.1, meta.2, meta.3)?;
        if matches!(fault, ReceiveFault::CancelBefore) {
            origin.set_cancel_requested(true);
        }
        handler
            .on_received_piece_frame(&origin, frame, &mut input)
            .await
            .map(|_| ())
    });
    fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        Duration::from_secs(6),
        task,
    ))??
}

fn assert_first_piece(live: &TorrentStateLive, verified: bool) {
    let guard = live.lock_read("received blocks test have");
    let tracker = guard.get_chunks().unwrap();
    assert_eq!(
        tracker.get_have_pieces().as_slice().count_ones(),
        usize::from(verified)
    );
    drop(guard);
    assert_eq!(
        live.stats_snapshot().downloaded_and_checked_bytes,
        if verified { RUN_PIECE_BYTES as u64 } else { 0 }
    );
}

#[test]
fn native_received_blocks_reordered_blocks_verify_before_have() {
    for (order, corrupt) in [
        ([2, 0, 3, 1], false),
        ([2, 0, 3, 1], true),
        ([0, 1, 2, 3], true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let trace = Arc::new(FileTrace::default());
        let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
        let (payload, metainfo) = run_fixture_metainfo(false);
        let (handle, live) = run_fixture_live(&fixture, &metainfo);
        let mut have = live.have_broadcast_tx.subscribe();
        let result = receive_run(
            &fixture,
            &live,
            &payload,
            &order,
            if corrupt {
                ReceiveFault::Corrupt
            } else {
                ReceiveFault::None
            },
        );
        assert_eq!(result.is_ok(), !corrupt, "{result:?}");
        assert_first_piece(&live, !corrupt);
        assert_eq!(have.try_recv().is_ok(), !corrupt);
        if !corrupt {
            assert_eq!(
                &std::fs::read(directory.path().join("payload.bin")).unwrap()[..RUN_PIECE_BYTES],
                &payload[..RUN_PIECE_BYTES]
            );
        }
        handle.pause().unwrap();
        fixture.finish();
    }
}

#[test]
fn native_received_blocks_splits_multifile_boundaries_and_empty_files() {
    let directory = tempfile::tempdir().unwrap();
    let trace = Arc::new(FileTrace::default());
    let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
    let (payload, metainfo) = run_fixture_metainfo(true);
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    receive_run(&fixture, &live, &payload, &[0, 1, 2, 3], ReceiveFault::None).unwrap();
    assert_first_piece(&live, true);
    assert_eq!(
        std::fs::read(directory.path().join("root/a.bin")).unwrap(),
        payload[..24577]
    );
    assert!(
        std::fs::read(directory.path().join("root/empty.bin"))
            .unwrap()
            .is_empty()
    );
    let tail = std::fs::read(directory.path().join("root/b.bin")).unwrap();
    assert_eq!(
        &tail[..RUN_PIECE_BYTES - 24577],
        &payload[24577..RUN_PIECE_BYTES]
    );
    assert!(
        tail[RUN_PIECE_BYTES - 24577..]
            .iter()
            .all(|byte| *byte == 0)
    );
    handle.pause().unwrap();
    fixture.finish();
}

#[test]
fn native_received_blocks_stop_mid_piece_rechecks_and_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let (payload, metainfo) = run_fixture_metainfo(false);
    {
        let fixture = NativeSession::new(directory.path().to_owned(), Arc::default());
        let (handle, live) = run_fixture_live(&fixture, &metainfo);
        receive_run(&fixture, &live, &payload, &[0, 1], ReceiveFault::None).unwrap();
        assert_first_piece(&live, false);
        handle.pause().unwrap();
        fixture.finish();
    }
    let fixture = NativeSession::new(directory.path().to_owned(), Arc::default());
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    receive_run(&fixture, &live, &payload, &[0, 1, 2, 3], ReceiveFault::None).unwrap();
    assert_first_piece(&live, true);
    assert_eq!(
        &std::fs::read(directory.path().join("payload.bin")).unwrap()[..RUN_PIECE_BYTES],
        &payload[..RUN_PIECE_BYTES]
    );
    handle.pause().unwrap();
    fixture.finish();
}

#[test]
fn native_received_blocks_cancelled_owner_writes_and_publishes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let trace = Arc::new(FileTrace::default());
    let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
    let (payload, metainfo) = run_fixture_metainfo(false);
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    assert!(
        receive_run(
            &fixture,
            &live,
            &payload,
            &[0, 1, 2, 3],
            ReceiveFault::CancelBefore
        )
        .is_err()
    );
    assert!(trace.attempts.lock().unwrap().is_empty());
    assert_first_piece(&live, false);
    handle.pause().unwrap();
    fixture.finish();
}

struct CrashFixtureChild(std::process::Child);

impl Drop for CrashFixtureChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn native_received_blocks_process_exit_mid_piece_never_restores_have() {
    const CHILD_ROOT: &str = "FBT_RECEIVED_BLOCKS_CRASH_ROOT";
    let (payload, metainfo) = run_fixture_metainfo(false);
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let fixture = NativeSession::new(root.into(), Arc::default());
        let (_handle, live) = run_fixture_live(&fixture, &metainfo);
        receive_run(&fixture, &live, &payload, &[0, 1], ReceiveFault::None).unwrap();
        assert_first_piece(&live, false);
        // Abrupt process exit: no pause, destructor, session drain or resume
        // flush. This proves process-crash recovery, not power-loss durability.
        std::process::exit(73);
    }
    let directory = tempfile::tempdir().unwrap();
    let mut child = CrashFixtureChild(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "torrent_state::live::live_fatal_write_tests::received_blocks::native_received_blocks_process_exit_mid_piece_never_restores_have", "--nocapture"])
        .env(CHILD_ROOT, directory.path())
        .env("HOME", directory.path())
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_RUNTIME_DIR", directory.path().join("runtime"))
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            child.0.wait().unwrap();
            panic!("private crash fixture timed out and was reaped");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        status.code(),
        Some(73),
        "child must exit after real partial writes"
    );
    let partial = std::fs::read(directory.path().join("payload.bin")).unwrap();
    assert_eq!(
        &partial[..2 * CHUNK_SIZE as usize],
        &payload[..2 * CHUNK_SIZE as usize]
    );
    assert!(
        partial[2 * CHUNK_SIZE as usize..]
            .iter()
            .all(|byte| *byte == 0)
    );
    let fixture = NativeSession::new(directory.path().to_owned(), Arc::default());
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    receive_run(&fixture, &live, &payload, &[0, 1, 2, 3], ReceiveFault::None).unwrap();
    assert_first_piece(&live, true);
    handle.pause().unwrap();
    fixture.finish();
}

#[test]
fn native_received_blocks_partial_multifile_failure_never_marks_or_announces_have() {
    let directory = tempfile::tempdir().unwrap();
    let trace = Arc::new(FileTrace::default());
    let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
    let (payload, metainfo) = run_fixture_metainfo(true);
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    let mut have = live.have_broadcast_tx.subscribe();
    trace.fault_file.store(2, Ordering::Release);
    trace.armed.store(true, Ordering::Release);
    assert!(receive_run(&fixture, &live, &payload, &[0, 1, 2, 3], ReceiveFault::None).is_err());
    assert!(!trace.armed.load(Ordering::Acquire));
    assert!(
        live.lock_read("partial write must signal fatal storage failure")
            .fatal_errors_tx
            .is_none()
    );
    assert_eq!(
        std::fs::read(directory.path().join("root/a.bin")).unwrap(),
        payload[..24577]
    );
    assert!(
        std::fs::read(directory.path().join("root/b.bin"))
            .unwrap()
            .iter()
            .all(|byte| *byte == 0)
    );
    assert_first_piece(&live, false);
    assert!(have.try_recv().is_err());
    assert!(trace.completed.lock().unwrap().is_empty());
    handle.pause().unwrap();
    fixture.finish();
    let fixture = NativeSession::new(directory.path().to_owned(), Arc::default());
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    receive_run(&fixture, &live, &payload, &[0, 1, 2, 3], ReceiveFault::None).unwrap();
    assert_first_piece(&live, true);
    handle.pause().unwrap();
    fixture.finish();
}

#[test]
fn native_received_blocks_unsolicited_block_keeps_prefix_and_stops_later_writes() {
    let directory = tempfile::tempdir().unwrap();
    let trace = Arc::new(FileTrace::default());
    let fixture = NativeSession::new(directory.path().to_owned(), trace.clone());
    let (payload, metainfo) = run_fixture_metainfo(false);
    let (handle, live) = run_fixture_live(&fixture, &metainfo);
    assert!(
        receive_run(
            &fixture,
            &live,
            &payload,
            &[0, 1, 2, 3],
            ReceiveFault::Unsolicited,
        )
        .is_err()
    );
    assert!(
        live.lock_read("unsolicited block is a peer error")
            .fatal_errors_tx
            .is_some()
    );
    assert_first_piece(&live, false);
    let bytes = std::fs::read(directory.path().join("payload.bin")).unwrap();
    assert_eq!(
        &bytes[..2 * CHUNK_SIZE as usize],
        &payload[..2 * CHUNK_SIZE as usize]
    );
    assert!(
        bytes[2 * CHUNK_SIZE as usize..]
            .iter()
            .all(|byte| *byte == 0)
    );
    handle.pause().unwrap();
    fixture.finish();
}
