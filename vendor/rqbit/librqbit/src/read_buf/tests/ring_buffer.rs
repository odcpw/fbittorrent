#[test]
fn test_ringbuf_ranges() {
    let mut b = ReadBuf::new();
    assert_eq!(b.as_slice_ranges(), (0..0, 0..0));

    b.start = 10;
    b.len = 10;
    assert_eq!(b.as_slice_ranges(), (10..20, 0..0));

    b.start = BUFLEN - 100;
    b.len = 100;
    assert_eq!(b.as_slice_ranges(), (BUFLEN - 100..BUFLEN, 0..0));

    b.start = BUFLEN - 100;
    b.len = 120;
    assert_eq!(b.as_slice_ranges(), (BUFLEN - 100..BUFLEN, 0..20));

    b.start = BUFLEN - 100;
    b.len = BUFLEN;
    assert_eq!(b.as_slice_ranges(), (BUFLEN - 100..BUFLEN, 0..BUFLEN - 100));
}

#[test]
fn test_ringbuf_advance() {
    let mut b = ReadBuf::new();

    b.start = 10;
    b.len = 10;
    assert_eq!(b.as_slice_ranges(), (10..20, 0..0));
    b.advance(5);
    assert_eq!(b.as_slice_ranges(), (15..20, 0..0));

    b.start = BUFLEN - 5;
    b.len = 10;
    assert_eq!(b.as_slice_ranges(), (BUFLEN - 5..BUFLEN, 0..5));
    b.advance(5);
    assert_eq!(b.as_slice_ranges(), (0..5, 0..0));
}

#[test]
fn test_ringbuf_make_contiguous() {
    let mut b = ReadBuf::new();
    assert!(b.is_contiguous());
    assert!(b.make_contiguous().is_err());

    fn fill(buf: &mut [u8], value: u8) {
        for b in buf.iter_mut() {
            *b = value;
        }
    }

    fill(&mut b.buf[BUFLEN - 100..], 42);
    fill(&mut b.buf[..200], 43);
    b.start = BUFLEN - 100;
    b.len = 300;
    assert!(!b.is_contiguous());
    assert!(b.make_contiguous().is_ok());
    assert_eq!(b.len, 300);
    assert_eq!(b.start, 0);
    assert_eq!(&b.buf[..100], &[42u8; 100]);
    assert_eq!(&b.buf[100..300], &[43u8; 200]);
    assert_eq!(&b.buf[300..], &[0u8; BUFLEN - 300]);
}

#[test]
fn test_unfilled_ioslices() {
    ReadBuf::new();

    fn offset(buf: &ReadBuf, s: *const u8, len: usize) -> std::ops::Range<usize> {
        if len == 0 {
            return 0..0;
        }
        let offset = unsafe { s.byte_offset_from(buf.buf.as_ptr()) } as usize;
        offset..offset + len
    }

    fn offsets(
        buf: &ReadBuf,
        ioslices: [(*const u8, usize); 2],
    ) -> [std::ops::Range<usize>; 2] {
        [
            offset(buf, ioslices[0].0, ioslices[0].1),
            offset(buf, ioslices[1].0, ioslices[1].1),
        ]
    }

    #[track_caller]
    fn assert_one(start: usize, len: usize, ranges: [std::ops::Range<usize>; 2]) {
        let mut b = ReadBuf::new();
        b.start = start;
        b.len = len;
        let [f, s] = b.unfilled_ioslices();
        let slices = [(f.as_ptr(), f.len()), (s.as_ptr(), s.len())];
        assert_eq!(offsets(&b, slices), ranges)
    }

    // full
    assert_one(0, BUFLEN, [0..0, 0..0]);
    assert_one(100, BUFLEN, [0..0, 0..0]);
    assert_one(BUFLEN, BUFLEN, [0..0, 0..0]);

    // start=0
    assert_one(0, 100, [100..BUFLEN, 0..0]);
    assert_one(0, BUFLEN - 100, [BUFLEN - 100..BUFLEN, 0..0]);
    assert_one(0, BUFLEN - 1, [BUFLEN - 1..BUFLEN, 0..0]);

    // start=N
    assert_one(100, 100, [200..BUFLEN, 0..100]);
    assert_one(100, BUFLEN - 100, [0..100, 0..0]);
    assert_one(100, BUFLEN - 101, [BUFLEN - 1..BUFLEN, 0..100]);

    // start=BUFLEN-1
    assert_one(BUFLEN - 1, 100, [99..BUFLEN - 1, 0..0]);
}

#[test]
fn can_read_long_metainfo_correctly() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
        let reader = rqbit_runtime::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = reader.local_addr().unwrap().port();
        let mut writer = rqbit_runtime::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap()
            .into_split()
            .1;

        const ITERATIONS: u32 = 4096;

        let reader = async {
            let reader = reader.accept().await.unwrap().0.into_split().0;
            let mut reader: BoxAsyncReadVectored = Box::new(reader);
            let mut rb = ReadBuf::new();

            for piece in 0..ITERATIONS {
                let msg = rb
                    .read_message(&cx, &mut reader, Duration::from_millis(1000))
                    .await
                    .unwrap();
                let utdata = match msg {
                    Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(
                        utdata,
                    ))) => utdata,
                    other => panic!("expected utdata, got {other:?}"),
                };
                assert_eq!(utdata.len(), CHUNK_SIZE as usize);
                assert_eq!(utdata.piece(), piece);
                #[allow(clippy::cast_possible_truncation)]
                let expected_byte = { piece as u8 };
                let all_good = utdata
                    .as_double_buf()
                    .get()
                    .into_iter()
                    .flatten()
                    .all(|b| *b == expected_byte);
                if !all_good {
                    panic!("broken data");
                }
            }
        };

        let pext = PeerExtendedMessageIds::my();
        let writer = async {
            let mut sbuf = [0u8; MAX_MSG_LEN];
            let mut pbuf = [0u8; CHUNK_SIZE as usize];
            for piece in 0..ITERATIONS {
                #[allow(clippy::cast_possible_truncation)]
                for b in pbuf.iter_mut() {
                    *b = piece as u8;
                }
                let len = Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(
                    UtMetadataData::from_bytes(piece, CHUNK_SIZE * ITERATIONS, pbuf[..].into()),
                )))
                .serialize(&mut sbuf, &|| pext)
                .unwrap();
                writer.write_all(&sbuf[..len]).await.unwrap();
            }
        };

        futures::join!(reader, writer);
    });
    runtime.block_on(task).unwrap();
}

/// A test to prove that unsafe usage in read_buf is not UB.
#[test]
#[ignore = "run with --features=miri only"]
fn test_read_buf_miri() {
    struct BufWrap(std::vec::IntoIter<u8>);
    impl AsyncRead for BufWrap {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut rqbit_runtime::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            unimplemented!("don't need this")
        }
    }

    impl AsyncReadVectored for BufWrap {
        fn poll_read_vectored(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            vec: &mut [std::io::IoSliceMut<'_>],
        ) -> Poll<std::io::Result<usize>> {
            // Yield one byte at a time to ensure that all code paths are executed
            // (both NotEnoughData and Ok).
            let this = self.get_mut();
            let byte = match this.0.next() {
                Some(byte) => byte,
                None => return Poll::Ready(Ok(0)),
            };
            let target = vec.iter_mut().find(|s| !s.is_empty()).unwrap();
            target[0] = byte;
            Poll::Ready(Ok(1))
        }
    }

    let mut rb = ReadBuf::new();
    rb.start = BUFLEN - 15;

    let mut src = {
        let mut buf = vec![0u8; 64];
        let piece = Message::Piece(Piece::from_data(42, 43, b"hello"));
        let len = piece.serialize(&mut buf, &|| Default::default()).unwrap();
        buf.truncate(len);
        Box::new(BufWrap(buf.into_iter())) as BoxAsyncReadVectored
    };

    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    runtime.block_on(async {
        let msg = rb
            .read_message(&cx, &mut src, Duration::ZERO)
            .await
            .unwrap();
        match msg {
            Message::Piece(p) => {
                assert_eq!(p.index, 42);
                assert_eq!(p.begin, 43);
                assert_eq!(p.data(), (b"he".as_slice(), b"llo".as_slice()));
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            rb.read_message(&cx, &mut src, Duration::ZERO)
                .await
                .expect_err("expected error"),
            crate::Error::PeerDisconnected,
        ));
    })
}
