#[test]
fn piece_batch_growth_preserves_wrapped_unread_tail() {
    let mut rb = ReadBuf::new();
    rb.start = BUFLEN - 5;
    rb.len = 10;
    rb.buf[BUFLEN - 5..].copy_from_slice(b"hello");
    rb.buf[..5].copy_from_slice(b"world");
    rb.grow_for_piece_batch();
    assert_eq!(rb.buf.len(), super::BATCH_CAPACITY);
    assert_eq!((rb.start, rb.len, rb.retained_wire_len), (0, 10, 0));
    assert_eq!(&rb.buf[..10], b"helloworld");
    let allocation = rb.buf.as_ptr();
    rb.grow_for_piece_batch();
    assert_eq!(rb.buf.as_ptr(), allocation);
}

#[test]
fn piece_batch_intake_reserves_wrapped_wire_and_preserves_allocation() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let key = cx
        .reusable_region_owner_key()
        .expect("native unrestricted caller");
    runtime.block_on(async {
        let mut wire = Vec::new();
        for value in [0x11, 0x22, 0x33, 0x44] {
            wire.extend_from_slice(&piece_wire(&vec![value; CHUNK_SIZE as usize]));
        }
        // If intake ignored the consumed-wire reservation this suffix
        // would overwrite the first payload when the next read wraps.
        wire.extend_from_slice(&[0, 0, 0, 65, 5]);
        wire.extend_from_slice(&[0xa5; 64]);
        let mut src = input(
            wire,
            2 * peer_binary_protocol::PIECE_MESSAGE_DEFAULT_LEN
                + peer_binary_protocol::PIECE_MESSAGE_DEFAULT_LEN / 2,
        );
        let mut rb = ReadBuf::new();
        rb.grow_for_piece_batch();
        rb.start = rb.buf.len() - 15;
        let allocation = rb.buf.as_ptr();
        let meta = {
            let Message::Piece(piece) = rb
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!("expected a Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let mut batch = rb
            .into_piece_frame(meta.0, meta.1, meta.2, meta.3)
            .unwrap()
            .into_batch();
        batch.intake_ready(&cx, &key, &mut src).await.unwrap();
        assert_eq!(batch.remaining_len(), 4);
        assert_eq!(batch.read_buf.buf.as_ptr(), allocation);
        assert_eq!(batch.read_buf.retained_wire_len, super::BATCH_CAPACITY);
        let available = batch
            .read_buf
            .unfilled_ioslices()
            .iter()
            .map(|part| part.len())
            .sum::<usize>();
        assert_eq!(available, 0);
        for (index, value) in [0x11, 0x22, 0x33, 0x44].into_iter().enumerate() {
            let piece = batch.as_piece(index);
            assert_eq!(
                [piece.data().0, piece.data().1].concat(),
                vec![value; CHUNK_SIZE as usize]
            );
            // This test exercises intake/layout only; physical processing
            // and region drain are covered by the native ownership tests.
            batch.complete_current();
        }
        let outcome = batch.into_receive_outcome();
        assert!(outcome.deferred_read_error.is_none());
        let mut returned = outcome.read_buf;
        assert_eq!(returned.buf.as_ptr(), allocation);
        assert_eq!(returned.retained_wire_len, 0);
        let Message::Bitfield(bitfield) = returned
            .read_message(&cx, &mut src, Duration::from_secs(1))
            .await
            .unwrap()
        else {
            panic!("expected the untouched control suffix");
        };
        assert_eq!(bitfield.0, &[0xa5; 64]);
    });
}

#[test]
fn piece_batch_intake_fences_control_and_defers_truncated_piece() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let key = cx
        .reusable_region_owner_key()
        .expect("native unrestricted caller");
    runtime.block_on(async {
        let mut wire = piece_wire(b"first");
        wire.extend_from_slice(&[0, 0, 0, 5, 4, 0, 0, 0, 99]);
        wire.extend_from_slice(&piece_wire(b"later"));
        let mut src = input(wire, usize::MAX);
        let mut rb = ReadBuf::new();
        rb.grow_for_piece_batch();
        let meta = {
            let Message::Piece(piece) = rb
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!("expected first Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let mut batch = rb
            .into_piece_frame(meta.0, meta.1, meta.2, meta.3)
            .unwrap()
            .into_batch();
        batch.intake_ready(&cx, &key, &mut src).await.unwrap();
        assert_eq!(batch.remaining_len(), 1);
        batch.complete_current();
        let outcome = batch.into_receive_outcome();
        assert!(outcome.deferred_read_error.is_none());
        let mut returned = outcome.read_buf;
        assert!(matches!(
            returned
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap(),
            Message::Have(99)
        ));
        let Message::Piece(piece) = returned
            .read_message(&cx, &mut src, Duration::from_secs(1))
            .await
            .unwrap()
        else {
            panic!("expected Piece after control fence");
        };
        assert_eq!([piece.data().0, piece.data().1].concat(), b"later");

        let mut wire = piece_wire(b"first");
        wire.extend_from_slice(&piece_wire(b"second"));
        let mut truncated = piece_wire(b"partial");
        truncated.pop();
        wire.extend_from_slice(&truncated);
        let mut src = input(wire, usize::MAX);
        let mut rb = ReadBuf::new();
        rb.grow_for_piece_batch();
        let meta = {
            let Message::Piece(piece) = rb
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!("expected first Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let mut batch = rb
            .into_piece_frame(meta.0, meta.1, meta.2, meta.3)
            .unwrap()
            .into_batch();
        batch.intake_ready(&cx, &key, &mut src).await.unwrap();
        assert_eq!(batch.remaining_len(), 2);
        assert_eq!(
            [batch.as_piece(1).data().0, batch.as_piece(1).data().1].concat(),
            b"second"
        );
        batch.complete_current();
        batch.complete_current();
        assert!(matches!(
            batch.into_receive_outcome().deferred_read_error,
            Some(crate::Error::PeerDisconnected)
        ));
    });
}

#[test]
fn piece_batch_grown_buffer_preserves_original_wire_frame_limit() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    runtime.block_on(async {
        let mut rb = ReadBuf::new();
        rb.grow_for_piece_batch();
        let mut src = input(piece_wire(&vec![0x55; BUFLEN]), usize::MAX);
        assert!(matches!(
            rb.read_message(&cx, &mut src, Duration::from_secs(1)).await,
            Err(crate::Error::ReadBufFull { .. })
        ));
        assert_eq!(rb.start, 0);
        assert_eq!(rb.len, BUFLEN);
    });
}
