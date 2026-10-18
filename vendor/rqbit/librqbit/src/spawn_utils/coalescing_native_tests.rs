// Native coalescing cancellation tests; reuse the parent Fixture and bounds.
//
// This exercises actual coalescing code in a native timer/cancellation task.
// The socket adapter supplies deterministic bytes/Pending, so this does not
// prove real socket teardown, Live peer retirement or reconnect behavior. Its
// short real timer can wake the task too: this does not isolate cancellation
// wake latency, but cancellation must survive that wake and the checked join.

use super::*;
use crate::{
    read_buf::{OwnedPieceBatch, ReadBuf as PeerReadBuf},
    type_aliases::BoxAsyncReadVectored,
    vectored_traits::AsyncReadVectored,
};
use peer_binary_protocol::{Message, Piece};
use rqbit_runtime::io::AsyncRead;
use std::{pin::Pin, task::Context};

const CANCEL_MARKER: &str = "native coalescing cancelled after actual timeout Pending";

struct FirstFrameInput {
    wire: Vec<u8>,
    consumed: usize,
}

impl AsyncRead for FirstFrameInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
        output: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let count = (this.wire.len() - this.consumed).min(output.remaining());
        output.put_slice(&this.wire[this.consumed..this.consumed + count]);
        this.consumed += count;
        Poll::Ready(Ok(()))
    }
}

impl AsyncReadVectored for FirstFrameInput {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
        outputs: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut total = 0;
        for output in outputs {
            let count = (this.wire.len() - this.consumed).min(output.len());
            output[..count].copy_from_slice(&this.wire[this.consumed..this.consumed + count]);
            this.consumed += count;
            total += count;
        }
        Poll::Ready(Ok(total))
    }
}

struct PendingInput {
    vectored_polls: Arc<AtomicUsize>,
}

impl AsyncRead for PendingInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
        _output: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncReadVectored for PendingInput {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
        _outputs: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        self.vectored_polls.fetch_add(1, Ordering::AcqRel);
        // Deliberately no socket wake: a real native cancellation or timer wake
        // must settle the owned task after the OUTER timeout returned Pending.
        Poll::Pending
    }
}

fn prepared_native_pending_batch(fixture: &Fixture) -> (OwnedPieceBatch, usize, Vec<u8>) {
    let payload: Vec<u8> = (0..16_384).map(|index| (index % 251) as u8).collect();
    let mut wire = vec![0; 13 + payload.len()];
    let written = Message::Piece(Piece::from_data(42, 0, &payload))
        .serialize(&mut wire, &Default::default)
        .unwrap();
    assert_eq!(written, wire.len());
    let mut input: BoxAsyncReadVectored = Box::new(FirstFrameInput { wire, consumed: 0 });
    let (batch, pointer) = fixture.block_on(async {
        let mut receive = PeerReadBuf::new();
        receive.grow_for_piece_batch();
        let (index, begin, length, first_length, pointer) = {
            let Message::Piece(piece) = receive
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap()
            else {
                panic!("the native cancellation fixture must begin with an actual parsed Piece");
            };
            assert!(piece.data().1.is_empty());
            (
                piece.index,
                piece.begin,
                piece.len(),
                piece.data().0.len(),
                piece.data().0.as_ptr() as usize,
            )
        };
        let batch = receive
            .into_piece_frame(index, begin, length, first_length)
            .unwrap()
            .into_batch();
        assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
        (batch, pointer)
    });
    (batch, pointer, payload)
}

struct PendingCancellationObservation {
    actual_outer_pending: bool,
    input_vectored_polls: usize,
    cursor: usize,
    remaining: usize,
    first_payload_pointer: usize,
    first_payload: Vec<u8>,
    helper_error: Option<String>,
    helper_is_typed_cancelled: bool,
    caller_reason: Option<String>,
    ambient_matches_original_task: bool,
    cached_owner_eligible_before_intake: bool,
}

impl std::fmt::Debug for PendingCancellationObservation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("PendingCancellationObservation")
            .field("actual_outer_pending", &self.actual_outer_pending)
            .field("input_vectored_polls", &self.input_vectored_polls)
            .field("cursor", &self.cursor)
            .field("remaining", &self.remaining)
            .field("first_payload_pointer", &self.first_payload_pointer)
            .field("first_payload_len", &self.first_payload.len())
            .field("helper_error", &self.helper_error)
            .field("helper_is_typed_cancelled", &self.helper_is_typed_cancelled)
            .field("caller_reason", &self.caller_reason)
            .field(
                "ambient_matches_original_task",
                &self.ambient_matches_original_task,
            )
            .field(
                "cached_owner_eligible_before_intake",
                &self.cached_owner_eligible_before_intake,
            )
            .finish()
    }
}

#[test]
fn native_coalescing_original_cancel_after_actual_pending_settles_and_preserves_first_frame() {
    let fixture = Fixture::new();
    let (mut batch, original_pointer, expected_payload) = prepared_native_pending_batch(&fixture);
    let spawner = BlockingSpawner::new(1);
    let (observed_tx, observed_rx) = mpsc::channel();
    let task = rqbit_runtime::task::spawn_with(&fixture.cx, move |original| async move {
        let lease = spawner.lease(&original).await?;
        lease.run(&original, || ()).await?; // Same warmed eligibility as Live.
        let eligible = lease.group.get().unwrap().accepts(&original);
        let ambient_matches = Cx::current().is_some_and(|cx| cx.task_id() == original.task_id());
        let key = original.reusable_region_owner_key().unwrap();
        let poll_count = Arc::new(AtomicUsize::new(0));
        let mut input: BoxAsyncReadVectored = Box::new(PendingInput {
            vectored_polls: poll_count.clone(),
        });
        let mut intake = Box::pin(batch.intake_coalescing(&original, &key, &mut input));
        let mut pending_seen = false;
        let result = poll_fn(|task_context| {
            match intake.as_mut().poll(task_context) {
                Poll::Pending => {
                    if !pending_seen {
                        // Only an actual Pending from the complete production
                        // method is a witness. Its timeout has already polled
                        // and registered before this original-Cx cancellation.
                        pending_seen = true;
                        original
                            .set_cancel_reason(rqbit_runtime::CancelReason::user(CANCEL_MARKER));
                    }
                    Poll::Pending
                }
                Poll::Ready(result) => Poll::Ready(result),
            }
        })
        .await;
        drop(intake);
        let first_piece = batch.as_piece(0);
        observed_tx
            .send(PendingCancellationObservation {
                actual_outer_pending: pending_seen,
                input_vectored_polls: poll_count.load(Ordering::Acquire),
                cursor: batch.cursor(),
                remaining: batch.remaining_len(),
                first_payload_pointer: first_piece.data().0.as_ptr() as usize,
                first_payload: [first_piece.data().0, first_piece.data().1].concat(),
                helper_error: result.as_ref().err().map(ToString::to_string),
                helper_is_typed_cancelled: result
                    .as_ref()
                    .err()
                    .and_then(|error| error.downcast_ref::<rqbit_runtime::AsupError>())
                    .is_some_and(rqbit_runtime::AsupError::is_cancelled),
                caller_reason: original.cancel_reason().map(|reason| reason.to_string()),
                ambient_matches_original_task: ambient_matches,
                cached_owner_eligible_before_intake: eligible,
            })
            .unwrap();
        drop(lease);
        // Preserve the actual helper cancellation result. Do not replace it
        // with a synthetic successful receipt that could hide cancellation.
        result
    });
    let joined = fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, task));
    let observed = observed_rx.recv_timeout(BOUND);
    fixture.finish(); // Real region close, zero obligations, native worker join.

    let observed =
        observed.expect("the actual native coalescing method must settle and report its state");
    assert!(
        observed.actual_outer_pending,
        "no cancellation coverage if the 1.5 ms window expired before the first Pending: {observed:?}"
    );
    assert!(
        observed.input_vectored_polls >= 1,
        "the Pending socket adapter was actually polled"
    );
    assert!(observed.ambient_matches_original_task);
    assert!(observed.cached_owner_eligible_before_intake);
    assert_eq!((observed.cursor, observed.remaining), (0, 1));
    assert_eq!(observed.first_payload_pointer, original_pointer);
    assert!(
        observed.first_payload == expected_payload,
        "exact first payload bytes differ: {observed:?}"
    );
    assert!(
        observed.helper_is_typed_cancelled,
        "actual cancellation must not become ordinary coalescing expiry: {observed:?}"
    );
    assert_eq!(
        observed.caller_reason,
        Some(rqbit_runtime::CancelReason::user(CANCEL_MARKER).to_string()),
        "the original caller retains its exact cancellation reason"
    );
    assert!(
        matches!(joined, Ok(Ok(Err(ref error))) if error
            .downcast_ref::<rqbit_runtime::AsupError>()
            .is_some_and(rqbit_runtime::AsupError::is_cancelled)),
        "the original-Cx cancellation result must reach the checked native join: {joined:?}"
    );
}

// The parser's first-key/no-poll, Ready control/EOF/no-wait, partial-prefix
// timeout and absolute-deadline/no-reset cases belong to the pipeline agent's
// stage3 read_buf test artifact. They are not duplicated or modeled here.
// This strict native case requires witnessing real Pending; it never invents
// that state if an unusually delayed first poll already passed the 1.5 ms bound.
