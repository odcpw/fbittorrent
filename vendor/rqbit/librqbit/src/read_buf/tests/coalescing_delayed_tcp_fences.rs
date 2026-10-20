use super::super::*;
use librqbit_core::constants::CHUNK_SIZE;
use rqbit_runtime::{Budget, cx::ChildRegionSpec, io::AsyncWriteExt};

const TRANSITION_WINDOW: Duration = Duration::from_millis(50);
const BOUND: Duration = Duration::from_secs(2);
const HAVE: &[u8] = &[0, 0, 0, 5, 4, 0, 0, 0, 99];

#[derive(Clone, Copy, Debug)]
enum Fence {
    Have,
    Eof,
}

fn identity(piece: &Piece<ByteBuf<'_>>) -> [(Option<usize>, usize); 2] {
    let (first, second) = piece.data();
    [first, second].map(|bytes| {
        (
            (!bytes.is_empty()).then_some(bytes.as_ptr() as usize),
            bytes.len(),
        )
    })
}

fn run_case(fence: Fence) {
    let runtime = rqbit_runtime::RuntimeBuilder::new()
        .worker_threads(1)
        .blocking_threads(1, 2)
        .enable_time()
        .build()
        .unwrap();
    let cleanup = runtime.request_cx_with_budget(Budget::INFINITE);
    let region = runtime
        .block_on(cleanup.open_child_region(ChildRegionSpec::inherit()))
        .unwrap();
    let task = rqbit_runtime::task::spawn_with(region.cx(), move |cx| async move {
        // First case crosses the headers and has an empty first payload
        // slice; second case splits seven real payload bytes at the wrap.
        for start in [BATCH_CAPACITY - 8, BATCH_CAPACITY - PIECE_PREAMBLE_LEN - 7] {
            let listener =
                rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                    .await
                    .unwrap();
            let mut remote = Some(
                rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
                    .await
                    .unwrap(),
            );
            let stream = listener.accept().await.unwrap().0;
            let (read, write) = stream.into_split();
            drop(write);
            let mut input: BoxAsyncReadVectored = Box::new(read);
            let payload: Vec<u8> =
                (0..CHUNK_SIZE).map(|index| (index % 251) as u8).collect();
            let mut wire = vec![0; PIECE_PREAMBLE_LEN + payload.len()];
            let size = Message::Piece(Piece::from_data(42, 0, &payload))
                .serialize(&mut wire, &Default::default)
                .unwrap();
            assert_eq!(size, wire.len());
            remote.as_mut().unwrap().write_all(&wire).await.unwrap();
            let mut rb = ReadBuf::new();
            rb.grow_for_piece_batch();
            rb.start = start;
            let allocation = rb.buf.as_ptr() as usize;
            let (index, begin, length, first_length, pointers) = {
                let Message::Piece(piece) =
                    rb.read_message(&cx, &mut input, BOUND).await.unwrap()
                else {
                    panic!("the TCP fixture must begin with an actual parsed Piece");
                };
                assert_eq!([piece.data().0, piece.data().1].concat(), payload);
                (
                    piece.index,
                    piece.begin,
                    piece.len(),
                    piece.data().0.len(),
                    identity(&piece),
                )
            };
            if start == BATCH_CAPACITY - 8 {
                assert_eq!(first_length, 0);
            } else {
                assert_eq!(first_length, 7);
            }
            let mut batch = rb
                .into_piece_frame(index, begin, length, first_length)
                .unwrap()
                .into_batch();
            let original_start = batch.read_buf.start;
            assert_eq!(batch.read_buf.len, 0);
            let key = cx
                .reusable_region_owner_key()
                .expect("native unrestricted test caller");
            let cutoff = cx.now() + TRANSITION_WINDOW;
            let mut intake = Box::pin(batch.intake_until(&cx, &key, &mut input, cutoff));
            let first =
                std::future::poll_fn(|task| Poll::Ready(intake.as_mut().poll(task))).await;
            assert!(
                first.is_pending(),
                "the complete method must actually park before delivering {fence:?}"
            );
            match fence {
                Fence::Have => remote.as_mut().unwrap().write_all(HAVE).await.unwrap(),
                Fence::Eof => drop(remote.take()),
            }
            let settled = rqbit_runtime::time::timeout(&cx, BOUND, intake.as_mut()).await;
            drop(intake);
            assert!(
                matches!(settled, Ok(Ok(()))),
                "actual delayed {fence:?} intake: {settled:?}"
            );
            assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
            assert_eq!(batch.read_buf.buf.as_ptr() as usize, allocation);
            assert_eq!(
                batch.read_buf.start, original_start,
                "control/EOF must not consume the retained first frame or advance the tail"
            );
            assert_eq!(batch.read_buf.retained_wire_len, wire.len());
            assert_eq!(identity(&batch.as_piece(0)), pointers);
            assert_eq!(
                [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
                payload
            );
            match fence {
                Fence::Have => {
                    // This proves Have was read DURING the waiting branch;
                    // a deadline return followed by a later normal read
                    // cannot satisfy the exact preserved tail assertion.
                    assert!(batch.deferred_read_error.is_none());
                    assert_eq!(batch.read_buf.len, HAVE.len());
                    let (first, second) = batch.read_buf.frame_slices();
                    assert_eq!([first, second].concat(), HAVE);
                }
                Fence::Eof => {
                    assert_eq!(batch.read_buf.len, 0);
                    assert!(
                        matches!(batch.deferred_read_error, Some(Error::PeerDisconnected)),
                        "EOF is a deferred transport error, not a claimed successful storage write"
                    );
                }
            }
            // Transport fixture only: completing metadata here does not
            // claim Live storage, checksum, mark or Have emission coverage.
            batch.complete_current();
            let outcome = batch.into_receive_outcome();
            assert_eq!(outcome.read_buf.retained_wire_len, 0);
            assert_eq!(outcome.read_buf.buf.as_ptr() as usize, allocation);
            match fence {
                Fence::Have => {
                    assert!(outcome.deferred_read_error.is_none());
                    let mut returned = outcome.read_buf;
                    assert!(matches!(
                        returned.read_message(&cx, &mut input, BOUND).await.unwrap(),
                        Message::Have(99)
                    ));
                    assert_eq!(
                        returned.len, 0,
                        "the negative control-tail assertion is exact"
                    );
                }
                Fence::Eof => assert!(matches!(
                    outcome.deferred_read_error,
                    Some(Error::PeerDisconnected)
                )),
            }
            drop(remote);
            drop(input);
            drop(listener);
        }
    });
    let joined = runtime.block_on(rqbit_runtime::time::timeout(&cleanup, BOUND, task));
    let closed = runtime.block_on(rqbit_runtime::time::timeout(
        &cleanup,
        BOUND,
        region.close(),
    ));
    let obligations = runtime.diagnostics().find_leaked_obligations().len();
    let shutdown = runtime.shutdown_timeout(BOUND);
    assert!(
        matches!(joined, Ok(Ok(()))),
        "actual native TCP task: {joined:?}"
    );
    assert!(
        matches!(closed, Ok(Ok(()))),
        "actual native region close: {closed:?}"
    );
    assert_eq!(obligations, 0);
    assert!(shutdown, "native scheduler and pool must join within 2 s");
}

#[test]
fn piece_coalescing_actual_pending_then_have_preserves_control_tail_and_wrapped_prefix() {
    run_case(Fence::Have);
}

#[test]
fn piece_coalescing_actual_pending_then_eof_defers_transport_error_and_preserves_wrapped_prefix()
 {
    run_case(Fence::Eof);
}
