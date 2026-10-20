use std::{
    net::SocketAddr,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    Error, PeerEncryption, Result, session::CheckedIncomingConnection,
    stream_connect::ConnectionKind,
};
use buffers::{ByteBuf, ByteBufOwned};
use futures::{FutureExt, TryFutureExt};
use librqbit_core::{
    hash_id::Id20,
    lengths::{ChunkInfo, ValidPieceIndex},
    peer_id::try_decode_peer_id,
};
use parking_lot::RwLock;
use peer_binary_protocol::{
    Handshake, MAX_MSG_LEN, Message,
    extended::{
        ExtendedMessage, PeerExtendedMessageIds, handshake::ExtendedHandshake,
        ut_metadata::UtMetadata, ut_pex::UtPex,
    },
    serialize_piece_preamble,
};
use rqbit_runtime::sync::{OwnedSemaphorePermit, Semaphore};
use rqbit_runtime::time::timeout;
use rqbit_runtime::{Cx, cx::cx::RegionOwnerKey};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tracing::{Instrument, trace, trace_span};

use crate::{
    read_buf::{OwnedPieceFrame, PieceReceiveOutcome, ReadBuf},
    spawn_utils::BlockingSpawner,
    stream_connect::StreamConnector,
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
};

pub(crate) type WriteBuffer = Box<[u8; MAX_MSG_LEN]>;

/// Moves the writer's existing buffer through an owned native blocking read.
/// The region-owned operation holds admission until its actual read finishes,
/// including when the peer drops its waiter. The operation acquires its own
/// permit so the runtime's obligation holder stays live for that whole lifetime.
pub(crate) async fn read_owned_upload<F>(
    cx: &Cx,
    admission: Arc<Semaphore>,
    mut buffer: WriteBuffer,
    payload: Range<usize>,
    read: F,
) -> Result<WriteBuffer>
where
    F: FnOnce(&mut [u8]) -> anyhow::Result<()> + Send + 'static,
{
    cx.checkpoint()
        .map_err(|error| Error::Anyhow(error.into()))?;
    let mut operation = cx
        .spawn(move |operation_cx| async move {
            let permit = OwnedSemaphorePermit::acquire(admission, &operation_cx, 1).await?;
            let read_cx = operation_cx.clone();
            // This task stays alive while the pool callback owns its permit.
            // Cancelling the peer waiter must not complete the holder early.
            let buffer = rqbit_runtime::runtime::spawn_blocking(move || {
                let _permit = permit;
                read_cx
                    .checkpoint()
                    .map_err(|error| Error::Anyhow(error.into()))?;
                read(&mut buffer[payload]).map_err(Error::ReadChunk)?;
                Ok::<_, Error>(buffer)
            })
            .await?;
            operation_cx
                .checkpoint()
                .map_err(|error| Error::Anyhow(error.into()))?;
            Ok::<_, Error>(buffer)
        })
        .map_err(|error| Error::Anyhow(error.into()))?;
    let buffer = operation.join(cx).await.map_err(|error| {
        Error::Anyhow(anyhow::anyhow!("native upload read failed: {error:?}"))
    })??;
    cx.checkpoint()
        .map_err(|error| Error::Anyhow(error.into()))?;
    Ok(buffer)
}

pub trait PeerConnectionHandler {
    fn on_connected(&self, _connection_time: Duration) {}
    fn on_connection_flags(&self, _encrypted: bool, _incoming: bool) {}
    fn should_send_bitfield(&self) -> bool;
    fn upload_enabled(&self) -> bool { true }
    fn serialize_bitfield_message_to_buf(&self, buf: &mut [u8]) -> anyhow::Result<usize>;
    fn on_handshake(&self, handshake: Handshake, ckind: ConnectionKind) -> anyhow::Result<()>;
    fn on_extended_handshake(
        &self,
        extended_handshake: &ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()>;
    async fn on_received_message(&self, cx: &Cx, msg: Message<'_>) -> anyhow::Result<()>;
    fn received_piece_batch_domain(&self, _cx: &Cx) -> Option<RegionOwnerKey> {
        None
    }
    async fn on_received_owned_piece(
        &self,
        cx: &Cx,
        frame: OwnedPieceFrame,
        _input: &mut BoxAsyncReadVectored,
    ) -> anyhow::Result<PieceReceiveOutcome> {
        self.on_received_message(cx, Message::Piece(frame.as_piece()))
            .await?;
        Ok(PieceReceiveOutcome::completed(frame.into_read_buf()))
    }
    fn should_transmit_have(&self, id: ValidPieceIndex) -> bool;
    fn on_uploaded_bytes(&self, bytes: u32);
    async fn read_chunk(
        &self,
        cx: &Cx,
        chunk: ChunkInfo,
        buffer: WriteBuffer,
        payload_offset: usize,
    ) -> Result<WriteBuffer>;
    fn update_my_extended_handshake(
        &self,
        _handshake: &mut ExtendedHandshake<ByteBuf>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn client_name_and_version(&self) -> &str {
        crate::client_name_and_version()
    }
}

#[derive(Debug)]
pub enum WriterRequest {
    Message(Message<'static>),
    UtMetadata(UtMetadata<ByteBufOwned>),
    UtPex(UtPex<ByteBufOwned>),
    ReadChunkRequest(ChunkInfo),
    UploadPolicyChanged,
    Disconnect(anyhow::Result<()>),
}

/// Coalesce only already-queued payload requests. No wait to fill a batch, no
/// crossing control/policy messages, and a bounded amount of work between the
/// writer's cancellation checkpoints. Socket writes retain their binding checks.
fn request_batch(
    first: peer_binary_protocol::Request,
    outgoing: &mut rqbit_runtime::sync::mpsc::UnboundedReceiver<WriterRequest>,
    pending: &mut Option<WriterRequest>,
    buffer: &mut [u8; MAX_MSG_LEN],
) -> Result<usize> {
    const MAX_REQUESTS: usize = 16;
    let mut len = Message::Request(first).serialize(buffer, &Default::default)?;
    for _ in 1..MAX_REQUESTS {
        match outgoing.try_recv() {
            Ok(WriterRequest::Message(Message::Request(request))) => {
                len += Message::Request(request).serialize(&mut buffer[len..], &Default::default)?;
            }
            Ok(other) => {
                *pending = Some(other);
                break;
            }
            Err(_) => break,
        }
    }
    Ok(len)
}

#[serde_as]
#[derive(Default, Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PeerConnectionOptions {
    /// None inherits the session policy.
    #[serde(default)]
    pub encryption: Option<PeerEncryption>,

    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub connect_timeout: Option<Duration>,

    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub read_write_timeout: Option<Duration>,

    #[serde_as(as = "Option<serde_with::DurationSeconds>")]
    pub keep_alive_interval: Option<Duration>,
}

pub(crate) struct PeerConnection<H> {
    handler: H,
    addr: SocketAddr,
    info_hash: Id20,
    peer_id: Id20,
    options: PeerConnectionOptions,
    _spawner: BlockingSpawner,
    connector: Arc<StreamConnector>,
}

#[cfg(not(feature = "miri"))]
pub(crate) async fn with_timeout<T>(
    cx: &Cx,
    name: &'static str,
    timeout_value: Duration,
    fut: impl std::future::Future<Output = Result<T>>,
) -> crate::Result<T> {
    cx.checkpoint()?;
    match timeout(cx, timeout_value, fut).await {
        Ok(v) => v,
        Err(_) => Err(Error::Timeout(name)),
    }
}

#[cfg(feature = "miri")]
pub(crate) async fn with_timeout<T>(
    cx: &Cx,
    _name: &'static str,
    _timeout_value: Duration,
    fut: impl std::future::Future<Output = Result<T>>,
) -> crate::Result<T> {
    cx.checkpoint()?;
    fut.await
}

struct ManagePeerArgs {
    handshake_supports_extended: bool,
    read_buf: ReadBuf,
    write_buf: WriteBuffer,
    read: BoxAsyncReadVectored,
    write: BoxAsyncWrite,
    outgoing_chan: rqbit_runtime::sync::mpsc::UnboundedReceiver<WriterRequest>,
    have_broadcast: rqbit_runtime::sync::broadcast::Receiver<ValidPieceIndex>,
}

impl<H: PeerConnectionHandler> PeerConnection<H> {
    pub fn new(
        addr: SocketAddr,
        info_hash: Id20,
        peer_id: Id20,
        handler: H,
        options: Option<PeerConnectionOptions>,
        spawner: BlockingSpawner,
        connector: Arc<StreamConnector>,
    ) -> Self {
        PeerConnection {
            handler,
            addr,
            info_hash,
            peer_id,
            _spawner: spawner,
            options: options.unwrap_or_default(),
            connector,
        }
    }

    // By the time this is called:
    // read_buf should start with valuable data. The handshake should be removed from it.
    pub async fn manage_peer_incoming(
        &self,
        cx: &Cx,
        outgoing_chan: rqbit_runtime::sync::mpsc::UnboundedReceiver<WriterRequest>,
        mut incoming: CheckedIncomingConnection,
        have_broadcast: rqbit_runtime::sync::broadcast::Receiver<ValidPieceIndex>,
    ) -> Result<()> {
        cx.checkpoint()?;
        use rqbit_runtime::io::AsyncWriteExt;

        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        if self.options.encryption == Some(PeerEncryption::Require) && !incoming.encrypted {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "plaintext peer rejected: encryption is required"
            )));
        }

        if incoming.handshake.info_hash != self.info_hash {
            return Err(Error::WrongInfoHash);
        }

        if incoming.handshake.peer_id == self.peer_id {
            return Err(Error::ConnectingToOurselves);
        }

        trace!(
            "incoming connection: id={:?}",
            try_decode_peer_id(incoming.handshake.peer_id)
        );

        let mut write_buf = Box::new([0u8; MAX_MSG_LEN]);
        let handshake = Handshake::new(self.info_hash, self.peer_id);
        let hlen = handshake.serialize_unchecked_len(&mut *write_buf);
        with_timeout(
            cx,
            "writing handshake",
            rwtimeout,
            incoming
                .writer
                .write_all(&write_buf[..hlen])
                .map_err(Error::WriteHandshake),
        )
        .await?;

        let handshake_supports_extended = incoming.handshake.supports_extended();

        self.handler
            .on_handshake(incoming.handshake, incoming.kind)
            .map_err(Error::Anyhow)?;

        self.handler.on_connection_flags(incoming.encrypted, true);
        self.manage_peer(
            cx,
            ManagePeerArgs {
                handshake_supports_extended,
                read_buf: incoming.read_buf,
                write_buf,
                read: incoming.reader,
                write: incoming.writer,
                outgoing_chan,
                have_broadcast,
            },
        )
        .await
    }

    pub async fn manage_peer_outgoing(
        &self,
        cx: &Cx,
        outgoing_chan: rqbit_runtime::sync::mpsc::UnboundedReceiver<WriterRequest>,
        have_broadcast: rqbit_runtime::sync::broadcast::Receiver<ValidPieceIndex>,
    ) -> Result<()> {
        cx.checkpoint()?;
        use rqbit_runtime::io::AsyncWriteExt;
        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let connect_timeout = self
            .options
            .connect_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let now = Instant::now();
        let encryption = self.options.encryption.unwrap_or_default();
        let handshake = Handshake::new(self.info_hash, self.peer_id);
        let mut bytes = vec![0u8; 68];
        let hsz = handshake.serialize_unchecked_len(&mut bytes);
        bytes.truncate(hsz);
        let attempt = |encrypted: bool| {
            let bytes = bytes.clone();
            async move {
                let (kind, mut read, mut write) = with_timeout(
                    cx,
                    "connecting",
                    connect_timeout,
                    self.connector.connect(cx, self.addr),
                )
                .await?;
                // A single deadline covers MSE and the inner BEP3 handshake.
                with_timeout(cx, "peer handshake", rwtimeout, async {
                    if encrypted {
                        (read, write) =
                            crate::mse::outgoing(cx, read, write, self.info_hash, bytes)
                                .await
                                .map_err(Error::Anyhow)?;
                    } else {
                        write
                            .write_all(&bytes)
                            .await
                            .map_err(Error::WriteHandshake)?;
                    }
                    let mut read_buf = ReadBuf::new();
                    let h = read_buf.read_handshake(cx, &mut read, rwtimeout).await?;
                    if h.info_hash != self.info_hash {
                        return Err(Error::WrongInfoHash);
                    }
                    if h.peer_id == self.peer_id {
                        return Err(Error::ConnectingToOurselves);
                    }
                    Ok((kind, read, write, read_buf, h, encrypted))
                })
                .await
            }
        };
        let first = attempt(encryption != PeerEncryption::Off).await;
        let (ckind, read, write, read_buf, h, encrypted) = match first {
            Ok(connected) => connected,
            Err(error) if encryption == PeerEncryption::Prefer => {
                // Ownership drops the failed socket before the fresh dial.
                // Cancellation never causes a plaintext fallback.
                cx.checkpoint()?;
                trace!(
                    ?error,
                    "encrypted handshake failed; reconnecting in plaintext"
                );
                attempt(false).await?
            }
            Err(error) => return Err(error),
        };
        self.handler.on_connected(now.elapsed());

        async move {
            let write_buf = Box::new([0u8; MAX_MSG_LEN]);
            let handshake_supports_extended = h.supports_extended();
            trace!(
                peer_id=?h.peer_id,
                decoded_id=?try_decode_peer_id(h.peer_id),
                "connected",
            );
            if h.info_hash != self.info_hash {
                return Err(Error::WrongInfoHash);
            }

            if h.peer_id == self.peer_id {
                return Err(Error::ConnectingToOurselves);
            }

            self.handler.on_handshake(h, ckind).map_err(Error::Anyhow)?;
            self.handler.on_connection_flags(encrypted, false);

            self.manage_peer(
                cx,
                ManagePeerArgs {
                    handshake_supports_extended,
                    read_buf,
                    write_buf,
                    read,
                    write,
                    outgoing_chan,
                    have_broadcast,
                },
            )
            .await
        }
        .instrument(trace_span!("", kind=%ckind))
        .await
    }

    async fn manage_peer(&self, cx: &Cx, args: ManagePeerArgs) -> Result<()> {
        let ManagePeerArgs {
            handshake_supports_extended,
            read_buf,
            mut write_buf,
            mut read,
            mut write,
            mut outgoing_chan,
            mut have_broadcast,
        } = args;

        cx.checkpoint()?;
        use rqbit_runtime::io::AsyncWriteExt;

        let rwtimeout = self
            .options
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let extended_handshake: RwLock<Option<PeerExtendedMessageIds>> = RwLock::new(None);
        let extended_handshake_ref = &extended_handshake;
        let supports_extended = handshake_supports_extended;

        if supports_extended {
            let mut my_extended = ExtendedHandshake::new();
            my_extended.v = Some(ByteBuf(self.handler.client_name_and_version().as_bytes()));
            my_extended.yourip = Some(self.addr.ip().into());
            self.handler
                .update_my_extended_handshake(&mut my_extended)
                .map_err(Error::Anyhow)?;
            let my_extended = Message::Extended(ExtendedMessage::Handshake(my_extended));
            trace!("sending extended handshake: {:?}", &my_extended);
            let esz = my_extended.serialize(&mut *write_buf, &Default::default)?;
            with_timeout(
                cx,
                "writing extended handshake",
                rwtimeout,
                write.write_all(&write_buf[..esz]).map_err(Error::Write),
            )
            .await?;
        }

        let writer = async move {
            let keep_alive_interval = self
                .options
                .keep_alive_interval
                .unwrap_or_else(|| Duration::from_secs(120));

            if self.handler.should_send_bitfield() {
                let len = self
                    .handler
                    .serialize_bitfield_message_to_buf(&mut *write_buf)
                    .map_err(Error::Anyhow)?;
                with_timeout(
                    cx,
                    "writing bitfield",
                    rwtimeout,
                    write.write_all(&write_buf[..len]).map_err(Error::Write),
                )
                .await?;
                trace!("sent bitfield");
            }

            let message = if self.handler.upload_enabled() {Message::Unchoke} else {Message::Choke};
            let len = message.serialize(&mut *write_buf, &Default::default)?;
            with_timeout(
                cx,
                "writing",
                rwtimeout,
                write.write_all(&write_buf[..len]).map_err(Error::Write),
            )
            .await?;
            trace!("sent unchoke");

            let mut broadcast_closed = false;
            let mut pending = None;

            loop {
                cx.checkpoint()?;
                let req = loop {
                    cx.checkpoint()?;
                    if let Some(req) = pending.take() {
                        break req;
                    }
                    enum Next {
                        Have(
                            std::result::Result<
                                ValidPieceIndex,
                                rqbit_runtime::sync::broadcast::RecvError,
                            >,
                        ),
                        Outgoing(
                            std::result::Result<
                                std::result::Result<
                                    WriterRequest,
                                    rqbit_runtime::sync::mpsc::RecvError,
                                >,
                                rqbit_runtime::time::Elapsed,
                            >,
                        ),
                    }
                    let next = {
                        let outgoing =
                            timeout(cx, keep_alive_interval, outgoing_chan.recv(cx)).fuse();
                        let have = async {
                            if broadcast_closed {
                                futures::future::pending().await
                            } else {
                                have_broadcast.recv(cx).await
                            }
                        }
                        .fuse();
                        futures::pin_mut!(outgoing, have);
                        futures::select! { value = have => Next::Have(value), value = outgoing => Next::Outgoing(value) }
                    };
                    match next {
                        Next::Have(Ok(id)) if self.handler.should_transmit_have(id) => {
                            break WriterRequest::Message(Message::Have(id.get()));
                        }
                        Next::Have(Err(rqbit_runtime::sync::broadcast::RecvError::Closed)) => {
                            broadcast_closed = true;
                            continue;
                        }
                        Next::Have(_) => continue,
                        Next::Outgoing(Ok(Ok(msg))) => break msg,
                        Next::Outgoing(Ok(Err(_))) => return Err(Error::TorrentIsNotLive),
                        Next::Outgoing(Err(_)) => break WriterRequest::Message(Message::KeepAlive),
                    }
                };

                cx.checkpoint().map_err(|e| Error::Anyhow(e.into()))?;
                rqbit_runtime::task::yield_now().await;

                let mut uploaded_add = None;

                trace!("about to send: {:?}", &req);
                let ext_msg_ids = &|| {
                    extended_handshake_ref
                        .read()
                        .as_ref()
                        .map(|e| *e)
                        .unwrap_or_default()
                };

                let len = match req {
                    WriterRequest::UploadPolicyChanged => {
                        let enabled = self.handler.upload_enabled();
                        let bitfield = if enabled && self.handler.should_send_bitfield() {
                            self.handler.serialize_bitfield_message_to_buf(&mut *write_buf).map_err(Error::Anyhow)?
                        } else {0};
                        let message = if enabled {Message::Unchoke} else {Message::Choke};
                        bitfield + message.serialize(&mut write_buf[bitfield..], &Default::default)?
                    }
                    WriterRequest::Message(Message::Unchoke) if !self.handler.upload_enabled() => Message::Choke.serialize(&mut *write_buf, ext_msg_ids)?,
                    WriterRequest::Message(Message::Request(request)) => {
                        request_batch(request, &mut outgoing_chan, &mut pending, &mut write_buf)?
                    }
                    WriterRequest::Message(msg) => msg.serialize(&mut *write_buf, ext_msg_ids)?,
                    WriterRequest::UtMetadata(utm) => {
                        Message::Extended(ExtendedMessage::UtMetadata(utm.as_borrowed()))
                            .serialize(&mut *write_buf, ext_msg_ids)?
                    }
                    WriterRequest::UtPex(ut_pex) => {
                        Message::Extended(ExtendedMessage::UtPex(ut_pex.as_borrowed()))
                            .serialize(&mut *write_buf, ext_msg_ids)?
                    }
                    WriterRequest::ReadChunkRequest(chunk) => {
                        if !self.handler.upload_enabled() { continue; }
                        // this whole section is an optimization
                        let preamble_len = serialize_piece_preamble(&chunk, &mut *write_buf);
                        let full_len = preamble_len + chunk.size as usize;
                        write_buf = self
                            .handler
                            .read_chunk(cx, chunk, write_buf, preamble_len)
                            .await?;

                        // Permission may change while the disk read is pending.
                        // Drop queued payload rather than send a formerly allowed piece.
                        if !self.handler.upload_enabled() { continue; }

                        uploaded_add = Some(chunk.size);
                        full_len
                    }
                    WriterRequest::Disconnect(res) => {
                        trace!("disconnect requested, closing writer");
                        match res {
                            Ok(()) => return Err(Error::Disconnect),
                            Err(e) => return Err(Error::DisconnectWithSource(e)),
                        }
                    }
                };

                with_timeout(
                    cx,
                    "writing",
                    rwtimeout,
                    write.write_all(&write_buf[..len]).map_err(Error::Write),
                )
                .await?;

                if let Some(uploaded_add) = uploaded_add {
                    self.handler.on_uploaded_bytes(uploaded_add)
                }
            }

            // For type inference.
            #[allow(unreachable_code)]
            Ok::<_, Error>(())
        };

        let reader = async move {
            let mut read_buf = Some(read_buf);
            loop {
                if self.handler.received_piece_batch_domain(cx).is_some() {
                    read_buf
                        .as_mut()
                        .expect("peer reader owns its receive buffer")
                        .grow_for_piece_batch();
                }
                let message = read_buf
                    .as_mut()
                    .expect("peer reader owns its receive buffer")
                    .read_message(cx, &mut read, rwtimeout)
                    .await?;
                trace!("received: {:?}", &message);

                cx.checkpoint().map_err(|e| Error::Anyhow(e.into()))?;
                rqbit_runtime::task::yield_now().await;

                match message {
                    Message::Piece(piece) => {
                        #[cfg(feature = "chunk-diagnostics")]
                        crate::chunk_diagnostics::received_piece_frame();
                        let (index, begin, payload_len, first_len) =
                            (piece.index, piece.begin, piece.len(), piece.data().0.len());
                        // The scalar metadata ends the Piece borrow before its
                        // allocation moves. No replacement buffer is allocated.
                        let frame = read_buf
                            .take()
                            .expect("peer reader owns its receive buffer")
                            .into_piece_frame(index, begin, payload_len, first_len)?;
                        let returned = self
                            .handler
                            .on_received_owned_piece(cx, frame, &mut read)
                            .await
                            .map_err(Error::Anyhow)?;
                        read_buf = Some(returned.read_buf);
                        if let Some(error) = returned.deferred_read_error {
                            return Err(error);
                        }
                    }
                    Message::Extended(ExtendedMessage::Handshake(h)) => {
                        *extended_handshake_ref.write() = Some(h.peer_extended_messages());
                        self.handler
                            .on_extended_handshake(&h)
                            .map_err(Error::Anyhow)?;
                    }
                    message => {
                        self.handler
                            .on_received_message(cx, message)
                            .await
                            .map_err(Error::Anyhow)?;
                    }
                }
            }

            // For type inference.
            #[allow(unreachable_code)]
            Ok::<_, Error>(())
        };

        futures::pin_mut!(reader, writer);
        match futures::future::select(reader, writer).await {
            futures::future::Either::Left((result, _))
            | futures::future::Either::Right((result, _)) => result,
        }
    }
}

#[cfg(test)]
#[path = "peer_connection/upload_tests.rs"]
mod upload_tests;

#[cfg(test)]
#[path = "peer_connection/request_batch_tests.rs"]
mod request_batch_tests;
