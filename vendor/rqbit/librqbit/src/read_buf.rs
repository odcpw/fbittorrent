//! Socket ring buffer, wire-frame bounds and owned receive types.
//! Piece ownership and batched intake are implemented in `piece_batch`.

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

mod piece_batch;

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

/// A completed prefix and the terminal read error observed after that prefix.
/// The reader returns this error directly, preserving its transport category.
pub struct PieceReceiveOutcome {
    pub(crate) read_buf: ReadBuf,
    pub(crate) deferred_read_error: Option<Error>,
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
mod tests;
