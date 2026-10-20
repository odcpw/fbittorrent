// The reference is the original first/last/sequential file traversal. Compare
// decisions, including overlapping files, unavailable peers and requeued pieces.
#[test]
fn picker_matches_original_order_with_overlap_and_peer_holes() {
    let mut files = make_test_file_infos(37);
    files[0].piece_range = 0..19;
    files[0].len = 18 * 16384 + 1000;
    files.push(crate::file_info::FileInfo {
        relative_filename: "second".into(),
        offset_in_torrent: 18 * 16384 + 1000,
        piece_range: 18..37,
        len: 19 * 16384 - 1000,
        attrs: Default::default(),
    });
    let order = vec![1, 0];
    let lengths = Lengths::new(37 * 16384, 16384).unwrap();
    let have = BF::from_boxed_slice(vec![0; lengths.piece_bitfield_bytes()].into());
    let mut selected = have.clone();
    selected[..37].fill(true);
    let mut tracker =
        PieceTracker::new(ChunkTracker::new(have.into_dyn(), selected, lengths, &files).unwrap());
    let mut remaining = vec![true; 37];
    for tick in 0..80 {
        let peer_has = |p: ValidPieceIndex| (p.get() + tick) % 5 != 0;
        let expected = order
            .iter()
            .flat_map(|i| files[*i].iter_piece_priorities())
            .find(|i| remaining[*i] && (*i as u32 + tick) % 5 != 0);
        let actual = tracker.acquire_piece(AcquireRequest {
            peer: peer(1),
            peer_avg_time: None,
            priority_pieces: std::iter::empty(),
            file_priorities: &order,
            file_infos: &files,
            peer_has_piece: peer_has,
            can_steal: |_| true,
        });
        match (expected, actual) {
            (Some(i), AcquireResult::Reserved(p)) => {
                assert_eq!(i as u32, p.get(), "tick {tick}");
                tracker.take_inflight(p);
                if tick % 7 == 0 {
                    tracker.mark_piece_hash_failed(p);
                } else {
                    tracker.mark_piece_hash_ok(p);
                    remaining[i] = false;
                }
            }
            (None, AcquireResult::NoneAvailable) => (),
            pair => panic!("different picker decision: {pair:?}"),
        }
    }
    assert!(!remaining.iter().any(|x| *x));
}

#[test]
#[ignore = "opt-in picker timing, not a shared-host latency assertion"]
fn picker_tick_benchmark() {
    use std::time::Instant;
    const PIECES: u32 = 16_384; // 4 GiB at the benchmark's 256 KiB piece size.
    let lengths = Lengths::new(u64::from(PIECES) * 256 * 1024, 256 * 1024).unwrap();
    let files = vec![crate::file_info::FileInfo {
        relative_filename: "4GiB.bin".into(),
        offset_in_torrent: 0,
        piece_range: 0..PIECES,
        len: lengths.total_length(),
        attrs: Default::default(),
    }];
    let order = vec![0];
    for have in [0, PIECES / 2, PIECES - 128] {
        let mut bits = BF::from_boxed_slice(vec![0; lengths.piece_bitfield_bytes()].into());
        let mut selected = bits.clone();
        selected[..PIECES as usize].fill(true);
        bits[..have as usize].fill(true);
        if have > 0 {
            bits.set(PIECES as usize - 1, true);
        }
        let chunks = ChunkTracker::new(bits.into_dyn(), selected, lengths, &files).unwrap();
        let mut tracker = PieceTracker::new(chunks);
        for trial in 0..13 {
            let started = Instant::now();
            for _ in 0..512 {
                let result = tracker.acquire_piece(AcquireRequest {
                    peer: peer(1),
                    peer_avg_time: None,
                    priority_pieces: std::iter::empty(),
                    file_priorities: &order,
                    file_infos: &files,
                    peer_has_piece: |_| true,
                    can_steal: |_| true,
                });
                let AcquireResult::Reserved(p) = result else {
                    panic!("{result:?}")
                };
                assert_eq!(p.get(), have);
                tracker.take_inflight(p);
                tracker.mark_piece_hash_failed(p);
                std::hint::black_box(&tracker);
            }
            println!(
                "PICKER {}",
                serde_json::json!({"have":have,"trial":trial,
                "warmup":trial<3,"ns_per_tick":started.elapsed().as_nanos() / 512})
            );
        }
    }
}
