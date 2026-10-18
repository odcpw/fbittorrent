// Native ownership tests. The parent retains every original assertion
// and its 2 s / 50 ms bounds.
//
// Coverage: actual protocol parsing, actual OwnedPieceFrame ownership, the
// production StorageLease::run dispatch and a real native file write. ByteInput
// supplies deterministic wire bytes; these are not Live PeerHandler tests.
// Original 19 storage tests must still be selected and counted by their exact
// original names separately from these new tests.

use super::*;
use crate::{
    read_buf::{OwnedPieceBatch, OwnedPieceFrame, ReadBuf as PeerReadBuf, process_piece_batch},
    type_aliases::BoxAsyncReadVectored,
    vectored_traits::AsyncReadVectored,
};
use peer_binary_protocol::{Message, Piece};
use rqbit_runtime::io::AsyncRead;
use std::{io::Write, pin::Pin, task::Context};

const HELD_FRAME_OBSERVATION: Duration = Duration::from_millis(50);
const FILE_PREFIX: &[u8] = b"begun:";
const TAIL_PAYLOAD: &[u8] = b"the same allocation also retains this tail";

// Empty slices have no meaningful pointer identity: the production constructor
// is free to represent them with another valid empty slice.
type SliceIdentity = [(Option<usize>, usize); 2];

fn slice_identity(piece: &Piece<buffers::ByteBuf<'_>>) -> SliceIdentity {
    let (first, second) = piece.data();
    [first, second].map(|bytes| {
        (
            (!bytes.is_empty()).then_some(bytes.as_ptr() as usize),
            bytes.len(),
        )
    })
}

struct ByteInput {
    wire: Vec<u8>,
    consumed: usize,
}

impl AsyncRead for ByteInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        target: &mut rqbit_runtime::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let count = (this.wire.len() - this.consumed).min(target.remaining());
        target.put_slice(&this.wire[this.consumed..this.consumed + count]);
        this.consumed += count;
        Poll::Ready(Ok(()))
    }
}

impl AsyncReadVectored for ByteInput {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        targets: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let mut copied = 0;
        for target in targets {
            let count = (this.wire.len() - this.consumed).min(target.len());
            target[..count].copy_from_slice(&this.wire[this.consumed..this.consumed + count]);
            this.consumed += count;
            copied += count;
            if this.consumed == this.wire.len() {
                break;
            }
        }
        Poll::Ready(Ok(copied))
    }
}

fn wire_piece(index: u32, begin: u32, payload: &[u8]) -> Vec<u8> {
    let mut wire = vec![0; 13 + payload.len()];
    let serialized = Message::Piece(Piece::from_data(index, begin, payload))
        .serialize(&mut wire, &Default::default)
        .unwrap();
    assert_eq!(serialized, wire.len());
    wire
}

fn parsed_owned_frame(
    fixture: &Fixture,
    payload: &[u8],
) -> (OwnedPieceFrame, BoxAsyncReadVectored, SliceIdentity) {
    let mut wire = wire_piece(42, 43, payload);
    wire.extend_from_slice(&[0, 0, 0, 5, 4, 0, 0, 0, 99]); // Have(99).
    wire.extend_from_slice(&wire_piece(44, 45, TAIL_PAYLOAD));
    wire.extend_from_slice(&[0, 0, 0, 0]); // KeepAlive.
    let mut input: BoxAsyncReadVectored = Box::new(ByteInput { wire, consumed: 0 });
    let (frame, pointers) = fixture.block_on(async {
        let mut receive = PeerReadBuf::new();
        let (index, begin, payload_len, first_len, pointers) = {
            let Message::Piece(piece) = receive
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap()
            else {
                panic!("expected an actual parsed Piece frame");
            };
            (
                piece.index,
                piece.begin,
                piece.len(),
                piece.data().0.len(),
                slice_identity(&piece),
            )
        };
        let frame = receive
            .into_piece_frame(index, begin, payload_len, first_len)
            .unwrap();
        assert_eq!(slice_identity(&frame.as_piece()), pointers);
        (frame, pointers)
    });
    (frame, input, pointers)
}

#[test]
fn native_owned_frame_returns_same_payload_allocation_and_unread_tail_after_real_write() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let payload: Vec<u8> = (0..16_384).map(|index| (index % 251) as u8).collect();
    let (frame, mut input, parsed_pointers) = parsed_owned_frame(&fixture, &payload);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("returned-owned-frame");
    let physical_path = path.clone();
    let caller = fixture.cx.task_id();
    let completed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        lease.run(&fixture.cx, move || {
            let owner = Cx::current().expect("the native physical owner is live");
            let piece = frame.as_piece();
            let pointers = slice_identity(&piece);
            let mut file = std::fs::File::create(physical_path)?;
            file.write_all(FILE_PREFIX)?;
            file.write_all(piece.data().0)?;
            file.write_all(piece.data().1)?;
            file.sync_data()?;
            Ok::<_, anyhow::Error>((frame.into_read_buf(), pointers, owner.task_id()))
        }),
    ));
    let (mut returned, physical_pointers, owner) = completed
        .expect("real physical frame write completes within the original bound")
        .expect("native storage dispatch succeeds")
        .expect("real physical frame write succeeds");

    let (tail_metadata, tail_pointers, tail_bytes, last_is_keepalive) = fixture.block_on(async {
        assert!(matches!(
            returned
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap(),
            Message::Have(99)
        ));
        let (metadata, pointers, bytes) = {
            let Message::Piece(piece) = returned
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap()
            else {
                panic!("the returned buffer must retain the actual following Piece");
            };
            (
                (piece.index, piece.begin),
                slice_identity(&piece),
                [piece.data().0, piece.data().1].concat(),
            )
        };
        let keepalive = matches!(
            returned
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap(),
            Message::KeepAlive
        );
        (metadata, pointers, bytes, keepalive)
    });
    let bytes = std::fs::read(path).unwrap();
    drop(returned);
    drop(lease);
    fixture.finish();

    assert_eq!(
        physical_pointers, parsed_pointers,
        "the physical callback borrows the parsed allocation"
    );
    assert_ne!(owner, caller, "the callback uses the live storage owner");
    assert_eq!(tail_metadata, (44, 45));
    assert_eq!(tail_bytes, TAIL_PAYLOAD);
    assert!(last_is_keepalive);
    assert_eq!(
        tail_pointers[0],
        (
            Some(parsed_pointers[0].0.unwrap() + payload.len() + 9 + 13),
            TAIL_PAYLOAD.len()
        ),
        "the retained tail is still at its original offset in the same allocation"
    );
    assert_eq!(bytes, [FILE_PREFIX, payload.as_slice()].concat());
}

#[test]
fn native_owned_frame_waiter_drop_and_region_close_keep_real_payload_capture_until_write_finishes()
{
    let mut fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    let payload: Vec<u8> = (0..16_384).map(|index| (index % 251) as u8).collect();
    let (frame, input, parsed_pointers) = parsed_owned_frame(&fixture, &payload);
    // The physical callback must own the frame; its synthetic input adapter is
    // no longer needed. In particular, no borrowed Piece crosses the dispatch.
    drop(input);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("held-owned-frame");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let caller = fixture.cx.task_id();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&fixture.cx, move || {
        let owner = Cx::current().expect("the native physical owner is live");
        let before = slice_identity(&frame.as_piece());
        let mut file = std::fs::File::create(physical_path)?;
        file.write_all(FILE_PREFIX)?;
        file.flush()?;
        entered_tx.send((before, owner.task_id())).unwrap();
        gate.wait();
        // Read from the actual captured production frame after the waiter and
        // lease have departed and region.close has been held for 50 ms.
        let piece = frame.as_piece();
        let after = slice_identity(&piece);
        file.write_all(piece.data().0)?;
        file.write_all(piece.data().1)?;
        file.sync_data()?;
        finished_tx.send(after).unwrap();
        Ok::<PeerReadBuf, anyhow::Error>(frame.into_read_buf())
    }));
    let physical_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    let entered = entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&path).unwrap();
    let shared = lease.group.get().unwrap().shared.clone();
    drop(physical); // Actual run waiter drop, not cancellation of a model future.
    drop(lease); // Natural sender EOF must not discard the accepted frame.
    let permit_while_held = spawner.semaphore().available_permits();
    let owners_while_held = shared.alive.load(Ordering::Acquire);
    let physical_still_held = matches!(finished_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    let region = fixture.region.take().unwrap();
    let mut close = Box::pin(region.close());
    let held_close = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        HELD_FRAME_OBSERVATION,
        close.as_mut(),
    ));
    let close_pending = held_close.is_err();
    // Collect all negative evidence before explicit release. Even a failed
    // assertion later cannot strand this gate or the native pool thread.
    fixture.gate.release();
    let finished = finished_rx.recv_timeout(BOUND);
    let closed = if close_pending {
        fixture.block_on(rqbit_runtime::time::timeout(&fixture.cleanup, BOUND, close))
    } else {
        held_close
    };
    let full_bytes = std::fs::read(path);
    let restored = spawner.semaphore().available_permits();
    let owners_after = shared.alive.load(Ordering::Acquire);
    fixture.finish();

    assert!(
        physical_pending,
        "run was actually polled while the write stayed held"
    );
    assert_eq!(
        prefix, FILE_PREFIX,
        "a real file mutation precedes the hold"
    );
    assert_eq!(entered.0, parsed_pointers);
    assert_ne!(
        entered.1, caller,
        "the callback uses the live storage owner"
    );
    assert_eq!(permit_while_held, 0);
    assert_eq!(owners_while_held, 1);
    assert!(physical_still_held);
    assert!(
        close_pending,
        "region close stays pending for the original 50 ms interval"
    );
    assert_eq!(
        finished.expect("released real write completes within the original 2 s bound"),
        parsed_pointers
    );
    assert!(matches!(closed, Ok(Ok(()))), "region close: {closed:?}");
    assert_eq!(
        full_bytes.unwrap(),
        [FILE_PREFIX, payload.as_slice()].concat()
    );
    assert_eq!(restored, 1);
    assert_eq!(owners_after, 0);
}

fn warmed_batch_lease(
    fixture: &Fixture,
    spawner: &BlockingSpawner,
) -> crate::spawn_utils::StorageLease {
    let lease = fixture.block_on(spawner.lease(&fixture.cx)).unwrap();
    fixture.block_on(lease.run(&fixture.cx, || ())).unwrap();
    assert!(lease.group.get().unwrap().accepts(&fixture.cx));
    lease
}

fn parsed_owned_batch(
    fixture: &Fixture,
) -> (
    OwnedPieceBatch,
    BoxAsyncReadVectored,
    [Vec<u8>; 2],
    [SliceIdentity; 2],
) {
    let first: Vec<u8> = (0..16_384).map(|index| (index % 251) as u8).collect();
    let second: Vec<u8> = (0..16_384).map(|index| 255 - (index % 251) as u8).collect();
    let mut wire = wire_piece(42, 0, &first);
    wire.extend_from_slice(&wire_piece(43, 16_384, &second));
    wire.extend_from_slice(&[0, 0, 0, 5, 4, 0, 0, 0, 99]);
    wire.extend_from_slice(&[0, 0, 0, 0]);
    let mut input: BoxAsyncReadVectored = Box::new(ByteInput { wire, consumed: 0 });
    let (batch, pointers) = fixture.block_on(async {
        let mut receive = PeerReadBuf::new();
        receive.grow_for_piece_batch(); // Before parsing or borrowing any frame.
        let (index, begin, payload_len, first_len, first_pointers) = {
            let Message::Piece(piece) = receive
                .read_message(&fixture.cx, &mut input, BOUND)
                .await
                .unwrap()
            else {
                panic!("expected the first actual Piece in the batch");
            };
            (
                piece.index,
                piece.begin,
                piece.len(),
                piece.data().0.len(),
                slice_identity(&piece),
            )
        };
        let mut batch = receive
            .into_piece_frame(index, begin, payload_len, first_len)
            .unwrap()
            .into_batch();
        let key = fixture.cx.reusable_region_owner_key().unwrap();
        batch
            .intake_ready(&fixture.cx, &key, &mut input)
            .await
            .unwrap();
        assert_eq!((batch.cursor(), batch.remaining_len()), (0, 2));
        let pointers = [
            slice_identity(&batch.as_piece(0)),
            slice_identity(&batch.as_piece(1)),
        ];
        assert_eq!(pointers[0], first_pointers);
        assert_eq!((batch.as_piece(0).index, batch.as_piece(1).index), (42, 43));
        (batch, pointers)
    });
    (batch, input, [first, second], pointers)
}

fn assert_completed_control_tail(
    fixture: &Fixture,
    batch: OwnedPieceBatch,
    input: &mut BoxAsyncReadVectored,
) {
    assert_eq!(
        (batch.cursor(), batch.remaining_len(), batch.is_complete()),
        (2, 0, true)
    );
    let outcome = batch.into_receive_outcome();
    assert!(outcome.deferred_read_error.is_none());
    let mut returned = outcome.read_buf;
    fixture.block_on(async {
        assert!(matches!(
            returned
                .read_message(&fixture.cx, input, BOUND)
                .await
                .unwrap(),
            Message::Have(99)
        ));
        assert!(matches!(
            returned
                .read_message(&fixture.cx, input, BOUND)
                .await
                .unwrap(),
            Message::KeepAlive
        ));
    });
}

#[test]
fn native_owned_batch_serial_helper_keeps_second_frame_after_held_first_write() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = warmed_batch_lease(&fixture, &spawner);
    let (batch, mut input, payloads, parsed_pointers) = parsed_owned_batch(&fixture);
    let origin = fixture.cx.clone();
    let physical_origin = origin.clone();
    let key = origin.reusable_region_owner_key().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("serial-batch");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let physical_events = events.clone();
    let pointers = Arc::new(Mutex::new(Vec::new()));
    let physical_pointers = pointers.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (second_tx, second_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&origin, move || {
        let mut file = std::fs::File::create(physical_path)?;
        process_piece_batch(&physical_origin, Some(&key), batch, 4, |piece, cursor| {
            physical_events.lock().unwrap().push(("enter", cursor));
            physical_pointers
                .lock()
                .unwrap()
                .push(slice_identity(&piece));
            if cursor == 0 {
                file.write_all(FILE_PREFIX)?;
                file.flush()?;
                entered_tx.send(()).unwrap();
                gate.wait();
            } else {
                second_tx.send(()).unwrap();
            }
            file.write_all(piece.data().0)?;
            file.write_all(piece.data().1)?;
            file.sync_data()?;
            physical_events.lock().unwrap().push(("finish", cursor));
            Ok(())
        })
    }));
    let was_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&path).unwrap();
    let events_while_held = events.lock().unwrap().clone();
    let second_while_held = second_rx.recv_timeout(HELD_FRAME_OBSERVATION);
    let permit_while_held = spawner.semaphore().available_permits();
    fixture.gate.release();
    let completed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        physical,
    ));
    let batch = completed
        .expect("native batch joins within 2 s")
        .unwrap()
        .unwrap();
    let bytes = std::fs::read(path).unwrap();
    let actual_events = events.lock().unwrap().clone();
    let actual_pointers = pointers.lock().unwrap().clone();
    assert_completed_control_tail(&fixture, batch, &mut input);
    drop(lease);
    fixture.finish();

    assert!(was_pending);
    assert_eq!(prefix, FILE_PREFIX);
    assert_eq!(events_while_held, [("enter", 0)]);
    assert!(matches!(
        second_while_held,
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(permit_while_held, 0);
    assert_eq!(
        actual_events,
        [("enter", 0), ("finish", 0), ("enter", 1), ("finish", 1)]
    );
    assert_eq!(actual_pointers, parsed_pointers);
    assert_eq!(
        bytes,
        [FILE_PREFIX, payloads[0].as_slice(), payloads[1].as_slice()].concat()
    );
}

#[test]
fn native_owned_batch_caller_cancel_between_items_drains_first_write_and_never_enters_second() {
    let mut fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = warmed_batch_lease(&fixture, &spawner);
    let (batch, input, payloads, parsed_pointers) = parsed_owned_batch(&fixture);
    drop(input);
    let origin = fixture.cx.clone();
    let physical_origin = origin.clone();
    let key = origin.reusable_region_owner_key().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("cancel-first");
    let second_path = directory.path().join("cancel-second-must-not-exist");
    let physical_first = first_path.clone();
    let physical_second = second_path.clone();
    let gate = fixture.gate.clone();
    let entered_cursors = Arc::new(Mutex::new(Vec::new()));
    let physical_cursors = entered_cursors.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&origin, move || {
        process_piece_batch(&physical_origin, Some(&key), batch, 4, |piece, cursor| {
            physical_cursors.lock().unwrap().push(cursor);
            if cursor == 0 {
                let mut file = std::fs::File::create(&physical_first)?;
                file.write_all(FILE_PREFIX)?;
                file.flush()?;
                entered_tx.send(slice_identity(&piece)).unwrap();
                gate.wait();
                file.write_all(piece.data().0)?;
                file.write_all(piece.data().1)?;
                file.sync_data()?;
                finished_tx.send(slice_identity(&piece)).unwrap();
            } else {
                std::fs::write(&physical_second, [piece.data().0, piece.data().1].concat())?;
            }
            Ok(())
        })
    }));
    let was_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    let entered = entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&first_path).unwrap();
    origin.set_cancel_reason(rqbit_runtime::CancelReason::user(
        "batch caller stopped between items",
    ));
    let region = fixture.region.take().unwrap();
    let mut closing = Box::pin(region.close());
    let held_close = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        HELD_FRAME_OBSERVATION,
        closing.as_mut(),
    ));
    let close_pending = held_close.is_err();
    let cursors_while_held = entered_cursors.lock().unwrap().clone();
    let permit_while_held = spawner.semaphore().available_permits();
    let physical_still_held = matches!(finished_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    fixture.gate.release();
    let finished = finished_rx.recv_timeout(BOUND);
    let settled = fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            physical,
        ))
        .expect("cancelled native batch settles within 2 s");
    let failure = match settled {
        Err(error) | Ok(Err(error)) => error.to_string(),
        Ok(Ok(_)) => panic!("the cancelled original caller cannot report a successful batch"),
    };
    let closed = if close_pending {
        fixture.block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            closing,
        ))
    } else {
        held_close
    };
    let bytes = std::fs::read(first_path);
    let second_exists = second_path.exists();
    let actual_cursors = entered_cursors.lock().unwrap().clone();
    let restored = spawner.semaphore().available_permits();
    drop(lease);
    fixture.finish();

    assert!(was_pending);
    assert_eq!(prefix, FILE_PREFIX);
    assert_eq!(entered, parsed_pointers[0]);
    assert_eq!(cursors_while_held, [0]);
    assert_eq!(permit_while_held, 0);
    assert!(physical_still_held);
    assert!(
        close_pending,
        "actual region close must stay pending for 50 ms"
    );
    assert_eq!(
        finished.expect("first real write physically finishes within 2 s"),
        parsed_pointers[0]
    );
    assert!(
        failure.contains("batch caller stopped between items"),
        "actual cancellation: {failure}"
    );
    assert!(matches!(closed, Ok(Ok(()))), "actual close: {closed:?}");
    assert_eq!(actual_cursors, [0]);
    assert!(
        !second_exists,
        "the later frame never creates its physical output"
    );
    assert_eq!(
        bytes.unwrap(),
        [FILE_PREFIX, payloads[0].as_slice()].concat()
    );
    assert_eq!(restored, 1);
}

#[test]
fn native_owned_batch_key_fence_returns_same_suffix_and_reselects_original_caller_one_frame_fallback()
 {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = warmed_batch_lease(&fixture, &spawner);
    let (batch, mut input, payloads, parsed_pointers) = parsed_owned_batch(&fixture);
    let origin = fixture.cx.clone();
    let physical_origin = origin.clone();
    let key = origin.reusable_region_owner_key().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("key-fence-batch");
    let physical_path = path.clone();
    let gate = fixture.gate.clone();
    let entered_cursors = Arc::new(Mutex::new(Vec::new()));
    let physical_cursors = entered_cursors.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&origin, move || {
        let mut file = std::fs::File::create(physical_path)?;
        process_piece_batch(&physical_origin, Some(&key), batch, 4, |piece, cursor| {
            physical_cursors.lock().unwrap().push(cursor);
            if cursor == 0 {
                file.write_all(FILE_PREFIX)?;
                file.flush()?;
                entered_tx
                    .send((Cx::current().unwrap().task_id(), slice_identity(&piece)))
                    .unwrap();
                gate.wait();
            }
            file.write_all(piece.data().0)?;
            file.write_all(piece.data().1)?;
            file.sync_data()?;
            Ok(())
        })
    }));
    let was_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    let entered = entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&path).unwrap();
    let tightened = origin
        .apply_child_capability_budget(
            rqbit_runtime::types::CapabilityBudget::UNSPECIFIED.with_io_bytes(65_536),
            rqbit_runtime::types::CapabilityBudgetRequirements::NONE,
        )
        .unwrap();
    let caller_checkpoint_ok = origin.checkpoint().is_ok();
    let caller_now_ineligible = origin.reusable_region_owner_key().is_none();
    fixture.gate.release();
    let remainder = fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            physical,
        ))
        .expect("old physical batch returns its fence within 2 s")
        .unwrap()
        .unwrap();
    let cursor_at_fence = (
        remainder.cursor(),
        remainder.remaining_len(),
        remainder.is_complete(),
    );
    let suffix_pointers = slice_identity(&remainder.as_piece(1));
    let cursors_at_fence = entered_cursors.lock().unwrap().clone();
    let bytes_at_fence = std::fs::read(&path).unwrap();

    // Normal lease.run performs real selection using the SAME original caller.
    // The caller's new explicit budget excludes the warmed group, so this is
    // its existing one-operation owner path, never a fabricated root context.
    let fallback_origin = origin.clone();
    let fallback_path = path.clone();
    let fallback_cursors = entered_cursors.clone();
    let resumed = fixture.block_on(rqbit_runtime::time::timeout(
        &fixture.cleanup,
        BOUND,
        lease.run(&origin, move || {
            let ambient = Cx::current().unwrap();
            let fallback_owner = ambient.task_id();
            let fallback_budget = ambient.capability_budget();
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(fallback_path)?;
            let complete =
                process_piece_batch(&fallback_origin, None, remainder, 1, |piece, cursor| {
                    fallback_cursors.lock().unwrap().push(cursor);
                    let pointers = slice_identity(&piece);
                    if pointers != parsed_pointers[1] {
                        anyhow::bail!(
                            "fallback did not retain the original unprocessed payload slices"
                        );
                    }
                    file.write_all(piece.data().0)?;
                    file.write_all(piece.data().1)?;
                    file.sync_data()?;
                    Ok(())
                })?;
            Ok::<_, anyhow::Error>((complete, fallback_owner, fallback_budget))
        }),
    ));
    let (complete, fallback_owner, fallback_budget) = resumed
        .expect("one-frame fallback joins within 2 s")
        .unwrap()
        .unwrap();
    let bytes = std::fs::read(path).unwrap();
    let actual_cursors = entered_cursors.lock().unwrap().clone();
    assert_completed_control_tail(&fixture, complete, &mut input);
    drop(lease);
    fixture.finish();

    assert!(was_pending);
    assert_eq!(prefix, FILE_PREFIX);
    assert_eq!(entered.1, parsed_pointers[0]);
    assert!(
        caller_checkpoint_ok,
        "attenuation is a key fence, not caller cancellation"
    );
    assert!(caller_now_ineligible);
    assert_eq!(cursor_at_fence, (1, 1, false));
    assert_eq!(suffix_pointers, parsed_pointers[1]);
    assert_eq!(cursors_at_fence, [0]);
    assert_eq!(
        bytes_at_fence,
        [FILE_PREFIX, payloads[0].as_slice()].concat()
    );
    assert_ne!(
        fallback_owner, entered.0,
        "normal selection creates the restricted operation owner"
    );
    assert_eq!(
        fallback_budget, tightened,
        "fallback keeps the original caller's actual budget"
    );
    assert_eq!(actual_cursors, [0, 1], "no processed prefix is retried");
    assert_eq!(
        bytes,
        [FILE_PREFIX, payloads[0].as_slice(), payloads[1].as_slice()].concat()
    );
}

#[test]
fn native_owned_batch_consumer_error_after_real_prefix_never_invokes_later_frame() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = warmed_batch_lease(&fixture, &spawner);
    let (batch, input, payloads, _) = parsed_owned_batch(&fixture);
    drop(input);
    let origin = fixture.cx.clone();
    let physical_origin = origin.clone();
    let key = origin.reusable_region_owner_key().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("fatal-consumer-first");
    let later_path = directory.path().join("fatal-consumer-later-must-not-exist");
    let physical_first = first_path.clone();
    let physical_later = later_path.clone();
    let gate = fixture.gate.clone();
    let entered_cursors = Arc::new(Mutex::new(Vec::new()));
    let physical_cursors = entered_cursors.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let mut physical = Box::pin(lease.run(&origin, move || {
        process_piece_batch(&physical_origin, Some(&key), batch, 4, |piece, cursor| {
            physical_cursors.lock().unwrap().push(cursor);
            if cursor == 0 {
                let mut file = std::fs::File::create(&physical_first)?;
                file.write_all(FILE_PREFIX)?;
                file.flush()?;
                entered_tx.send(()).unwrap();
                gate.wait();
                file.write_all(piece.data().0)?;
                file.write_all(piece.data().1)?;
                file.sync_data()?;
                anyhow::bail!("batch consumer fatal write marker");
            }
            std::fs::write(&physical_later, [piece.data().0, piece.data().1].concat())?;
            Ok(())
        })
    }));
    let was_pending = fixture.block_on(poll_fn(|task| {
        Poll::Ready(physical.as_mut().poll(task).is_pending())
    }));
    entered_rx.recv_timeout(BOUND).unwrap();
    let prefix = std::fs::read(&first_path).unwrap();
    let cursors_while_held = entered_cursors.lock().unwrap().clone();
    fixture.gate.release();
    let settled = fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            physical,
        ))
        .expect("native first-error batch settles within 2 s")
        .expect("the live caller's outer dispatch succeeds");
    let failure = settled
        .err()
        .expect("the actual consumer error must stop the helper");
    let actual_cursors = entered_cursors.lock().unwrap().clone();
    let bytes = std::fs::read(first_path).unwrap();
    let later_exists = later_path.exists();
    drop(lease);
    fixture.finish();

    assert!(was_pending);
    assert_eq!(prefix, FILE_PREFIX);
    assert_eq!(cursors_while_held, [0]);
    assert_eq!(failure.to_string(), "batch consumer fatal write marker");
    assert_eq!(actual_cursors, [0]);
    assert!(
        !later_exists,
        "a later consumer cannot create a file after the first error"
    );
    assert_eq!(bytes, [FILE_PREFIX, payloads[0].as_slice()].concat());
}

#[test]
fn native_owned_batch_physical_panic_after_real_prefix_outranks_origin_cancellation() {
    let fixture = Fixture::new();
    let spawner = BlockingSpawner::new(1);
    let lease = warmed_batch_lease(&fixture, &spawner);
    let (batch, input, _, _) = parsed_owned_batch(&fixture);
    drop(input);
    let origin = fixture.cx.clone();
    let physical_origin = origin.clone();
    let key = origin.reusable_region_owner_key().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("panic-consumer-prefix");
    let physical_path = path.clone();
    let entered_cursors = Arc::new(Mutex::new(Vec::new()));
    let physical_cursors = entered_cursors.clone();
    let settled = fixture
        .block_on(rqbit_runtime::time::timeout(
            &fixture.cleanup,
            BOUND,
            lease.run(&origin, move || {
                process_piece_batch(&physical_origin, Some(&key), batch, 4, |_piece, cursor| {
                    physical_cursors.lock().unwrap().push(cursor);
                    let mut file = std::fs::File::create(&physical_path)?;
                    file.write_all(FILE_PREFIX)?;
                    file.flush()?;
                    physical_origin.set_cancel_reason(rqbit_runtime::CancelReason::user(
                        "batch simultaneous panic cancellation",
                    ));
                    panic!("batch physical panic marker");
                })
            }),
        ))
        .expect("the native physical panic settles within 2 s");
    let failure = settled
        .err()
        .expect("a physical panic must remain the outer typed storage failure");
    let prefix = std::fs::read(path).unwrap();
    let actual_cursors = entered_cursors.lock().unwrap().clone();
    let caller_cancelled = origin.checkpoint().is_err();
    drop(lease);
    fixture.finish();

    assert_eq!(prefix, FILE_PREFIX);
    assert_eq!(actual_cursors, [0]);
    assert!(caller_cancelled);
    assert_eq!(
        failure.to_string(),
        "blocking storage operation failed: task panicked: panic: batch physical panic marker"
    );
}

// These batch tests call the actual production serial helper, not a second
// algorithm. Their consumer performs real files and deliberate Err/panic;
// they do not exercise Live's file_ops, checksum or on_fatal_error channel.
// Those paths need separate preserved-body review and real interoperability.
// Pre-entry TypedJob authority rejection remains the original disposal policy;
// this API can return a still-owned suffix only at a between-item key fence.
