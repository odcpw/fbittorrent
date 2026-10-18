use rqbit_runtime::{Cx, types::Time};
use std::{future::Future, io::IoSliceMut, ops::Range, pin::Pin, task::Poll, time::Duration};

use crate::{Error, Result};
use crate::{
    peer_connection::with_timeout, type_aliases::BoxAsyncReadVectored,
    vectored_traits::AsyncReadVectoredExt,
};
use buffers::ByteBuf;
use futures::TryFutureExt;
use peer_binary_protocol::{
    DoubleBufHelper, Handshake, Message, MessageDeserializeError, PIECE_MESSAGE_DEFAULT_LEN, Piece,
};
use rqbit_runtime::cx::cx::RegionOwnerKey;
use rqbit_runtime::io::AsyncReadExt;

// This should be greater than MAX_MSG_LEN
const BUFLEN: usize = 0x8000; // The initial allocation and maximum accepted wire frame.
const MAX_BATCH_FRAMES: usize = 4;
const MAX_READY_READ_ATTEMPTS: usize = 4;
const PIECE_COALESCING_WINDOW: Duration = Duration::from_micros(1_500);
const SINGLE_PIECE_PASSIVE_BATCHES: u8 = 8;
const BATCH_CAPACITY: usize = MAX_BATCH_FRAMES * PIECE_MESSAGE_DEFAULT_LEN;
const PIECE_PREAMBLE_LEN: usize = 13;

/// A ringbuffer for reading bittorrent messages from socket.
/// Messages may thus span 2 slices (notably, Piece message and UtMetadata messages), which is reflected in their contents.
pub struct ReadBuf {
    buf: Box<[u8]>,
    start: usize,
    len: usize,
    retained_wire_len: usize,
    coalescing_passive_batches_left: u8,
}

/// A parsed Piece and its existing receive allocation. The consumed payload
/// remains valid until this owner is returned or dropped; no socket read can
/// reuse its bytes while storage owns the frame.
pub struct OwnedPieceFrame {
    read_buf: ReadBuf,
    index: u32,
    begin: u32,
    payload: [Range<usize>; 2],
}

impl OwnedPieceFrame {
    pub(crate) fn as_piece(&self) -> Piece<ByteBuf<'_>> {
        Piece::from_data_slices(
            self.index,
            self.begin,
            &self.read_buf.buf[self.payload[0].clone()],
            &self.read_buf.buf[self.payload[1].clone()],
        )
    }

    pub(crate) fn into_read_buf(self) -> ReadBuf {
        self.read_buf
    }

    pub(crate) fn into_batch(self) -> OwnedPieceBatch {
        let wire_len = self
            .payload
            .iter()
            .map(|range| range.end - range.start)
            .sum::<usize>()
            + PIECE_PREAMBLE_LEN;
        let mut read_buf = self.read_buf;
        assert_eq!(read_buf.retained_wire_len, 0);
        assert!(wire_len <= read_buf.buf.len() - read_buf.len);
        read_buf.retained_wire_len = wire_len;
        OwnedPieceBatch {
            read_buf,
            frames: [
                Some(PieceFrameMetadata {
                    index: self.index,
                    begin: self.begin,
                    payload: self.payload,
                }),
                None,
                None,
                None,
            ],
            cursor: 0,
            count: 1,
            deferred_read_error: None,
        }
    }
}

/// A completed prefix and the terminal read error observed after that prefix.
/// The reader returns this error directly, preserving its transport category.
pub struct PieceReceiveOutcome {
    pub(crate) read_buf: ReadBuf,
    pub(crate) deferred_read_error: Option<Error>,
}

impl PieceReceiveOutcome {
    pub(crate) fn completed(read_buf: ReadBuf) -> Self {
        Self {
            read_buf,
            deferred_read_error: None,
        }
    }
}

struct PieceFrameMetadata {
    index: u32,
    begin: u32,
    payload: [Range<usize>; 2],
}

/// One receive allocation and at most four parsed Piece frames. The ring's
/// reservation includes each frame's headers as well as its payload, so intake
/// cannot reuse consumed bytes until the whole batch has settled.
pub struct OwnedPieceBatch {
    read_buf: ReadBuf,
    frames: [Option<PieceFrameMetadata>; MAX_BATCH_FRAMES],
    cursor: usize,
    count: usize,
    deferred_read_error: Option<Error>,
}

enum PieceIntake {
    Added,
    NeedMore,
    Fence,
}

#[derive(Debug, PartialEq, Eq)]
enum PieceIntakeEnd {
    Stopped,
    Deadline,
}

impl OwnedPieceBatch {
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }
    pub(crate) fn remaining_len(&self) -> usize {
        self.count - self.cursor
    }
    pub(crate) fn is_complete(&self) -> bool {
        self.cursor == self.count
    }

    pub(crate) fn as_piece(&self, index: usize) -> Piece<ByteBuf<'_>> {
        let frame = self.frames[index].as_ref().expect("parsed Piece metadata");
        Piece::from_data_slices(
            frame.index,
            frame.begin,
            &self.read_buf.buf[frame.payload[0].clone()],
            &self.read_buf.buf[frame.payload[1].clone()],
        )
    }

    pub(crate) fn complete_current(&mut self) {
        assert!(self.cursor < self.count);
        self.cursor += 1;
    }

    pub(crate) fn into_receive_outcome(mut self) -> PieceReceiveOutcome {
        assert!(self.is_complete());
        self.read_buf.retained_wire_len = 0;
        PieceReceiveOutcome {
            read_buf: self.read_buf,
            deferred_read_error: self.deferred_read_error,
        }
    }

    fn scan_next_piece(&mut self) -> PieceIntake {
        if self.count == MAX_BATCH_FRAMES || self.deferred_read_error.is_some() {
            return PieceIntake::Fence;
        }
        let parsed = {
            let (first, second) = self.read_buf.frame_slices();
            let mut header = DoubleBufHelper::new(first, second);
            let Ok(prefix) = header.read_u32_be() else {
                return PieceIntake::NeedMore;
            };
            if prefix == 0 {
                return PieceIntake::Fence;
            }
            match header.read_u8() {
                Some(7) => {}
                Some(_) => return PieceIntake::Fence,
                None => return PieceIntake::NeedMore,
            }
            match Message::deserialize(first, second) {
                Ok((Message::Piece(piece), wire_len)) => Ok((
                    piece.index,
                    piece.begin,
                    piece.len(),
                    piece.data().0.len(),
                    wire_len,
                )),
                Ok(_) => return PieceIntake::Fence,
                Err(error) => Err(error),
            }
        };
        match parsed {
            Ok((index, begin, payload_len, first_len, wire_len)) => {
                self.read_buf.advance(wire_len);
                let payload = match self.read_buf.piece_payload_ranges(payload_len, first_len) {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.deferred_read_error = Some(error);
                        return PieceIntake::Fence;
                    }
                };
                self.read_buf.retained_wire_len += wire_len;
                assert!(
                    self.read_buf.retained_wire_len + self.read_buf.len <= self.read_buf.buf.len()
                );
                self.frames[self.count] = Some(PieceFrameMetadata {
                    index,
                    begin,
                    payload,
                });
                self.count += 1;
                #[cfg(feature = "chunk-diagnostics")]
                crate::chunk_diagnostics::received_piece_frame();
                PieceIntake::Added
            }
            Err(MessageDeserializeError::NotEnoughData(missing, ..)) => {
                if self.read_buf.len >= BUFLEN {
                    self.deferred_read_error = Some(ReadBuf::frame_full_error(missing));
                    PieceIntake::Fence
                } else {
                    PieceIntake::NeedMore
                }
            }
            Err(error) => {
                self.deferred_read_error = Some(Error::Deserialize(error));
                PieceIntake::Fence
            }
        }
    }

    fn poll_intake(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
        task_cx: &mut std::task::Context<'_>,
        ready_reads: &mut usize,
        deadline: Option<Time>,
    ) -> Poll<anyhow::Result<PieceIntakeEnd>> {
        loop {
            cx.checkpoint()?;
            if cx.reusable_region_owner_key().as_ref() != Some(expected_key) {
                return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
            }
            match self.scan_next_piece() {
                PieceIntake::Added => continue,
                PieceIntake::Fence => return Poll::Ready(Ok(PieceIntakeEnd::Stopped)),
                PieceIntake::NeedMore => {}
            }
            if *ready_reads == MAX_READY_READ_ATTEMPTS || self.read_buf.is_full() {
                return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
            }
            if deadline.is_some_and(|deadline| cx.now() >= deadline) {
                return Poll::Ready(Ok(PieceIntakeEnd::Deadline));
            }
            match Pin::new(&mut **input)
                .poll_read_vectored(task_cx, &mut self.read_buf.unfilled_ioslices())
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(0)) => {
                    self.deferred_read_error = Some(Error::PeerDisconnected);
                    return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
                }
                Poll::Ready(Ok(bytes)) => {
                    *ready_reads += 1;
                    self.read_buf.len += bytes;
                }
                Poll::Ready(Err(error)) => {
                    self.deferred_read_error = Some(Error::Read(error));
                    return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
                }
            }
        }
    }

    /// Poll only already-ready socket data. A pending socket future never
    /// delays the prepared frame or creates a coalescing timer.
    pub(crate) async fn intake_ready(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
    ) -> anyhow::Result<()> {
        let mut ready_reads = 0;
        match std::future::poll_fn(|task_cx| {
            Poll::Ready(self.poll_intake(cx, expected_key, input, task_cx, &mut ready_reads, None))
        })
        .await
        {
            Poll::Pending => Ok(()),
            Poll::Ready(result) => result.map(|_| ()),
        }
    }

    /// After a single-frame attempt, use a bounded number of passive batches
    /// before probing again. A passive multi-frame batch enables waiting for
    /// the next batch. Passive attempts add no entry-clock read or timeout.
    /// Timed attempts use one unchanged absolute coalescing window; already
    /// ready work remains immediate, and scheduler delay can overshoot it.
    pub(crate) async fn intake_coalescing(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
    ) -> anyhow::Result<()> {
        if self.read_buf.coalescing_passive_batches_left != 0 {
            self.read_buf.coalescing_passive_batches_left -= 1;
            self.intake_ready(cx, expected_key, input).await?;
            if self.count >= 2 {
                self.read_buf.coalescing_passive_batches_left = 0;
            }
            return Ok(());
        }
        let deadline = cx.now() + PIECE_COALESCING_WINDOW;
        self.intake_until(cx, expected_key, input, deadline).await?;
        if self.count == 1 {
            self.read_buf.coalescing_passive_batches_left = SINGLE_PIECE_PASSIVE_BATCHES;
        }
        Ok(())
    }

    async fn intake_until(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
        deadline: Time,
    ) -> anyhow::Result<()> {
        #[cfg(feature = "chunk-diagnostics")]
        let mut observation = crate::chunk_diagnostics::CoalescingObservation::new(
            Duration::from_nanos(deadline.duration_since(cx.now())),
        );
        let mut ready_reads = 0;
        let initial = std::future::poll_fn(|task_cx| {
            Poll::Ready(self.poll_intake(
                cx,
                expected_key,
                input,
                task_cx,
                &mut ready_reads,
                Some(deadline),
            ))
        })
        .await;
        if let Poll::Ready(result) = initial {
            #[cfg(feature = "chunk-diagnostics")]
            if matches!(&result, Ok(PieceIntakeEnd::Deadline)) {
                observation.deadline_stop();
            }
            return result.map(|_| ());
        }
        let remaining = Duration::from_nanos(deadline.duration_since(cx.now()));
        if remaining.is_zero() {
            cx.checkpoint()?;
            #[cfg(feature = "chunk-diagnostics")]
            observation.deadline_stop();
            return Ok(());
        }
        // One timeout wraps the entire remaining intake, rather than one
        // timeout per read or per Piece. The poll helper also checks the same
        // absolute deadline before reading after a timer/scheduler wakeup.
        let mut timeout = std::pin::pin!(rqbit_runtime::time::timeout_at(
            deadline,
            std::future::poll_fn(|task_cx| {
                self.poll_intake(
                    cx,
                    expected_key,
                    input,
                    task_cx,
                    &mut ready_reads,
                    Some(deadline),
                )
            }),
        ));
        let waited = std::future::poll_fn(|task_cx| {
            let progress = timeout.as_mut().poll(task_cx);
            #[cfg(feature = "chunk-diagnostics")]
            if progress.is_pending() {
                observation.pending();
            }
            progress
        })
        .await;
        // Timeout/sleep cancellation must remain an original-caller error;
        // explicit cancellation never turns into a successful partial batch.
        cx.checkpoint()?;
        match waited {
            Ok(result) => {
                #[cfg(feature = "chunk-diagnostics")]
                if matches!(&result, Ok(PieceIntakeEnd::Deadline)) {
                    observation.deadline_stop();
                }
                result.map(|_| ())
            }
            Err(_) => {
                #[cfg(feature = "chunk-diagnostics")]
                observation.deadline_stop();
                Ok(())
            }
        }
    }
}

/// The physical serial processor used by Live and native ownership tests.
/// A key fence is a successful owned remainder, not a fabricated peer error.
/// The existing storage wrapper still handles pre-entry rejection and panics.
pub(crate) fn process_piece_batch<F>(
    origin: &Cx,
    expected_key: Option<&RegionOwnerKey>,
    mut batch: OwnedPieceBatch,
    max_frames: usize,
    mut process: F,
) -> anyhow::Result<OwnedPieceBatch>
where
    F: for<'a> FnMut(Piece<ByteBuf<'a>>, usize) -> anyhow::Result<()>,
{
    let physical_owner =
        Cx::current().ok_or_else(|| anyhow::anyhow!("storage batch has no physical owner"))?;
    assert!((1..=MAX_BATCH_FRAMES).contains(&max_frames));
    assert!(max_frames == 1 || expected_key.is_some());
    for _ in 0..max_frames {
        if batch.is_complete() {
            break;
        }
        origin.checkpoint()?;
        if let Some(key) = expected_key {
            if origin.reusable_region_owner_key().as_ref() != Some(key) {
                return Ok(batch);
            }
        }
        physical_owner.checkpoint()?;
        #[cfg(feature = "chunk-diagnostics")]
        crate::chunk_diagnostics::physical_piece_entry();
        process(batch.as_piece(batch.cursor), batch.cursor)?;
        batch.complete_current();
    }
    Ok(batch)
}

/// Advance by N bytes. A macro so that existing field-level
/// borrows are understood by the borrow checker.
macro_rules! advance {
    ($self:expr, $len:expr) => {
        $self.len = $self.len.saturating_sub($len);
        $self.start = ($self.start + $len) % $self.buf.len();
    };
}

/// Convert readbuf into 2 slices. A macro so that field-level
/// borrows are understood by the borrow checker.
macro_rules! as_slices {
    ($self:expr) => {{
        let (first, second) = $self.as_slice_ranges();
        (&$self.buf[first], &$self.buf[second])
    }};
}

impl ReadBuf {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0u8; BUFLEN]),
            start: 0,
            len: 0,
            retained_wire_len: 0,
            coalescing_passive_batches_left: 0,
        }
    }

    /// Move the allocation after a successful Piece parse. `read_message` has
    /// already advanced past the frame, whose final bytes are its payload.
    /// Keep the parser's original split, including an empty first slice when
    /// the Piece headers crossed the ring boundary.
    pub(crate) fn into_piece_frame(
        self,
        index: u32,
        begin: u32,
        payload_len: usize,
        first_len: usize,
    ) -> Result<OwnedPieceFrame> {
        let payload = self.piece_payload_ranges(payload_len, first_len)?;
        Ok(OwnedPieceFrame {
            read_buf: self,
            index,
            begin,
            payload,
        })
    }

    fn piece_payload_ranges(
        &self,
        payload_len: usize,
        first_len: usize,
    ) -> Result<[Range<usize>; 2]> {
        let capacity = self.buf.len();
        if self.start >= capacity
            || payload_len > capacity
            || first_len > payload_len
            || self.len > capacity - payload_len
        {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "invalid received piece buffer metadata"
            )));
        }
        let payload_start = (self.start + capacity - payload_len) % capacity;
        let second_start = (payload_start + first_len) % capacity;
        let second_len = payload_len - first_len;
        if first_len > capacity - payload_start || second_len > capacity - second_start {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "received piece payload exceeds its buffer slice"
            )));
        }
        Ok([
            payload_start..payload_start + first_len,
            second_start..second_start + second_len,
        ])
    }

    /// Called at the reader loop before parsing or retaining any Piece. Grow
    /// once after the peer's first successful storage operation warmed its lease.
    pub(crate) fn grow_for_piece_batch(&mut self) {
        assert_eq!(self.retained_wire_len, 0);
        if self.buf.len() >= BATCH_CAPACITY {
            return;
        }
        let mut bigger = vec![0; BATCH_CAPACITY].into_boxed_slice();
        let (first, second) = as_slices!(self);
        bigger[..first.len()].copy_from_slice(first);
        bigger[first.len()..first.len() + second.len()].copy_from_slice(second);
        self.buf = bigger;
        self.start = 0;
    }

    fn frame_full_error(missing: usize) -> Error {
        // Preserve the original bounded frame error's u16 payload.
        #[allow(clippy::cast_possible_truncation)]
        let need_additional_bytes = missing as u16;
        Error::ReadBufFull {
            need_additional_bytes,
        }
    }

    // Read the BT handshake.
    // This MUST be run as the first operation on the buffer.
    pub async fn read_handshake(
        &mut self,
        cx: &Cx,
        conn: &mut BoxAsyncReadVectored,
        timeout: Duration,
    ) -> Result<Handshake> {
        // TCP may split the fixed 68-byte BEP 3 handshake across reads.
        // Read exactly the handshake so later protocol bytes remain on the socket.
        with_timeout(cx, "reading handshake", timeout, async {
            cx.checkpoint()?;
            conn.read_exact(&mut self.buf[..68])
                .await
                .map_err(Error::ReadHandshake)?;
            self.len = 68;
            let (handshake, size) =
                Handshake::deserialize(&self.buf[..68]).map_err(Error::DeserializeHandshake)?;
            self.advance(size);
            Ok(handshake)
        })
        .await
    }

    fn is_contiguous(&self) -> bool {
        self.start + self.len == (self.start + self.len) % self.buf.len()
    }

    // In rare cases, we might need to make the buffer contiguous, as the message
    // parsing code won't work with a split ringbuffer. Only "bitfield" and "extended" messages
    // need contiguous buffers.
    // "Bitfield" however is always sent early, so in practice it won't ever happen. For "extended"
    // in practice, it would rarely happen with UtMetadata::Data messages if the split happens to land
    // in the middle of bencoded data (as we only can deserialize bencode from a single slice).
    fn make_contiguous(&mut self) -> Result<()> {
        assert_eq!(self.retained_wire_len, 0);
        if self.is_contiguous() {
            let (Ok(start), Ok(len)) = (u16::try_from(self.start), u16::try_from(self.len)) else {
                return Err(Error::Anyhow(anyhow::anyhow!(
                    "make_contiguous called for contiguous buffer start={} len={}",
                    self.start,
                    self.len
                )));
            };
            return Err(Error::BugReadBufMakeContiguous { start, len });
        }
        self.buf.rotate_left(self.start);
        self.start = 0;
        Ok(())
    }

    /// Convert into 2 slices (as ranges).
    fn as_slice_ranges(&self) -> (Range<usize>, Range<usize>) {
        self.slice_ranges_with_limit(self.buf.len())
    }

    fn slice_ranges_with_limit(&self, limit: usize) -> (Range<usize>, Range<usize>) {
        let capacity = self.buf.len();
        let len = self.len.min(capacity).min(limit);
        let start = self.start.min(capacity);
        let first_len = len.min(capacity - start);
        (start..start + first_len, 0..len.saturating_sub(first_len))
    }

    fn frame_slices(&self) -> (&[u8], &[u8]) {
        let (first, second) = self.slice_ranges_with_limit(BUFLEN);
        (&self.buf[first], &self.buf[second])
    }

    fn advance(&mut self, len: usize) {
        advance!(self, len);
    }

    fn is_full(&self) -> bool {
        self.len + self.retained_wire_len == self.buf.len()
    }

    /// Get a part of the buffer for reading into.
    fn unfilled_ioslices(&mut self) -> [IoSliceMut<'_>; 2] {
        self.unfilled_ioslices_with_limit(self.buf.len())
    }

    fn unfilled_ioslices_with_limit(&mut self, limit: usize) -> [IoSliceMut<'_>; 2] {
        let capacity = self.buf.len();
        let write_start = (self.start.saturating_add(self.len)) % capacity;
        let available_len = capacity
            .saturating_sub(self.len)
            .saturating_sub(self.retained_wire_len)
            .min(limit);
        let first_len = (capacity.saturating_sub(write_start)).min(available_len);
        let second_len = available_len.saturating_sub(first_len);

        let (second, first) = self.buf.split_at_mut(write_start);

        let first_len = first_len.min(first.len());
        let second_len = second_len.min(second.len());
        [
            IoSliceMut::new(&mut first[..first_len]),
            IoSliceMut::new(&mut second[..second_len]),
        ]
    }

    /// Read a message into the buffer, try to deserialize it and call the callback on it.
    pub async fn read_message(
        &mut self,
        cx: &Cx,
        conn: &mut BoxAsyncReadVectored,
        timeout: Duration,
    ) -> Result<Message<'_>> {
        loop {
            cx.checkpoint().map_err(|e| Error::Anyhow(e.into()))?;
            let err = {
                // A workaround for borrow-checker not understanding early returns.
                //
                // A stacked reborrow of self. After this block we either return from
                // the function, or the block returns an owned error.
                //
                // Safety: there's nothing inherently unsafe here, this is just a
                // borrow checker workaround. However to ensure we respect aliasing
                // rules (&mut references are exclusive), there's a miri test below.
                //
                // In this case, 2 &mut references DO exist lexically, but in reality the
                // latter one is a reborrow that ends with the block.
                //
                // Run with `cargo +nightly miri test -p librqbit --features miri test_read_buf_miri -- --nocapture --ignore`
                let this = unsafe { &mut *(self as *mut Self) };
                let (first, second) = this.slice_ranges_with_limit(BUFLEN);
                let (first, second) = (&this.buf[first], &this.buf[second]);
                match Message::deserialize(first, second) {
                    Ok((msg, size)) => {
                        advance!(this, size);
                        return Ok(msg);
                    }
                    Err(e) => e,
                }
            };

            match err {
                MessageDeserializeError::NotEnoughData(mut need_additional_bytes, ..) => {
                    while need_additional_bytes > 0 {
                        if self.len >= BUFLEN || self.is_full() {
                            return Err(Self::frame_full_error(need_additional_bytes));
                        }
                        let size = with_timeout(
                            cx,
                            "reading",
                            timeout,
                            conn.read_vectored(
                                cx,
                                &mut self.unfilled_ioslices_with_limit(BUFLEN - self.len),
                            )
                            .map_err(Error::Read),
                        )
                        .await?;
                        if size == 0 {
                            return Err(Error::PeerDisconnected);
                        }
                        self.len += size;
                        need_additional_bytes = need_additional_bytes.saturating_sub(size)
                    }
                }
                MessageDeserializeError::NeedContiguous => {
                    self.make_contiguous()?;
                    continue;
                }
                e => return Err(Error::Deserialize(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use librqbit_core::constants::CHUNK_SIZE;
    use peer_binary_protocol::{
        MAX_MSG_LEN, Message, Piece,
        extended::{
            ExtendedMessage, PeerExtendedMessageIds,
            ut_metadata::{UtMetadata, UtMetadataData},
        },
    };
    use rqbit_runtime::io::{AsyncRead, AsyncWriteExt};
    use std::{future::Future, net::Ipv4Addr, task::Poll, time::Duration};

    use crate::{type_aliases::BoxAsyncReadVectored, vectored_traits::AsyncReadVectored};

    use super::{BUFLEN, ReadBuf};

    struct FragmentedInput {
        bytes: Vec<u8>,
        consumed: usize,
        max_read: usize,
    }

    impl AsyncRead for FragmentedInput {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut rqbit_runtime::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            let count = (this.bytes.len() - this.consumed)
                .min(this.max_read)
                .min(buf.remaining());
            buf.put_slice(&this.bytes[this.consumed..this.consumed + count]);
            this.consumed += count;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncReadVectored for FragmentedInput {
        fn poll_read_vectored(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            vec: &mut [std::io::IoSliceMut<'_>],
        ) -> Poll<std::io::Result<usize>> {
            let this = self.get_mut();
            let mut remaining = (this.bytes.len() - this.consumed).min(this.max_read);
            let mut copied = 0;
            for target in vec {
                let count = remaining.min(target.len());
                target[..count].copy_from_slice(&this.bytes[this.consumed..this.consumed + count]);
                this.consumed += count;
                copied += count;
                remaining -= count;
                if remaining == 0 {
                    break;
                }
            }
            Poll::Ready(Ok(copied))
        }
    }

    fn input(bytes: Vec<u8>, max_read: usize) -> BoxAsyncReadVectored {
        Box::new(FragmentedInput {
            bytes,
            consumed: 0,
            max_read,
        })
    }

    fn piece_wire(payload: &[u8]) -> Vec<u8> {
        let mut wire = vec![0; payload.len() + 13];
        let count = Message::Piece(Piece::from_data(42, 43, payload))
            .serialize(&mut wire, &Default::default)
            .unwrap();
        assert_eq!(count, wire.len());
        wire
    }

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

    #[derive(Clone, Copy)]
    enum ProbeEnd {
        Pending,
        Eof,
        Fault,
    }

    struct ProbeInput {
        end: ProbeEnd,
        polls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl AsyncRead for ProbeInput {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut rqbit_runtime::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut target = [std::io::IoSliceMut::new(buf.unfilled())];
            match AsyncReadVectored::poll_read_vectored(self, cx, &mut target) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(count)) => {
                    buf.advance(count);
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            }
        }
    }
    impl AsyncReadVectored for ProbeInput {
        fn poll_read_vectored(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _target: &mut [std::io::IoSliceMut<'_>],
        ) -> Poll<std::io::Result<usize>> {
            self.polls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match self.end {
                ProbeEnd::Pending => Poll::Pending,
                ProbeEnd::Eof => Poll::Ready(Ok(0)),
                ProbeEnd::Fault => {
                    Poll::Ready(Err(std::io::Error::other("coalescing input fault")))
                }
            }
        }
    }
    fn probe(
        end: ProbeEnd,
    ) -> (
        BoxAsyncReadVectored,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Box::new(ProbeInput {
                end,
                polls: polls.clone(),
            }),
            polls,
        )
    }

    async fn coalescing_batch(cx: &rqbit_runtime::Cx, suffix: &[u8]) -> super::OwnedPieceBatch {
        let mut wire = piece_wire(b"first");
        wire.extend_from_slice(suffix);
        let mut src = input(wire, usize::MAX);
        let mut rb = ReadBuf::new();
        rb.grow_for_piece_batch();
        let meta = {
            let Message::Piece(piece) = rb
                .read_message(cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!("expected first Piece");
            };
            (piece.index, piece.begin, piece.len(), piece.data().0.len())
        };
        rb.into_piece_frame(meta.0, meta.1, meta.2, meta.3)
            .unwrap()
            .into_batch()
    }

    #[test]
    fn piece_coalescing_first_key_fence_does_not_poll_input() {
        let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
            .blocking_threads(1, 2)
            .build()
            .unwrap();
        let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let key = cx.reusable_region_owner_key().unwrap();
        runtime.block_on(async {
            let mut batch = coalescing_batch(&cx, &[]).await;
            let (mut src, polls) = probe(ProbeEnd::Pending);
            cx.apply_child_capability_budget(
                rqbit_runtime::types::CapabilityBudget::UNSPECIFIED.with_io_bytes(65_536),
                rqbit_runtime::types::CapabilityBudgetRequirements::NONE,
            )
            .unwrap();
            assert!(cx.checkpoint().is_ok());
            assert!(cx.reusable_region_owner_key().is_none());
            batch.intake_coalescing(&cx, &key, &mut src).await.unwrap();
            assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
            assert!(batch.deferred_read_error.is_none());
            assert_eq!(
                [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
                b"first"
            );
        });
        assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    }

    #[test]
    fn piece_coalescing_ready_control_parse_eof_and_fault_do_not_wait() {
        let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
            .blocking_threads(1, 2)
            .build()
            .unwrap();
        let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let key = cx.reusable_region_owner_key().unwrap();
        runtime.block_on(async {
            let mut batch = coalescing_batch(&cx, &[0, 0, 0, 5, 4, 0, 0, 0, 99]).await;
            let (mut src, polls) = probe(ProbeEnd::Pending);
            // A single manual poll must finish: the control fence cannot park.
            {
                let mut receiving = std::pin::pin!(batch.intake_coalescing(&cx, &key, &mut src));
                assert!(matches!(
                    std::future::poll_fn(|task| Poll::Ready(receiving.as_mut().poll(task))).await,
                    Poll::Ready(Ok(()))
                ));
            }
            assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
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

            for (end, expected) in [(ProbeEnd::Eof, "eof"), (ProbeEnd::Fault, "fault")] {
                let mut batch = coalescing_batch(&cx, &[]).await;
                let (mut src, polls) = probe(end);
                {
                    let mut receiving =
                        std::pin::pin!(batch.intake_coalescing(&cx, &key, &mut src));
                    assert!(matches!(
                        std::future::poll_fn(|task| Poll::Ready(receiving.as_mut().poll(task)))
                            .await,
                        Poll::Ready(Ok(()))
                    ));
                }
                assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 1);
                assert_eq!(batch.remaining_len(), 1);
                match (expected, &batch.deferred_read_error) {
                    ("eof", Some(crate::Error::PeerDisconnected)) => {}
                    ("fault", Some(crate::Error::Read(error))) => {
                        assert_eq!(error.to_string(), "coalescing input fault")
                    }
                    _ => panic!("expected deferred {expected}"),
                }
            }

            let mut batch = coalescing_batch(&cx, &piece_wire(&[])).await;
            let (mut src, polls) = probe(ProbeEnd::Pending);
            batch.intake_coalescing(&cx, &key, &mut src).await.unwrap();
            assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert_eq!(batch.remaining_len(), 1);
            assert!(matches!(
                batch.deferred_read_error,
                Some(crate::Error::Deserialize(
                    peer_binary_protocol::MessageDeserializeError::IncorrectMsgLen { .. }
                ))
            ));
        });
        assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    }

    #[test]
    fn piece_coalescing_timeout_preserves_prefix_and_absolute_deadline() {
        let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
            .blocking_threads(1, 2)
            .build()
            .unwrap();
        let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let key = cx.reusable_region_owner_key().unwrap();
        runtime.block_on(async {
            assert_eq!(super::PIECE_COALESCING_WINDOW, Duration::from_micros(1_500));
            let mut batch = coalescing_batch(&cx, &[]).await;
            let (mut src, polls) = probe(ProbeEnd::Pending);
            let deadline = cx.now() + super::PIECE_COALESCING_WINDOW;
            batch
                .intake_until(&cx, &key, &mut src, deadline)
                .await
                .unwrap();
            assert!(
                cx.now() >= deadline,
                "a healthy Pending input reaches its original cutoff"
            );
            assert!(polls.load(std::sync::atomic::Ordering::Relaxed) >= 1);
            assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
            assert!(batch.deferred_read_error.is_none());
            assert_eq!(
                [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
                b"first"
            );

            // No synthetic clock: use an already-expired native cutoff while
            // a second complete Piece is buffered. Parsing it cannot reset
            // that cutoff and permit a further socket read.
            let mut batch = coalescing_batch(&cx, &piece_wire(b"second")).await;
            let (mut src, polls) = probe(ProbeEnd::Pending);
            let cutoff = rqbit_runtime::types::Time::ZERO;
            batch
                .intake_until(&cx, &key, &mut src, cutoff)
                .await
                .unwrap();
            assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert_eq!(batch.remaining_len(), 2);
            assert_eq!(
                [batch.as_piece(1).data().0, batch.as_piece(1).data().1].concat(),
                b"second"
            );
            assert_eq!(cutoff, rqbit_runtime::types::Time::ZERO);
        });
        assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    }

    mod adaptive_coalescing {
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
    }

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
    #[cfg(test)]
    mod tcp_batch_metamorphic {
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
    }

    #[cfg(test)]
    mod coalescing_delayed_tcp_fences {
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
    }
}
