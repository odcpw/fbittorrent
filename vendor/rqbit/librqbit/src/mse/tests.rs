use super::*;
use std::sync::{Arc, Mutex};

struct PartialWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    pending: bool,
}
impl AsyncWrite for PartialWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.pending {
            self.pending = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.pending = true;
        let n = bytes.len().min(3);
        self.bytes.lock().unwrap().extend_from_slice(&bytes[..n]);
        Poll::Ready(Ok(n))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
#[test]
fn crypto_writer_pending_partial_and_changed_buffer_keep_cipher_position() {
    let key = [9; 20];
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let mut writer = CryptoWriter {
        inner: Box::new(PartialWriter {
            bytes: bytes.clone(),
            pending: true,
        }),
        cipher: Some(Rc4Stream::new(&key)),
        scratch: Vec::new(),
    };
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(
        Pin::new(&mut writer)
            .poll_write(&mut cx, b"cancelled")
            .is_pending()
    );
    // The cancelled poll did not commit any bytes or cipher state.
    assert!(matches!(
        Pin::new(&mut writer).poll_write(&mut cx, b"abcdef"),
        Poll::Ready(Ok(3))
    ));
    assert!(
        Pin::new(&mut writer)
            .poll_write(&mut cx, b"def")
            .is_pending()
    );
    assert!(matches!(
        Pin::new(&mut writer).poll_write(&mut cx, b"def"),
        Poll::Ready(Ok(3))
    ));
    let mut wire = bytes.lock().unwrap().clone();
    assert_ne!(wire, b"abcdef");
    Rc4Stream::new(&key).apply(&mut wire);
    assert_eq!(wire, b"abcdef");
}

struct BytesReader {
    bytes: Vec<u8>,
    offset: usize,
}
impl AsyncRead for BytesReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let n = buf.remaining().min(this.bytes.len() - this.offset);
        buf.put_slice(&this.bytes[this.offset..this.offset + n]);
        this.offset += n;
        Poll::Ready(Ok(()))
    }
}
impl AsyncReadVectored for BytesReader {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let mut read = 0;
        for buf in bufs {
            let n = buf.len().min(this.bytes.len() - this.offset);
            buf[..n].copy_from_slice(&this.bytes[this.offset..this.offset + n]);
            this.offset += n;
            read += n;
        }
        Poll::Ready(Ok(read))
    }
}
#[test]
fn crypto_reader_vectored_preserves_prefix_and_ring_wrap_cipher_order() {
    let key = [7; 20];
    let mut wire = b"abcdefghij".to_vec();
    Rc4Stream::new(&key).apply(&mut wire);
    let mut reader = CryptoReader {
        inner: Box::new(BytesReader {
            bytes: wire,
            offset: 0,
        }),
        cipher: Some(Rc4Stream::new(&key)),
        prefix: b"IA".to_vec(),
        offset: 0,
    };
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut a = [0; 3];
    let mut b = [0; 9];
    assert!(matches!(
        Pin::new(&mut reader).poll_read_vectored(
            &mut cx,
            &mut [IoSliceMut::new(&mut a), IoSliceMut::new(&mut b)]
        ),
        Poll::Ready(Ok(2))
    ));
    assert_eq!(&a[..2], b"IA");
    assert!(matches!(
        Pin::new(&mut reader).poll_read_vectored(
            &mut cx,
            &mut [IoSliceMut::new(&mut a), IoSliceMut::new(&mut b)]
        ),
        Poll::Ready(Ok(10))
    ));
    assert_eq!(&a, b"abc");
    assert_eq!(&b[..7], b"defghij");
}

#[test]
fn malformed_dh_public_keys_fail_before_negotiation() {
    for mut public in [[0u8; 96], [0xffu8; 96]] {
        let mut responder = Responder::new([5; 20], 2, true);
        assert!(matches!(
            responder.receive(&public, &[[1; 20]], &mut Random),
            Err(protocol::Error::PublicKey)
        ));
        public = [0; 96];
        public[95] = 1;
        let mut responder = Responder::new([5; 20], 2, true);
        assert!(matches!(
            responder.receive(&public, &[[1; 20]], &mut Random),
            Err(protocol::Error::PublicKey)
        ));
    }
}

#[test]
fn native_partial_handshake_timeout_drops_socket() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        let listener = rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut remote = rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let stream = listener.accept().await.unwrap().0;
        let (read, write) = stream.into_split();
        remote.write_all(&[0x42]).await.unwrap();
        let result = incoming_handshake(
            &cx,
            Box::new(read),
            Box::new(write),
            PeerEncryption::Require,
            &[],
            Duration::from_millis(60),
        )
        .await;
        assert!(result.is_err());
        let mut byte = [0; 1];
        let closed =
            rqbit_runtime::time::timeout(&cx, Duration::from_secs(1), remote.read(&mut byte))
                .await
                .unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)));
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
#[ignore = "requires the independent Python libtorrent oracle; run explicitly"]
fn libtorrent_metadata_and_payload_interoperability() {
    oracle::run();
}

#[test]
#[ignore = "requires the independent Python libtorrent oracle with one blocking pool thread; run explicitly"]
fn libtorrent_single_blocking_thread_metadata_and_payload_interoperability() {
    oracle::run_with_pool_max(1);
}
mod oracle;

#[test]
fn native_cancellation_drains_parked_encryption_handshake() {
    let runtime = rqbit_runtime::RuntimeBuilder::new()
        .worker_threads(2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let outer = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        let listener = rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut remote = rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let stream = listener.accept().await.unwrap().0;
        let (read, write) = stream.into_split();
        remote.write_all(&[0x42]).await.unwrap();
        let region = cx
            .open_child_region(rqbit_runtime::ChildRegionSpec::inherit())
            .await
            .unwrap();
        let (sender, mut receiver) = rqbit_runtime::sync::oneshot::channel();
        let task = rqbit_runtime::task::spawn_with(region.cx(), move |cx| async move {
            let mut handshake = Box::pin(incoming_handshake(
                &cx,
                Box::new(read),
                Box::new(write),
                PeerEncryption::Require,
                &[],
                Duration::from_secs(120),
            ));
            let mut parked = Some(sender);
            std::future::poll_fn(|poll| {
                let result = std::future::Future::poll(handshake.as_mut(), poll);
                if result.is_pending() {
                    if let Some(sender) = parked.take() {
                        sender.send(&cx, ()).unwrap();
                    }
                }
                result
            })
            .await
        });
        receiver.recv(&cx).await.unwrap();
        rqbit_runtime::time::timeout(&cx, Duration::from_secs(2), region.close())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(task.await, Ok(Err(_)) | Err(_)),
            "cancelled handshake must not return a stream"
        );
        let mut byte = [0; 1];
        let closed =
            rqbit_runtime::time::timeout(&cx, Duration::from_secs(1), remote.read(&mut byte))
                .await
                .unwrap();
        assert!(
            matches!(closed, Ok(0) | Err(_)),
            "cancelled socket must be closed"
        );
    });
    runtime.block_on(outer).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn malformed_encrypted_padding_is_rejected_before_allocation() {
    use sha1::{Digest, Sha1};
    fn hash(parts: &[&[u8]]) -> [u8; 20] {
        let mut hash = Sha1::new();
        for part in parts {
            hash.update(part);
        }
        hash.finalize().into()
    }
    let private = [5; 20];
    let info_hash = [0x42; 20];
    let mut responder = Responder::new([6; 20], 2, true);
    let public = protocol::dh_public(&private);
    assert!(
        responder
            .receive(&public, &[info_hash], &mut Random)
            .unwrap()
            .is_none()
    );
    let response = responder.take_outbound();
    let peer_public: [u8; 96] = response[..96].try_into().unwrap();
    let secret = protocol::dh_secret(&private, &peer_public);
    let mut request = hash(&[b"req1", &secret]).to_vec();
    let mut req2 = hash(&[b"req2", &info_hash]);
    let req3 = hash(&[b"req3", &secret]);
    for (a, b) in req2.iter_mut().zip(req3) {
        *a ^= b;
    }
    request.extend(req2);
    let mut body = vec![0; 8];
    body.extend(2u32.to_be_bytes());
    body.extend(513u16.to_be_bytes());
    Rc4Stream::new(&hash(&[b"keyA", &secret, &info_hash])).apply(&mut body);
    request.extend(body);
    assert!(matches!(
        responder.receive(&request, &[info_hash], &mut Random),
        Err(protocol::Error::Length)
    ));
}

#[test]
fn native_encrypted_stream_key_must_match_inner_bittorrent_hash() {
    let runtime = rqbit_runtime::RuntimeBuilder::new()
        .worker_threads(2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        let listener = rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let client = rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let server = listener.accept().await.unwrap().0;
        let (client_read, client_write) = client.into_split();
        let (server_read, server_write) = server.into_split();
        let mut bytes = vec![0; 68];
        let _ = Handshake::new(Id20::new([2; 20]), Id20::new([3; 20])).serialize_unchecked_len(&mut bytes);
        let outgoing = outgoing(
            &cx,
            Box::new(client_read),
            Box::new(client_write),
            Id20::new([1; 20]),
            bytes,
        );
        let hashes = [Id20::new([1; 20])];
        let incoming = incoming_handshake(
            &cx,
            Box::new(server_read),
            Box::new(server_write),
            PeerEncryption::Require,
            &hashes,
            Duration::from_secs(2),
        );
        let (out, incoming) = futures::join!(outgoing, incoming);
        assert!(out.is_ok());
        let error = incoming.err().expect("wrong inner hash must reject");
        assert!(error.to_string().contains("does not match"), "{error:#}");
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn outgoing_prefer_reconnects_and_require_never_downgrades() {
    use crate::peer_connection::{
        PeerConnection, PeerConnectionHandler, PeerConnectionOptions, WriteBuffer,
    };
    use buffers::ByteBuf;
    use librqbit_core::lengths::{ChunkInfo, ValidPieceIndex};
    use peer_binary_protocol::{Message, extended::handshake::ExtendedHandshake};
    struct Handler;
    impl PeerConnectionHandler for Handler {
        fn should_send_bitfield(&self) -> bool {
            false
        }
        fn serialize_bitfield_message_to_buf(&self, _: &mut [u8]) -> anyhow::Result<usize> {
            Ok(0)
        }
        fn on_handshake(
            &self,
            _: Handshake,
            _: crate::stream_connect::ConnectionKind,
        ) -> anyhow::Result<()> {
            bail!("fixture handshake completed")
        }
        fn on_extended_handshake(&self, _: &ExtendedHandshake<ByteBuf>) -> anyhow::Result<()> {
            Ok(())
        }
        async fn on_received_message(&self, _: &Cx, _: Message<'_>) -> anyhow::Result<()> {
            Ok(())
        }
        fn should_transmit_have(&self, _: ValidPieceIndex) -> bool {
            false
        }
        fn on_uploaded_bytes(&self, _: u32) {}
        async fn read_chunk(
            &self,
            _: &Cx,
            _: ChunkInfo,
            _: WriteBuffer,
            _: usize,
        ) -> crate::Result<WriteBuffer> {
            Err(crate::Error::ReadChunk(anyhow::anyhow!("fixture has no chunks")))
        }
    }
    for policy in [PeerEncryption::Prefer, PeerEncryption::Require] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut first, _) = listener.accept().unwrap();
            first
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut prefix = [0; 20];
            first.read_exact(&mut prefix).unwrap();
            assert_ne!(&prefix, b"\x13BitTorrent protocol");
            drop(first);
            listener.set_nonblocking(true).unwrap();
            if policy == PeerEncryption::Require {
                std::thread::sleep(Duration::from_millis(200));
                assert!(
                    matches!(listener.accept(),Err(error) if error.kind()==io::ErrorKind::WouldBlock),
                    "Require opened a plaintext fallback socket"
                );
                return;
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut second, _) = loop {
                match listener.accept() {
                    Ok(peer) => break peer,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "fallback socket deadline"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            second
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 68];
            second.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes[..20], b"\x13BitTorrent protocol");
            let mut response = [0; 68];
            let _ = Handshake::new(Id20::new([1; 20]), Id20::new([4; 20]))
                .serialize_unchecked_len(&mut response);
            second.write_all(&response).unwrap();
        });
        let runtime = rqbit_runtime::RuntimeBuilder::new()
            .worker_threads(2)
            .build()
            .unwrap();
        let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
            let connector = crate::stream_connect::StreamConnector::new(
                &cx,
                crate::stream_connect::StreamConnectorArgs {
                    enable_tcp: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let connection = PeerConnection::new(
                address,
                Id20::new([1; 20]),
                Id20::new([3; 20]),
                Handler,
                Some(PeerConnectionOptions {
                    encryption: Some(policy),
                    connect_timeout: Some(Duration::from_secs(2)),
                    read_write_timeout: Some(Duration::from_secs(2)),
                    ..Default::default()
                }),
                crate::spawn_utils::BlockingSpawner::new(1),
                Arc::new(connector),
            );
            let (_, messages) = rqbit_runtime::sync::mpsc::unbounded_channel();
            let (_, have) = rqbit_runtime::sync::broadcast::channel(1);
            let error = connection
                .manage_peer_outgoing(&cx, messages, have)
                .await
                .unwrap_err();
            if policy == PeerEncryption::Prefer {
                assert!(
                    error.to_string().contains("fixture handshake completed"),
                    "{error}"
                );
            } else {
                assert!(!error.to_string().contains("fixture handshake completed"));
            }
        });
        runtime.block_on(task).unwrap();
        server.join().unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    }
}

#[test]
fn native_encryption_policies_reject_incompatible_incoming() {
    let runtime = rqbit_runtime::RuntimeBuilder::new()
        .worker_threads(2)
        .build()
        .unwrap();
    let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, |cx| async move {
        for (policy, prefix) in [
            (PeerEncryption::Require, b"\x13BitTorrent protocol".to_vec()),
            (PeerEncryption::Off, vec![0x42; 20]),
        ] {
            let listener =
                rqbit_runtime::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                    .await
                    .unwrap();
            let mut remote = rqbit_runtime::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let stream = listener.accept().await.unwrap().0;
            let (read, write) = stream.into_split();
            remote.write_all(&prefix).await.unwrap();
            assert!(
                incoming_handshake(
                    &cx,
                    Box::new(read),
                    Box::new(write),
                    policy,
                    &[],
                    Duration::from_secs(1)
                )
                .await
                .is_err()
            );
            let mut buffer = [0; 1];
            assert!(matches!(remote.read(&mut buffer).await, Ok(0) | Err(_)));
        }
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}
