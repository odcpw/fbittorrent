use super::super::{BATCH_CAPACITY, OwnedPieceBatch, SINGLE_PIECE_PASSIVE_BATCHES};
use super::*;
use rqbit_runtime::{Budget, Cx, types::Time};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const HAVE: &[u8] = &[0, 0, 0, 5, 4, 0, 0, 0, 99];

async fn parsed_batch(
    mut receive: ReadBuf,
    cx: &Cx,
    payloads: &[&[u8]],
    tail: &[u8],
) -> OwnedPieceBatch {
    assert!(!payloads.is_empty());
    assert_eq!((receive.len, receive.retained_wire_len), (0, 0));
    let mut wire = Vec::new();
    for payload in payloads {
        wire.extend_from_slice(&piece_wire(payload));
    }
    wire.extend_from_slice(tail);
    let mut source = input(wire, usize::MAX);
    let metadata = {
        let Message::Piece(piece) = receive
            .read_message(cx, &mut source, Duration::from_secs(1))
            .await
            .unwrap()
        else {
            panic!("parsed fixture must begin with Piece");
        };
        assert_eq!([piece.data().0, piece.data().1].concat(), payloads[0]);
        (piece.index, piece.begin, piece.len(), piece.data().0.len())
    };
    receive
        .into_piece_frame(metadata.0, metadata.1, metadata.2, metadata.3)
        .unwrap()
        .into_batch()
}

fn return_completed(mut batch: OwnedPieceBatch, allocation: *const u8) -> ReadBuf {
    while !batch.is_complete() {
        batch.complete_current();
    }
    let outcome = batch.into_receive_outcome();
    assert!(outcome.deferred_read_error.is_none());
    assert_eq!(outcome.read_buf.buf.as_ptr(), allocation);
    assert_eq!(outcome.read_buf.retained_wire_len, 0);
    outcome.read_buf
}

#[test]
fn piece_adaptive_coalescing_parsed_high_low_high_low_and_periodic_probe() {
    // These are real parsed frames and native timer waits through the
    // production selector, not a policy model or rate benchmark. The
    // deterministic input supplies buffered bursts and real Pending.
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .enable_time()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        assert_eq!(SINGLE_PIECE_PASSIVE_BATCHES, 8);
        let mut receive = ReadBuf::new();
        receive.grow_for_piece_batch();
        let allocation = receive.buf.as_ptr();
        let mut high =
            parsed_batch(receive, &cx, &[b"one", b"two", b"three", b"four"], &[]).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        high.intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert_eq!(
            (
                high.remaining_len(),
                high.read_buf.coalescing_passive_batches_left
            ),
            (4, 0)
        );
        assert_eq!(polls.load(Ordering::Relaxed), 0);
        receive = return_completed(high, allocation);

        let mut low = parsed_batch(receive, &cx, &[b"slow"], &[]).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        let before = cx.now();
        low.intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert!(cx.now() >= before + super::super::PIECE_COALESCING_WINDOW);
        assert!(polls.load(Ordering::Relaxed) >= 1);
        assert_eq!(
            (
                low.remaining_len(),
                low.read_buf.coalescing_passive_batches_left
            ),
            (1, 8)
        );
        receive = return_completed(low, allocation);

        for left in (0..SINGLE_PIECE_PASSIVE_BATCHES).rev() {
            let mut low = parsed_batch(receive, &cx, &[b"slow"], &[]).await;
            let (mut pending, polls) = probe(ProbeEnd::Pending);
            {
                let mut intake =
                    std::pin::pin!(low.intake_coalescing(&cx, &key, &mut pending));
                assert!(
                    matches!(
                        std::future::poll_fn(|task| Poll::Ready(
                            intake.as_mut().poll(task)
                        ))
                        .await,
                        Poll::Ready(Ok(()))
                    ),
                    "a passive single-frame batch must settle on its first poll"
                );
            }
            assert_eq!(polls.load(Ordering::Relaxed), 1);
            assert_eq!(
                (
                    low.remaining_len(),
                    low.read_buf.coalescing_passive_batches_left
                ),
                (1, left)
            );
            receive = return_completed(low, allocation);
        }

        // Countdown exhaustion must allow a timed probe even if no
        // passive burst was ever observed; otherwise a newly faster
        // paced peer could be trapped in the measured passive shape.
        let mut probe_batch = parsed_batch(receive, &cx, &[b"probe"], &[]).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        let before = cx.now();
        probe_batch
            .intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert!(cx.now() >= before + super::super::PIECE_COALESCING_WINDOW);
        assert!(polls.load(Ordering::Relaxed) >= 1);
        assert_eq!(probe_batch.read_buf.coalescing_passive_batches_left, 8);
        receive = return_completed(probe_batch, allocation);

        // A Ready two-frame burst recovers credit for the NEXT batch,
        // while preserving an unconsumed control and the allocation.
        let mut recovery =
            parsed_batch(receive, &cx, &[b"fast one", b"fast two"], HAVE).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        recovery
            .intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert_eq!(
            (
                recovery.remaining_len(),
                recovery.read_buf.coalescing_passive_batches_left
            ),
            (2, 0)
        );
        assert_eq!(polls.load(Ordering::Relaxed), 0);
        receive = return_completed(recovery, allocation);
        assert!(matches!(
            receive
                .read_message(&cx, &mut pending, Duration::from_secs(1))
                .await
                .unwrap(),
            Message::Have(99)
        ));
        assert_eq!(receive.buf.as_ptr(), allocation);

        let mut high =
            parsed_batch(receive, &cx, &[b"one", b"two", b"three", b"four"], &[]).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        high.intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert_eq!(
            (
                high.remaining_len(),
                high.read_buf.coalescing_passive_batches_left
            ),
            (4, 0)
        );
        assert_eq!(polls.load(Ordering::Relaxed), 0);
        receive = return_completed(high, allocation);
        let mut low = parsed_batch(receive, &cx, &[b"slow again"], &[]).await;
        let (mut pending, _) = probe(ProbeEnd::Pending);
        low.intake_coalescing(&cx, &key, &mut pending)
            .await
            .unwrap();
        assert_eq!(
            (
                low.remaining_len(),
                low.read_buf.coalescing_passive_batches_left
            ),
            (1, 8)
        );
        receive = return_completed(low, allocation);
        assert_eq!(receive.buf.len(), BATCH_CAPACITY);
    });
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[derive(Debug, Default)]
struct CountedClock {
    calls: AtomicUsize,
}
impl rqbit_runtime::time::TimeSource for CountedClock {
    fn now(&self) -> Time {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Time::ZERO
    }
}

struct CountedReads {
    inner: FragmentedInput,
    polls: Arc<AtomicUsize>,
}
impl AsyncRead for CountedReads {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        task: &mut std::task::Context<'_>,
        output: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::Relaxed);
        AsyncRead::poll_read(std::pin::Pin::new(&mut this.inner), task, output)
    }
}
impl AsyncReadVectored for CountedReads {
    fn poll_read_vectored(
        self: std::pin::Pin<&mut Self>,
        task: &mut std::task::Context<'_>,
        output: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::Relaxed);
        AsyncReadVectored::poll_read_vectored(
            std::pin::Pin::new(&mut this.inner),
            task,
            output,
        )
    }
}

#[test]
fn piece_adaptive_coalescing_passive_has_only_checkpoint_clock_no_sleep_and_keeps_read_bound()
 {
    // A counted, fixed clock makes a wrongly polled 1.5ms Sleep stay
    // Pending. It isolates extra selector clock reads/parking; it is
    // not evidence about native scheduling or production wait latency.
    let clock = Arc::new(CountedClock::default());
    let driver = rqbit_runtime::time::TimerDriverHandle::new(Arc::new(
        rqbit_runtime::time::TimerDriver::with_clock(clock.clone()),
    ));
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .with_timer_driver(driver.clone())
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        let mut receive = ReadBuf::new();
        receive.grow_for_piece_batch();
        receive.coalescing_passive_batches_left = 8;
        let allocation = receive.buf.as_ptr();
        let mut batch = parsed_batch(receive, &cx, &[b"first"], &[]).await;
        let (mut pending, polls) = probe(ProbeEnd::Pending);
        {
            let mut intake = std::pin::pin!(batch.intake_coalescing(&cx, &key, &mut pending));
            std::future::poll_fn(|task| {
                let before = clock.calls.load(Ordering::Relaxed);
                assert_eq!(driver.pending_count(), 0);
                assert!(matches!(intake.as_mut().poll(task), Poll::Ready(Ok(()))));
                assert_eq!(clock.calls.load(Ordering::Relaxed) - before, 1,
                    "retain the required caller checkpoint clock; add no entry deadline clock");
                assert_eq!(driver.pending_count(), 0, "passive intake must not poll a Sleep");
                Poll::Ready(())
            }).await;
        }
        assert_eq!(polls.load(Ordering::Relaxed), 1);
        receive = return_completed(batch, allocation);

        receive.start = BATCH_CAPACITY - 8;
        let payload = vec![0x5a; CHUNK_SIZE as usize];
        let mut batch = parsed_batch(receive, &cx, &[&payload], &[]).await;
        let first_identity = (batch.as_piece(0).data().0.as_ptr(), batch.as_piece(0).data().1.as_ptr());
        let mut suffix = piece_wire(b"second");
        suffix.extend_from_slice(HAVE);
        let polls = Arc::new(AtomicUsize::new(0));
        let mut source: BoxAsyncReadVectored = Box::new(CountedReads {
            inner: FragmentedInput { bytes: suffix, consumed: 0, max_read: 1 }, polls: polls.clone()
        });
        batch.intake_coalescing(&cx, &key, &mut source).await.unwrap();
        assert_eq!(polls.load(Ordering::Relaxed), 4, "same four successful Ready-read bound");
        assert_eq!(batch.remaining_len(), 1);
        assert_eq!(batch.read_buf.len, 4, "partial suffix stays buffered after the read bound");
        assert_eq!(batch.read_buf.retained_wire_len, payload.len() + 13);
        assert_eq!(batch.read_buf.buf.as_ptr(), allocation);
        assert_eq!((batch.as_piece(0).data().0.as_ptr(), batch.as_piece(0).data().1.as_ptr()), first_identity);
        assert_eq!([batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(), payload);
        receive = return_completed(batch, allocation);
        let Message::Piece(second) = receive.read_message(&cx, &mut source, Duration::from_secs(1)).await.unwrap()
        else { panic!("partial suffix must complete as the next Piece"); };
        assert_eq!([second.data().0, second.data().1].concat(), b"second");
        assert!(matches!(receive.read_message(&cx, &mut source, Duration::from_secs(1)).await.unwrap(), Message::Have(99)));
        assert_eq!(receive.buf.as_ptr(), allocation);
        assert_eq!(driver.pending_count(), 0);
    });
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn piece_adaptive_coalescing_passive_preserves_control_eof_parse_fault_key_and_cancel() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .enable_time()
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        for (suffix, end, expected) in [
            (HAVE.to_vec(), ProbeEnd::Pending, "control"),
            (piece_wire(&[]), ProbeEnd::Pending, "parse"),
            (vec![], ProbeEnd::Eof, "eof"),
            (vec![], ProbeEnd::Fault, "fault"),
        ] {
            let mut receive = ReadBuf::new();
            receive.grow_for_piece_batch();
            receive.start = BATCH_CAPACITY - 15;
            receive.coalescing_passive_batches_left = 8;
            let allocation = receive.buf.as_ptr();
            let mut batch = parsed_batch(receive, &cx, &[b"first"], &suffix).await;
            let first_identity = (
                batch.as_piece(0).data().0.as_ptr(),
                batch.as_piece(0).data().1.as_ptr(),
            );
            let (mut source, polls) = probe(end);
            batch
                .intake_coalescing(&cx, &key, &mut source)
                .await
                .unwrap();
            assert_eq!(batch.remaining_len(), 1);
            assert_eq!(batch.read_buf.buf.as_ptr(), allocation);
            assert_eq!(
                [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
                b"first"
            );
            assert_eq!(
                (
                    batch.as_piece(0).data().0.as_ptr(),
                    batch.as_piece(0).data().1.as_ptr()
                ),
                first_identity
            );
            assert_eq!(
                polls.load(Ordering::Relaxed),
                usize::from(expected == "eof" || expected == "fault")
            );
            match (expected, &batch.deferred_read_error) {
                ("control", None) => {}
                (
                    "parse",
                    Some(crate::Error::Deserialize(
                        peer_binary_protocol::MessageDeserializeError::IncorrectMsgLen {
                            ..
                        },
                    )),
                ) => {}
                ("eof", Some(crate::Error::PeerDisconnected)) => {}
                ("fault", Some(crate::Error::Read(error))) => {
                    assert_eq!(error.to_string(), "coalescing input fault")
                }
                _ => panic!("expected retained first frame and deferred {expected}"),
            }
            batch.complete_current();
            let outcome = batch.into_receive_outcome();
            assert_eq!(outcome.read_buf.buf.as_ptr(), allocation);
            if expected == "control" {
                let mut returned = outcome.read_buf;
                assert!(matches!(
                    returned
                        .read_message(&cx, &mut source, Duration::from_secs(1))
                        .await
                        .unwrap(),
                    Message::Have(99)
                ));
                assert_eq!(returned.buf.as_ptr(), allocation);
            } else {
                assert!(
                    outcome.deferred_read_error.is_some(),
                    "terminal read fault survives prefix settlement"
                );
            }
        }
    });

    for cancellation in [false, true] {
        let origin = runtime.request_cx_with_budget(Budget::INFINITE);
        let key = origin.reusable_region_owner_key().unwrap();
        runtime.block_on(async {
            let mut receive = ReadBuf::new();
            receive.grow_for_piece_batch();
            receive.coalescing_passive_batches_left = 8;
            let mut batch = parsed_batch(receive, &origin, &[b"first"], &[]).await;
            let (mut source, polls) = probe(ProbeEnd::Pending);
            if cancellation {
                origin.set_cancel_reason(rqbit_runtime::CancelReason::user(
                    "passive original caller cancelled",
                ));
                let error = batch
                    .intake_coalescing(&origin, &key, &mut source)
                    .await
                    .unwrap_err();
                assert!(
                    error
                        .downcast_ref::<rqbit_runtime::AsupError>()
                        .is_some_and(rqbit_runtime::AsupError::is_cancelled)
                );
            } else {
                origin
                    .apply_child_capability_budget(
                        rqbit_runtime::types::CapabilityBudget::UNSPECIFIED
                            .with_io_bytes(65_536),
                        rqbit_runtime::types::CapabilityBudgetRequirements::NONE,
                    )
                    .unwrap();
                assert!(origin.checkpoint().is_ok());
                assert!(origin.reusable_region_owner_key().is_none());
                batch
                    .intake_coalescing(&origin, &key, &mut source)
                    .await
                    .unwrap();
            }
            assert_eq!(polls.load(Ordering::Relaxed), 0);
            assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
            assert!(batch.deferred_read_error.is_none());
            assert_eq!(
                [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
                b"first"
            );
        });
    }
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}
