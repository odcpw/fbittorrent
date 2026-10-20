#[test]
fn owned_piece_frame_preserves_wrapped_payload_tail_and_allocation() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    runtime.block_on(async {
        for payload in [b"hello".to_vec(), vec![0x5a; CHUNK_SIZE as usize]] {
            for start in std::iter::once(0).chain(BUFLEN - 20..BUFLEN) {
                let mut wire = piece_wire(&payload);
                // A HAVE and keepalive already in the same socket read.
                wire.extend_from_slice(&[0, 0, 0, 5, 4, 0, 0, 0, 99, 0, 0, 0, 0]);
                let mut src = input(wire, usize::MAX);
                let mut rb = ReadBuf::new();
                rb.start = start;
                let allocation = rb.buf.as_ptr();
                let (index, begin, payload_len, first_len, first, second, pointers) = {
                    let Message::Piece(piece) = rb
                        .read_message(&cx, &mut src, Duration::from_secs(1))
                        .await
                        .unwrap()
                    else {
                        panic!("expected a Piece");
                    };
                    let (first, second) = piece.data();
                    assert_eq!([first, second].concat(), payload);
                    (
                        piece.index,
                        piece.begin,
                        piece.len(),
                        first.len(),
                        first.to_vec(),
                        second.to_vec(),
                        (first.as_ptr(), second.as_ptr()),
                    )
                };
                let tail_state = (rb.start, rb.len);
                let frame = rb
                    .into_piece_frame(index, begin, payload_len, first_len)
                    .unwrap();
                assert_eq!(frame.read_buf.buf.as_ptr(), allocation);
                let borrowed = frame.as_piece();
                assert_eq!((borrowed.index, borrowed.begin), (42, 43));
                assert_eq!(borrowed.data(), (first.as_slice(), second.as_slice()));
                if !first.is_empty() {
                    assert_eq!(borrowed.data().0.as_ptr(), pointers.0);
                }
                if !second.is_empty() {
                    assert_eq!(borrowed.data().1.as_ptr(), pointers.1);
                }
                let mut returned = frame.into_read_buf();
                assert_eq!(returned.buf.as_ptr(), allocation);
                assert_eq!((returned.start, returned.len), tail_state);
                assert!(matches!(
                    returned
                        .read_message(&cx, &mut src, Duration::from_secs(1))
                        .await
                        .unwrap(),
                    Message::Have(99)
                ));
                assert!(matches!(
                    returned
                        .read_message(&cx, &mut src, Duration::from_secs(1))
                        .await
                        .unwrap(),
                    Message::KeepAlive
                ));
                assert_eq!(returned.len, 0);
            }
        }
    });
}

#[test]
fn owned_piece_frame_preserves_fragmented_headers() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    runtime.block_on(async {
        for start in std::iter::once(0).chain(BUFLEN - 13..BUFLEN) {
            let mut rb = ReadBuf::new();
            rb.start = start;
            let allocation = rb.buf.as_ptr();
            let mut src = input(piece_wire(b"hello"), 1);
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
            let frame = rb.into_piece_frame(meta.0, meta.1, meta.2, meta.3).unwrap();
            assert_eq!(
                [frame.as_piece().data().0, frame.as_piece().data().1].concat(),
                b"hello"
            );
            let mut returned = frame.into_read_buf();
            assert_eq!(returned.buf.as_ptr(), allocation);
            assert!(matches!(
                returned
                    .read_message(&cx, &mut src, Duration::from_secs(1))
                    .await,
                Err(crate::Error::PeerDisconnected)
            ));
        }
    });
}

#[test]
fn owned_piece_frame_rejects_invalid_memory_bounds() {
    assert!(
        ReadBuf::new()
            .into_piece_frame(1, 0, usize::MAX, 0)
            .is_err()
    );
    assert!(ReadBuf::new().into_piece_frame(1, 0, 5, 6).is_err());
    let mut rb = ReadBuf::new();
    rb.start = BUFLEN;
    assert!(rb.into_piece_frame(1, 0, 5, 5).is_err());
    let mut rb = ReadBuf::new();
    rb.len = BUFLEN;
    assert!(rb.into_piece_frame(1, 0, 1, 1).is_err());
    let mut rb = ReadBuf::new();
    rb.start = 2;
    assert!(rb.into_piece_frame(1, 0, 5, 5).is_err());
    let mut rb = ReadBuf::new();
    rb.start = 2;
    assert!(rb.into_piece_frame(1, 0, 5, 0).is_err());
    // Ownership validates memory bounds; protocol/chunk validity stays in
    // the existing parser and handler, including zero-length metadata.
    assert_eq!(
        ReadBuf::new()
            .into_piece_frame(1, 0, 0, 0)
            .unwrap()
            .as_piece()
            .len(),
        0
    );
}

#[test]
fn owned_piece_frame_preserves_malformed_tail_and_parser_boundaries() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    runtime.block_on(async {
        let mut wire = piece_wire(b"hello");
        wire.extend_from_slice(&[0, 0, 0, 1, 99]);
        let mut src = input(wire, usize::MAX);
        let mut rb = ReadBuf::new();
        rb.start = BUFLEN - 3;
        let meta = {
            let Message::Piece(piece) = rb
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!("expected a Piece before the malformed tail");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let frame = rb.into_piece_frame(meta.0, meta.1, meta.2, meta.3).unwrap();
        assert_eq!(
            [frame.as_piece().data().0, frame.as_piece().data().1].concat(),
            b"hello"
        );
        let mut returned = frame.into_read_buf();
        assert!(matches!(
            returned
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await,
            Err(crate::Error::Deserialize(
                peer_binary_protocol::MessageDeserializeError::UnsupportedMessageId(99)
            ))
        ));

        let mut src = input(piece_wire(&[]), 1);
        let mut rb = ReadBuf::new();
        assert!(matches!(
            rb.read_message(&cx, &mut src, Duration::from_secs(1)).await,
            Err(crate::Error::Deserialize(
                peer_binary_protocol::MessageDeserializeError::IncorrectMsgLen { .. }
            ))
        ));
        assert_eq!(rb.start, 0);

        let mut truncated = piece_wire(b"hello");
        truncated.pop();
        let mut src = input(truncated, 1);
        let mut rb = ReadBuf::new();
        assert!(matches!(
            rb.read_message(&cx, &mut src, Duration::from_secs(1)).await,
            Err(crate::Error::PeerDisconnected)
        ));
        assert_eq!(rb.start, 0);

        let mut oversized = vec![0; BUFLEN];
        oversized[..4].copy_from_slice(&(BUFLEN as u32).to_be_bytes());
        oversized[4] = 7;
        let mut src = input(oversized, usize::MAX);
        let mut rb = ReadBuf::new();
        assert!(matches!(
            rb.read_message(&cx, &mut src, Duration::from_secs(1)).await,
            Err(crate::Error::ReadBufFull { .. })
        ));
        assert_eq!(rb.start, 0);
    });
}
