use super::super::*;
use crate::vectored_traits::{AsyncReadVectored, AsyncReadVectoredExt};
use librqbit_core::{constants::CHUNK_SIZE, lengths::Lengths};
use rqbit_runtime::io::{AsyncRead, AsyncWriteExt};
use std::sync::{Arc, Mutex};

const TCP_CASE_BOUND: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
enum IntakeMode {
    Passive,
    Coalescing,
}

async fn intake(
    mode: IntakeMode,
    batch: &mut OwnedPieceBatch,
    cx: &Cx,
    key: &rqbit_runtime::cx::cx::RegionOwnerKey,
    input: &mut BoxAsyncReadVectored,
) {
    match mode {
        IntakeMode::Passive => batch.intake_ready(cx, key, input).await.unwrap(),
        IntakeMode::Coalescing => batch.intake_coalescing(cx, key, input).await.unwrap(),
    }
}

#[derive(Default)]
struct ReadTrace {
    polls: usize,
    ready_sizes: Vec<usize>,
}

struct SegmentedTcpRead {
    inner: rqbit_runtime::net::OwnedReadHalf,
    max_read: usize,
    trace: Arc<Mutex<ReadTrace>>,
}

impl AsyncRead for SegmentedTcpRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        target: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let capacity = target.remaining().min(this.max_read);
        let mut limited =
            rqbit_runtime::io::ReadBuf::new(&mut target.unfilled()[..capacity]);
        let result = AsyncRead::poll_read(Pin::new(&mut this.inner), cx, &mut limited);
        let bytes = limited.filled().len();
        drop(limited);
        let mut trace = this.trace.lock().unwrap();
        trace.polls += 1;
        if matches!(result, Poll::Ready(Ok(()))) {
            target.advance(bytes);
            trace.ready_sizes.push(bytes);
        }
        result
    }
}

impl AsyncReadVectored for SegmentedTcpRead {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        targets: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut remaining = this.max_read;
        let mut limited = Vec::with_capacity(2);
        for target in targets {
            let size = target.len().min(remaining);
            if size != 0 {
                limited.push(IoSliceMut::new(&mut target[..size]));
                remaining -= size;
            }
            if remaining == 0 {
                break;
            }
        }
        let result = rqbit_runtime::io::AsyncReadVectored::poll_read_vectored(
            Pin::new(&mut this.inner),
            cx,
            &mut limited,
        );
        let mut trace = this.trace.lock().unwrap();
        trace.polls += 1;
        if let Poll::Ready(Ok(bytes)) = &result {
            trace.ready_sizes.push(*bytes);
        }
        result
    }
}

fn wire_piece(index: u32, payload: &[u8]) -> Vec<u8> {
    let mut wire = vec![0; payload.len() + PIECE_PREAMBLE_LEN];
    let count = Message::Piece(Piece::from_data(index, 0, payload))
        .serialize(&mut wire, &Default::default)
        .unwrap();
    assert_eq!(count, wire.len());
    wire
}

fn expected_frames(count: usize) -> Vec<(u32, u32, Vec<u8>)> {
    (0..count)
        .map(|index| {
            (
                index as u32,
                0,
                vec![0x31 + index as u8; CHUNK_SIZE as usize],
            )
        })
        .collect()
}

fn wire_with_control_tail(count: usize) -> Vec<u8> {
    let mut wire = Vec::new();
    for (index, _, payload) in expected_frames(count) {
        wire.extend(wire_piece(index, &payload));
    }
    // Have must fence intake before the following short Piece, even when
    // all of these bytes are already readable on the real socket.
    wire.extend_from_slice(&[0, 0, 0, 5, 4, 0, 0, 0, 99]);
    wire.extend(wire_piece(77, b"untouched control-fenced tail"));
    wire.extend_from_slice(&[0, 0, 0, 0]);
    wire
}

async fn tcp_input(
    wire: &[u8],
    max_read: usize,
) -> (BoxAsyncReadVectored, Arc<Mutex<ReadTrace>>) {
    let listener =
        rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
    let mut remote = rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let stream = listener.accept().await.unwrap().0;
    let (read, write) = stream.into_split();
    drop(write);
    // Small header fragments followed by larger blocks; real writes may
    // coalesce, while max_read enforces independently varied read polls.
    let widths = [1, 2, 3, 7, 13, 16_384];
    let mut sent = 0;
    for width in widths.into_iter().cycle() {
        if sent == wire.len() {
            break;
        }
        let next = (sent + width).min(wire.len());
        remote.write_all(&wire[sent..next]).await.unwrap();
        sent = next;
    }
    drop(remote); // Real EOF follows the finite fixture bytes.
    let trace = Arc::new(Mutex::new(ReadTrace::default()));
    (
        Box::new(SegmentedTcpRead {
            inner: read,
            max_read,
            trace: trace.clone(),
        }),
        trace,
    )
}

async fn preload_actual_tcp(
    cx: &Cx,
    rb: &mut ReadBuf,
    input: &mut BoxAsyncReadVectored,
    bytes: usize,
) {
    while rb.len < bytes {
        let count = input
            .read_vectored(cx, &mut rb.unfilled_ioslices_with_limit(bytes - rb.len))
            .await
            .unwrap();
        assert!(
            count > 0,
            "finite TCP fixture must provide its declared prefix"
        );
        rb.len += count;
    }
}

async fn drain_frames(
    cx: &Cx,
    mut rb: ReadBuf,
    input: &mut BoxAsyncReadVectored,
    trace: &Arc<Mutex<ReadTrace>>,
    count: usize,
    first_ready_depth: Option<usize>,
    mode: IntakeMode,
) -> Vec<(u32, u32, Vec<u8>)> {
    let key = cx
        .reusable_region_owner_key()
        .expect("native unbounded test task");
    let allocation = rb.buf.as_ptr() as usize;
    let mut result = Vec::new();
    let mut depths = Vec::new();
    while result.len() < count {
        let meta = {
            let Message::Piece(piece) =
                rb.read_message(cx, input, TCP_CASE_BOUND).await.unwrap()
            else {
                panic!("expected real TCP Piece before control fence");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        let mut batch = rb
            .into_piece_frame(meta.0, meta.1, meta.2, meta.3)
            .unwrap()
            .into_batch();
        let reads_before = trace.lock().unwrap().ready_sizes.len();
        intake(mode, &mut batch, cx, &key, input).await;
        // Coalescing may repoll after a real Pending readiness wakeup.
        // The shared bound counts successful ready reads, not wakeups.
        assert!(
            trace.lock().unwrap().ready_sizes.len() - reads_before
                <= MAX_READY_READ_ATTEMPTS
        );
        let depth = batch.remaining_len();
        assert!((1..=MAX_BATCH_FRAMES).contains(&depth));
        if depths.is_empty() {
            if let Some(expected) = first_ready_depth {
                assert_eq!(depth, expected, "preloaded real TCP ready-frame boundary");
            }
        }
        depths.push(depth);
        assert_eq!(batch.read_buf.buf.as_ptr() as usize, allocation);
        assert_eq!(
            batch.read_buf.retained_wire_len,
            depth * (CHUNK_SIZE as usize + PIECE_PREAMBLE_LEN)
        );
        assert!(batch.read_buf.len + batch.read_buf.retained_wire_len <= BATCH_CAPACITY);
        if depth == 4 {
            assert_eq!(batch.read_buf.retained_wire_len, 65_588);
            assert_eq!(
                batch
                    .read_buf
                    .unfilled_ioslices()
                    .iter()
                    .map(|slice| slice.len())
                    .sum::<usize>(),
                0
            );
        }
        while !batch.is_complete() {
            let piece = batch.as_piece(batch.cursor());
            result.push((
                piece.index,
                piece.begin,
                [piece.data().0, piece.data().1].concat(),
            ));
            batch.complete_current();
        }
        let outcome = batch.into_receive_outcome();
        assert!(outcome.deferred_read_error.is_none());
        rb = outcome.read_buf;
        assert_eq!(rb.buf.as_ptr() as usize, allocation);
        assert_eq!(rb.retained_wire_len, 0);
    }
    assert_eq!(result, expected_frames(count));
    assert!(matches!(
        rb.read_message(cx, input, TCP_CASE_BOUND).await.unwrap(),
        Message::Have(99)
    ));
    let meta = {
        let Message::Piece(piece) =
            rb.read_message(cx, input, TCP_CASE_BOUND).await.unwrap()
        else {
            panic!("control-fenced Piece tail must remain intact");
        };
        assert_eq!((piece.index, piece.begin), (77, 0));
        assert_eq!(
            [piece.data().0, piece.data().1].concat(),
            b"untouched control-fenced tail"
        );
        (piece.index, piece.begin, piece.len(), piece.data().0.len())
    };
    let mut tail = rb
        .into_piece_frame(meta.0, meta.1, meta.2, meta.3)
        .unwrap()
        .into_batch();
    intake(mode, &mut tail, cx, &key, input).await;
    assert_eq!(tail.remaining_len(), 1, "KeepAlive fences tail batching");
    tail.complete_current();
    let outcome = tail.into_receive_outcome();
    assert!(outcome.deferred_read_error.is_none());
    let mut rb = outcome.read_buf;
    assert!(matches!(
        rb.read_message(cx, input, TCP_CASE_BOUND).await.unwrap(),
        Message::KeepAlive
    ));
    assert!(matches!(
        rb.read_message(cx, input, TCP_CASE_BOUND).await,
        Err(Error::PeerDisconnected)
    ));
    println!(
        "R4 real TCP mode={mode:?}, count={count}, depths={depths:?}, exact ordered payload/control/EOF verified"
    );
    result
}

#[test]
fn piece_batch_actual_tcp_segmentation_and_ring_offsets_are_metamorphic() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        for mode in [IntakeMode::Passive, IntakeMode::Coalescing] {
            for count in [3, 4, 5] {
                for start in [0, BATCH_CAPACITY - 1, BATCH_CAPACITY - PIECE_PREAMBLE_LEN] {
                    for max_read in [1, 13, BATCH_CAPACITY] {
                        let wire = wire_with_control_tail(count);
                        let (mut input, trace) = tcp_input(&wire, max_read).await;
                        let mut rb = ReadBuf::new();
                        rb.grow_for_piece_batch();
                        rb.start = start;
                        assert_eq!(
                            drain_frames(&cx, rb, &mut input, &trace, count, None, mode)
                                .await,
                            expected_frames(count)
                        );
                        let trace = trace.lock().unwrap();
                        assert!(trace.ready_sizes.iter().all(|size| *size <= max_read));
                        assert_eq!(trace.ready_sizes.iter().sum::<usize>(), wire.len());
                        println!(
                            "R4 actual TCP start={start}, max_read={max_read}, polls={}",
                            trace.polls
                        );
                    }
                }
            }
        }
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(TCP_CASE_BOUND));
}

#[test]
fn piece_batch_actual_tcp_ready_three_four_five_frames_obey_stack_capacity() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        assert_eq!((MAX_BATCH_FRAMES, BATCH_CAPACITY), (4, 65_588));
        for mode in [IntakeMode::Passive, IntakeMode::Coalescing] {
            for count in [3, 4, 5] {
                let wire = wire_with_control_tail(count);
                let (mut input, trace) = tcp_input(&wire, 7).await;
                let mut rb = ReadBuf::new();
                rb.grow_for_piece_batch();
                rb.start = BATCH_CAPACITY - PIECE_PREAMBLE_LEN;
                preload_actual_tcp(
                    &cx,
                    &mut rb,
                    &mut input,
                    wire.len().min(BATCH_CAPACITY),
                )
                .await;
                // One fewer:3 stop at control. Exact:4 fill capacity. One more:
                // fifth remains on actual socket until the first batch returns.
                drain_frames(&cx, rb, &mut input, &trace, count, Some(count.min(4)), mode)
                    .await;
            }
        }
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(TCP_CASE_BOUND));
}

#[test]
fn piece_batch_actual_tcp_preserves_wire_limit_and_live_chunk_shape_boundary() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        let lengths = Lengths::new(65_536, 65_536).unwrap();
        let index = lengths.validate_piece_index(0).unwrap();
        assert!(
            lengths
                .chunk_info_from_received_data(index, 0, CHUNK_SIZE)
                .is_some()
        );
        for payload_len in [
            31 * 1024,
            BUFLEN - PIECE_PREAMBLE_LEN - 1,
            BUFLEN - PIECE_PREAMBLE_LEN,
            BUFLEN - PIECE_PREAMBLE_LEN + 1,
        ] {
            for start in [0, BATCH_CAPACITY - 7] {
                let payload = vec![0x5a; payload_len];
                let wire = wire_piece(0, &payload);
                let (mut input, trace) = tcp_input(&wire, 13).await;
                let mut rb = ReadBuf::new();
                rb.grow_for_piece_batch();
                rb.start = start;
                let allocation = rb.buf.as_ptr() as usize;
                if wire.len() <= BUFLEN {
                    let meta = {
                        let Message::Piece(piece) = rb
                            .read_message(&cx, &mut input, TCP_CASE_BOUND)
                            .await
                            .unwrap()
                        else {
                            panic!("wire-bounded Piece must preserve its metadata/payload");
                        };
                        assert_eq!([piece.data().0, piece.data().1].concat(), payload);
                        // This is the actual public shape validation called
                        // by Live::prepare_received_piece, not a fake Live.
                        assert!(
                            lengths
                                .chunk_info_from_received_data(
                                    index,
                                    piece.begin,
                                    piece.len() as u32
                                )
                                .is_none()
                        );
                        (piece.index, piece.begin, piece.len(), piece.data().0.len())
                    };
                    let frame =
                        rb.into_piece_frame(meta.0, meta.1, meta.2, meta.3).unwrap();
                    rb = frame.into_read_buf();
                    assert_eq!(rb.buf.as_ptr() as usize, allocation);
                    assert!(matches!(
                        rb.read_message(&cx, &mut input, TCP_CASE_BOUND).await,
                        Err(Error::PeerDisconnected)
                    ));
                } else {
                    assert!(matches!(
                        rb.read_message(&cx, &mut input, TCP_CASE_BOUND).await,
                        Err(Error::ReadBufFull {
                            need_additional_bytes: 1
                        })
                    ));
                    assert_eq!(rb.len, BUFLEN);
                    assert_eq!(rb.start, start);
                    assert_eq!(rb.buf.as_ptr() as usize, allocation);
                }
                assert!(
                    trace
                        .lock()
                        .unwrap()
                        .ready_sizes
                        .iter()
                        .all(|size| *size <= 13)
                );
                println!(
                    "R4 actual TCP wire_length={}, ring_start={start}, parser_bound={BUFLEN}",
                    wire.len()
                );
            }
        }
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(TCP_CASE_BOUND));
}
