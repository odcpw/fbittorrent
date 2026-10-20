use rqbit_runtime::Cx;
use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use bencode::from_bytes;
use buffers::{ByteBuf, ByteBufOwned};
use bytes::Bytes;
use librqbit_core::{
    constants::CHUNK_SIZE,
    hash_id::Id20,
    lengths::{ChunkInfo, last_element_size},
    torrent_metainfo::TorrentMetaV1Info,
};
use parking_lot::{Mutex, RwLock};
use peer_binary_protocol::{
    Handshake, Message,
    extended::{
        ExtendedMessage,
        handshake::ExtendedHandshake,
        ut_metadata::{UtMetadata, UtMetadataData},
    },
};
use rqbit_runtime::sync::mpsc::UnboundedSender;
use sha1w::{ISha1, Sha1};
use tracing::trace;

use crate::{
    peer_connection::{
        PeerConnection, PeerConnectionHandler, PeerConnectionOptions, WriteBuffer, WriterRequest,
    },
    spawn_utils::BlockingSpawner,
    stream_connect::{ConnectionKind, StreamConnector},
};

pub(crate) async fn read_metainfo_from_peer(
    cx: &Cx,
    addr: SocketAddr,
    peer_id: Id20,
    info_hash: Id20,
    peer_connection_options: Option<PeerConnectionOptions>,
    spawner: BlockingSpawner,
    connector: Arc<StreamConnector>,
    client_name_and_version: String,
) -> anyhow::Result<TorrentAndInfoBytes> {
    cx.checkpoint()?;
    let (result_tx, mut result_rx) = rqbit_runtime::sync::oneshot::channel::<
        Result<(TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned), bencode::DeserializeError>,
    >();
    let (writer_tx, writer_rx) = rqbit_runtime::sync::mpsc::unbounded_channel::<WriterRequest>();
    let handler = Handler {
        addr,
        info_hash,
        writer_tx,
        result_tx: Mutex::new(Some(result_tx)),
        locked: RwLock::new(None),
        client_name_and_version,
    };
    let connection = PeerConnection::new(
        addr,
        info_hash,
        peer_id,
        handler,
        peer_connection_options,
        spawner,
        connector,
    );

    let result_reader = result_rx.recv(cx);
    let (_, brx) = rqbit_runtime::sync::broadcast::channel(1);
    let connection_runner =
        async move { connection.manage_peer_outgoing(cx, writer_rx, brx).await };

    futures::pin_mut!(result_reader, connection_runner);
    match futures::future::select(result_reader, connection_runner).await {
        futures::future::Either::Left((result, _)) => Ok(result??),
        futures::future::Either::Right((whatever, _)) => match whatever {
            Ok(_) => anyhow::bail!("connection runner completed first"),
            Err(e) => Err(e.into()),
        },
    }
}

/// One peer may announce up to this much metadata.
const MAX_METADATA_SIZE: u32 = 32 * 1024 * 1024;
/// Bytes allocated at once for peer-announced metadata across every magnet
/// resolution. Each peer chooses its own `metadata_size`; without a shared
/// budget, many concurrent peers can each make us allocate the maximum.
const IN_FLIGHT_METADATA_CAP: usize = 64 * 1024 * 1024;
static IN_FLIGHT_METADATA: MetadataBudget = MetadataBudget::new(IN_FLIGHT_METADATA_CAP);

struct MetadataBudget {
    used: std::sync::atomic::AtomicUsize,
    cap: usize,
}
impl MetadataBudget {
    const fn new(cap: usize) -> Self {
        Self {
            used: std::sync::atomic::AtomicUsize::new(0),
            cap,
        }
    }
    fn reserve(&'static self, bytes: usize) -> anyhow::Result<MetadataReservation> {
        use std::sync::atomic::Ordering;
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(bytes).filter(|total| *total <= self.cap)
            })
            .map_err(|_| anyhow::anyhow!("too much peer metadata is already being received"))?;
        Ok(MetadataReservation(Some((self, bytes))))
    }
}

/// A share of a metadata budget, returned on drop.
#[derive(Default)]
struct MetadataReservation(Option<(&'static MetadataBudget, usize)>);
impl Drop for MetadataReservation {
    fn drop(&mut self) {
        if let Some((budget, bytes)) = self.0.take() {
            budget
                .used
                .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
        }
    }
}

#[derive(Default)]
struct HandlerLocked {
    metadata_size: u32,
    total_pieces: usize,
    buffer: Vec<u8>,
    received_pieces: Vec<bool>,
    _reservation: MetadataReservation,
}

impl HandlerLocked {
    fn new(metadata_size: u32) -> anyhow::Result<Self> {
        if metadata_size == 0 || metadata_size > MAX_METADATA_SIZE {
            anyhow::bail!("metadata size {} is invalid or too big", metadata_size);
        }
        // Reserve before allocating; the reservation lives with the buffer.
        let reservation = IN_FLIGHT_METADATA.reserve(metadata_size as usize)?;
        let buffer = vec![0u8; metadata_size as usize];
        let total_pieces: usize = (metadata_size as u64)
            .div_ceil(CHUNK_SIZE as u64)
            .try_into()?;
        let received_pieces = vec![false; total_pieces];
        Ok(Self {
            metadata_size,
            received_pieces,
            buffer,
            total_pieces,
            _reservation: reservation,
        })
    }
    fn piece_size(&self, index: u32) -> usize {
        if index as usize == self.total_pieces - 1 {
            last_element_size(self.metadata_size as u64, CHUNK_SIZE as u64)
                .try_into()
                .unwrap()
        } else {
            CHUNK_SIZE as usize
        }
    }
    fn record_piece(
        &mut self,
        d: &UtMetadataData<ByteBuf>,
        info_hash: &Id20,
    ) -> anyhow::Result<bool> {
        let piece = d.piece();
        if piece as usize >= self.total_pieces {
            anyhow::bail!("wrong index");
        }
        let offset = (piece * CHUNK_SIZE) as usize;
        let size = self.piece_size(piece);
        if d.len() != size {
            anyhow::bail!(
                "expected length of piece {} to be {}, but got {}",
                piece,
                size,
                d.len()
            );
        }
        if self.received_pieces[piece as usize] {
            anyhow::bail!("already received piece {}", piece);
        }
        d.copy_to_slice(&mut self.buffer[offset..offset + d.len()]);
        self.received_pieces[piece as usize] = true;

        if self.received_pieces.iter().all(|p| *p) {
            // check metadata
            let mut hash = Sha1::new();
            hash.update(&self.buffer);
            if hash.finish() != info_hash.0 {
                anyhow::bail!("info checksum invalid");
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub type TorrentAndInfoBytes = (TorrentMetaV1Info<ByteBufOwned>, ByteBufOwned);

struct Handler {
    addr: SocketAddr,
    info_hash: Id20,
    writer_tx: UnboundedSender<WriterRequest>,
    result_tx: Mutex<
        Option<
            rqbit_runtime::sync::oneshot::Sender<
                Result<TorrentAndInfoBytes, bencode::DeserializeError>,
            >,
        >,
    >,
    locked: RwLock<Option<HandlerLocked>>,
    client_name_and_version: String,
}

impl PeerConnectionHandler for Handler {
    // Metadata bootstrap has no payload store. Keep its wire state choked just
    // like the download-only live torrent; BEP 9 metadata does not need Unchoke.
    fn upload_enabled(&self) -> bool {
        false
    }

    fn should_send_bitfield(&self) -> bool {
        false
    }

    fn serialize_bitfield_message_to_buf(&self, _buf: &mut [u8]) -> anyhow::Result<usize> {
        Ok(0)
    }

    fn on_handshake(&self, handshake: Handshake, _kind: ConnectionKind) -> anyhow::Result<()> {
        if !handshake.supports_extended() {
            anyhow::bail!(
                "this peer does not support extended handshaking, which is a prerequisite to download metadata"
            )
        }
        Ok(())
    }

    async fn on_received_message(&self, cx: &Cx, msg: Message<'_>) -> anyhow::Result<()> {
        cx.checkpoint()?;
        trace!("{}: received message: {:?}", self.addr, msg);

        if let Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(utdata))) = msg {
            // A peer may send data before its extended handshake, or after
            // the metadata completed: refuse it instead of panicking.
            let piece_ready = self
                .locked
                .write()
                .as_mut()
                .context("metadata piece before an accepted extended handshake")?
                .record_piece(&utdata, &self.info_hash)?;
            if piece_ready {
                let mut locked = self
                    .locked
                    .write()
                    .take()
                    .context("metadata already completed")?;
                let buf = Bytes::from(std::mem::take(&mut locked.buffer));
                drop(locked);
                let info = from_bytes::<TorrentMetaV1Info<ByteBuf>>(&buf)
                    .map(|i| {
                        use clone_to_owned::CloneToOwned;
                        i.clone_to_owned(Some(&buf))
                    })
                    .map_err(|e| {
                        trace!("error deserializing TorrentMetaV1Info: {e:#}");
                        e.into_kind()
                    })
                    .map(|i| (i, ByteBufOwned(buf)));

                self.result_tx
                    .lock()
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("oneshot is consumed"))?
                    .send(cx, info)
                    .map_err(|_| {
                        anyhow::anyhow!("torrent info deserialized, but consumer closed")
                    })?;
            }
        }
        Ok(())
    }

    fn on_uploaded_bytes(&self, _bytes: u32) {}

    async fn read_chunk(
        &self,
        _cx: &Cx,
        _chunk: ChunkInfo,
        _buffer: WriteBuffer,
        _payload_offset: usize,
    ) -> crate::Result<WriteBuffer> {
        Err(crate::Error::ReadChunk(anyhow::anyhow!(
            "the peer is not supposed to be requesting chunks"
        )))
    }

    fn on_extended_handshake(
        &self,
        extended_handshake: &ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        let metadata_size = match extended_handshake.metadata_size {
            Some(metadata_size) => metadata_size,
            None => anyhow::bail!("peer does not have metadata_size"),
        };

        if extended_handshake.m.ut_metadata.is_none() {
            anyhow::bail!("peer does not support ut_metadata");
        }

        self.writer_tx
            .send(WriterRequest::Message(Message::Unchoke))?;
        self.writer_tx
            .send(WriterRequest::Message(Message::Interested))?;

        // A repeated handshake releases the previous buffer before reserving.
        self.locked.write().take();
        let inner = HandlerLocked::new(metadata_size)?;
        let total_pieces = inner.total_pieces;

        self.locked.write().replace(inner);

        for i in 0..total_pieces {
            self.writer_tx
                .send(WriterRequest::Message(Message::Extended(
                    ExtendedMessage::UtMetadata(UtMetadata::Request(i.try_into()?)),
                )))?;
        }
        Ok(())
    }

    fn should_transmit_have(&self, _id: librqbit_core::lengths::ValidPieceIndex) -> bool {
        false
    }

    fn client_name_and_version(&self) -> &str {
        &self.client_name_and_version
    }
}

#[cfg(test)]
mod budget_tests {
    use super::{HandlerLocked, MAX_METADATA_SIZE, MetadataBudget};
    use std::sync::atomic::Ordering;

    #[test]
    fn peer_metadata_allocations_share_one_bounded_budget() {
        assert!(HandlerLocked::new(0).is_err());
        assert!(HandlerLocked::new(MAX_METADATA_SIZE + 1).is_err());
        let ok = HandlerLocked::new(1000).unwrap();
        assert_eq!(ok.buffer.len(), 1000);
        // A private budget: concurrent tests may hold shares of the global one.
        static BUDGET: MetadataBudget = MetadataBudget::new(3 * MAX_METADATA_SIZE as usize);
        let max = MAX_METADATA_SIZE as usize;
        let held: Vec<_> = (0..3).map(|_| BUDGET.reserve(max).unwrap()).collect();
        // Another peer announcing a large size is refused, a tiny one too.
        assert!(BUDGET.reserve(max).is_err());
        assert!(BUDGET.reserve(1).is_err());
        assert_eq!(BUDGET.used.load(Ordering::Acquire), 3 * max);
        drop(held);
        // Dropping a handler (peer gone or metadata complete) returns its share.
        assert_eq!(BUDGET.used.load(Ordering::Acquire), 0);
        assert!(BUDGET.reserve(max).is_ok());
        assert_eq!(BUDGET.used.load(Ordering::Acquire), 0);
    }
}
