//! Native Message Stream Encryption transport. The sans-I/O protocol in
//! `protocol` is adapted from Apache-2.0 urtorrent-mse 0.14.2; see its NOTICE.
//! MSE is BitTorrent traffic obfuscation, without peer authentication or a MAC.
mod protocol;

use crate::{
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
    vectored_traits::AsyncReadVectored,
};
use anyhow::{Context as _, bail};
use librqbit_core::hash_id::Id20;
use peer_binary_protocol::Handshake;
use protocol::{Initiator, Outcome, Rc4Stream, Responder, SkeyIndex};
use rqbit_runtime::{
    Cx,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, IoSliceMut},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// Peer encryption policy, applied to metadata and payload connections alike.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerEncryption {
    /// Use and accept ordinary plaintext peer connections only.
    Off,
    /// Try RC4 first; a failed handshake permits a fresh plaintext connection.
    #[default]
    Prefer,
    /// Require RC4 for the handshake and all following peer traffic.
    Require,
}

struct Random;
impl protocol::Rng for Random {
    fn next_u32(&mut self) -> u32 {
        rand::random()
    }
}

fn private_key() -> [u8; 20] {
    let mut key = [0u8; 20];
    rand::fill(&mut key);
    key
}

// At most 65535 bytes of IA, four bounded pads, the fixed handshake, and one
// small read ahead. No untrusted field can grow this buffer without a bound.
const HANDSHAKE_BYTE_LIMIT: usize = 70 * 1024;
const READ_SIZE: usize = 1024;

async fn flush(cx: &Cx, writer: &mut BoxAsyncWrite, bytes: Vec<u8>) -> anyhow::Result<()> {
    cx.checkpoint()?;
    if !bytes.is_empty() {
        writer
            .write_all(&bytes)
            .await
            .context("writing MSE handshake")?;
    }
    Ok(())
}

async fn next(
    cx: &Cx,
    reader: &mut BoxAsyncReadVectored,
    buf: &mut [u8],
    total: &mut usize,
) -> anyhow::Result<usize> {
    cx.checkpoint()?;
    let n = reader.read(buf).await.context("reading MSE handshake")?;
    if n == 0 {
        bail!("peer closed during MSE handshake");
    }
    *total += n;
    if *total > HANDSHAKE_BYTE_LIMIT {
        bail!("MSE handshake exceeds byte limit");
    }
    Ok(n)
}

fn wrap(
    reader: BoxAsyncReadVectored,
    writer: BoxAsyncWrite,
    outcome: Outcome,
) -> (BoxAsyncReadVectored, BoxAsyncWrite) {
    (
        Box::new(CryptoReader {
            inner: reader,
            cipher: outcome.decrypt,
            prefix: outcome.plaintext,
            offset: 0,
        }),
        Box::new(CryptoWriter {
            inner: writer,
            cipher: outcome.encrypt,
            scratch: Vec::new(),
        }),
    )
}

/// Initiate an RC4-only exchange. IA carries the local BEP3 handshake, so the
/// caller must not write it again once this succeeds.
pub(crate) async fn outgoing(
    cx: &Cx,
    mut reader: BoxAsyncReadVectored,
    mut writer: BoxAsyncWrite,
    info_hash: Id20,
    handshake: Vec<u8>,
) -> anyhow::Result<(BoxAsyncReadVectored, BoxAsyncWrite)> {
    cx.checkpoint()?;
    let mut rng = Random;
    let mut protocol = Initiator::new(info_hash.0, private_key(), handshake, 2, &mut rng);
    flush(cx, &mut writer, protocol.take_outbound()).await?;
    let mut buffer = [0u8; READ_SIZE];
    let mut total = 0;
    loop {
        let n = next(cx, &mut reader, &mut buffer, &mut total).await?;
        let outcome = protocol.receive(&buffer[..n], &mut rng)?;
        flush(cx, &mut writer, protocol.take_outbound()).await?;
        if let Some(outcome) = outcome {
            if outcome.method != protocol::CryptoMethod::Rc4 {
                bail!("peer selected plaintext payload");
            }
            return Ok(wrap(reader, writer, outcome));
        }
    }
}

/// Detect ordinary BEP3 vs MSE, finish crypto, and preserve any initial payload.
/// The returned stream key must match the inner BEP3 infohash at the caller.
pub(crate) async fn incoming(
    cx: &Cx,
    mut reader: BoxAsyncReadVectored,
    mut writer: BoxAsyncWrite,
    policy: PeerEncryption,
    torrents: &[Id20],
) -> anyhow::Result<(BoxAsyncReadVectored, BoxAsyncWrite, Option<Id20>)> {
    cx.checkpoint()?;
    let mut prefix = [0u8; 20];
    reader
        .read_exact(&mut prefix)
        .await
        .context("reading peer protocol prefix")?;
    if &prefix == b"\x13BitTorrent protocol" {
        if policy == PeerEncryption::Require {
            bail!("plaintext peer rejected: encryption is required");
        }
        return Ok((
            Box::new(CryptoReader {
                inner: reader,
                cipher: None,
                prefix: prefix.to_vec(),
                offset: 0,
            }),
            writer,
            None,
        ));
    }
    if policy == PeerEncryption::Off {
        bail!("encrypted peer rejected: encryption is off");
    }
    let mut index = SkeyIndex::new();
    for hash in torrents {
        index.insert(hash.0);
    }
    let mut rng = Random;
    // Negotiating plaintext payload would defeat Require. Prefer also selects
    // RC4; its compatibility escape is an ordinary fresh plaintext connection.
    let mut protocol = Responder::new(private_key(), 2, true);
    let mut outcome = protocol.receive(&prefix, &index, &mut rng)?;
    let mut buffer = [0u8; READ_SIZE];
    let mut total = prefix.len();
    loop {
        flush(cx, &mut writer, protocol.take_outbound()).await?;
        if let Some(outcome) = outcome {
            let hash = Id20::new(outcome.info_hash);
            let (reader, writer) = wrap(reader, writer, outcome);
            return Ok((reader, writer, Some(hash)));
        }
        let n = next(cx, &mut reader, &mut buffer, &mut total).await?;
        outcome = protocol.receive(&buffer[..n], &index, &mut rng)?;
    }
}

/// Includes the full MSE and BEP3 handshake in one timeout, so partial input
/// cannot extend the deadline. Interrupted handshakes are dropped by ownership.
pub(crate) async fn incoming_handshake(
    cx: &Cx,
    reader: BoxAsyncReadVectored,
    writer: BoxAsyncWrite,
    policy: PeerEncryption,
    torrents: &[Id20],
    duration: Duration,
) -> anyhow::Result<(
    BoxAsyncReadVectored,
    BoxAsyncWrite,
    crate::read_buf::ReadBuf,
    Handshake,
    bool,
)> {
    let result = rqbit_runtime::time::timeout(cx, duration, async {
        let (mut reader, writer, key) = incoming(cx, reader, writer, policy, torrents).await?;
        let mut read_buf = crate::read_buf::ReadBuf::new();
        let handshake = read_buf.read_handshake(cx, &mut reader, duration).await?;
        if key.is_some_and(|hash| hash != handshake.info_hash) {
            bail!("MSE stream key does not match BitTorrent infohash");
        }
        Ok((reader, writer, read_buf, handshake, key.is_some()))
    })
    .await
    .context("incoming peer handshake timed out")?;
    cx.checkpoint()?;
    result
}

struct CryptoReader {
    inner: BoxAsyncReadVectored,
    cipher: Option<Rc4Stream>,
    prefix: Vec<u8>,
    offset: usize,
}
impl CryptoReader {
    fn prefix_read(&mut self, target: &mut [u8]) -> usize {
        let n = target.len().min(self.prefix.len() - self.offset);
        target[..n].copy_from_slice(&self.prefix[self.offset..self.offset + n]);
        self.offset += n;
        if self.offset == self.prefix.len() {
            self.prefix.clear();
            self.offset = 0;
        }
        n
    }
}
impl AsyncRead for CryptoReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() && buf.remaining() > 0 {
            let n = buf.remaining().min(this.prefix.len() - this.offset);
            buf.put_slice(&this.prefix[this.offset..this.offset + n]);
            this.offset += n;
            if this.offset == this.prefix.len() {
                this.prefix.clear();
                this.offset = 0;
            }
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if let Some(cipher) = &mut this.cipher {
                    cipher.apply(&mut buf.filled_mut()[before..]);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
impl AsyncReadVectored for CryptoReader {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            let mut n = 0;
            for buf in bufs {
                n += this.prefix_read(buf);
                if this.prefix.is_empty() {
                    break;
                }
            }
            return Poll::Ready(Ok(n));
        }
        match Pin::new(&mut this.inner).poll_read_vectored(cx, bufs) {
            Poll::Ready(Ok(n)) => {
                if let Some(cipher) = &mut this.cipher {
                    let mut left = n;
                    for buf in bufs {
                        let take = left.min(buf.len());
                        cipher.apply(&mut buf[..take]);
                        left -= take;
                        if left == 0 {
                            break;
                        }
                    }
                }
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }
}
struct CryptoWriter {
    inner: BoxAsyncWrite,
    cipher: Option<Rc4Stream>,
    scratch: Vec<u8>,
}
impl AsyncWrite for CryptoWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let Some(cipher) = &mut this.cipher else {
            return Pin::new(&mut this.inner).poll_write(cx, buf);
        };
        let n = buf.len().min(32 * 1024);
        this.scratch.clear();
        this.scratch.extend_from_slice(&buf[..n]);
        // Speculative encryption: Pending and a partial write must not advance
        // the live keystream over bytes the socket did not accept. Cloning also
        // allows a cancelled poll_write future to be replaced with another one.
        let mut speculative = cipher.clone();
        speculative.apply(&mut this.scratch);
        match Pin::new(&mut this.inner).poll_write(cx, &this.scratch) {
            Poll::Ready(Ok(written)) if written <= n => {
                if written == n {
                    // Normal full writes commit the prepared state directly.
                    *cipher = speculative;
                } else {
                    cipher.apply(&mut this.scratch[..written]);
                }
                Poll::Ready(Ok(written))
            }
            Poll::Ready(Ok(_)) => {
                Poll::Ready(Err(io::Error::other("invalid transport write length")))
            }
            other => other,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
