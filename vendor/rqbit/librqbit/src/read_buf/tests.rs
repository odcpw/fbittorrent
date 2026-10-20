// Shared fixtures stay in this module so existing test names and paths remain stable.
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

include!("tests/owned_frames.rs");

include!("tests/piece_batch.rs");

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

include!("tests/coalescing.rs");

mod adaptive_coalescing;

include!("tests/ring_buffer.rs");

#[cfg(test)]
mod tcp_batch_metamorphic;

#[cfg(test)]
mod coalescing_delayed_tcp_fences;
