//! Async networking primitives.
//!
//! Phase 0 exposes synchronous std::net wrappers through async-looking APIs.
//! This keeps the public surface stable while the runtime lacks a reactor.

#![allow(clippy::unused_async)]

/// DNS resolution with caching and Happy Eyeballs support.
pub mod dns;
/// Happy Eyeballs v2 (RFC 8305) concurrent dual-stack connection racing.
pub mod happy_eyeballs;
/// STUN protocol for NAT traversal and ICE candidate gathering.
#[cfg(not(target_arch = "wasm32"))]
pub mod stun;
mod resolve;
pub mod sys;
/// TCP networking primitives.
///
/// Browser/wasm builds keep the type surface available for API compatibility,
/// but native socket entry points fail fast with `io::ErrorKind::Unsupported`.
pub mod tcp;
mod udp;
/// Unix domain socket networking primitives (includes `UnixListener`, `UnixStream`).
#[cfg(unix)]
pub mod unix;
/// WebSocket protocol implementation (RFC 6455).
pub mod websocket;
/// MessagePort-based coordination utilities for browser worker runtimes.
pub mod worker_channel;

pub use happy_eyeballs::{HappyEyeballsConfig, connect as happy_eyeballs_connect};
pub use resolve::{lookup_all, lookup_one};
#[cfg(not(target_arch = "wasm32"))]
pub use stun::{IceCandidate, IceCandidateType, StunClient, StunError, StunMessageType};
#[cfg(target_os = "windows")]
pub use sys::windows::{NamedPipeClient, NamedPipeClientOptions};
pub use tcp::listener::{Incoming, TcpListener};
pub use tcp::socket::TcpSocket;
pub use tcp::split::{OwnedReadHalf, OwnedWriteHalf, ReadHalf, ReuniteError, WriteHalf};
pub use tcp::stream::TcpStream;
pub use tcp::stream::TcpStreamBuilder;
/// Opt-in fallback-driver diagnostics for native integration tests.
#[cfg(all(not(target_arch = "wasm32"), feature = "test-internals"))]
pub use udp::{FallbackIoDriverProbe, fallback_io_driver_probe};
pub use udp::{
    RecvStream, SendSink, UDP_DEFAULT_GSO_SEGMENT_BYTES, UDP_MAX_GSO_SEGMENTS,
    UDP_MAX_SENDMMSG_BATCH, UDP_RENDEZVOUS_MAX_ATTEMPTS, UDP_RENDEZVOUS_MAX_CANDIDATES,
    UDP_RENDEZVOUS_MAX_ID_BYTES, UDP_RENDEZVOUS_NONCE_BYTES, UdpAddressFamily,
    UdpBatchCapabilities, UdpBatchIoReport, UdpBufferConfig, UdpBufferTuneReport, UdpCapability,
    UdpEndpointObservation, UdpHairpinSupport, UdpInboundDatagram, UdpNatAssessment,
    UdpNatConfidence, UdpNatKind, UdpOutboundDatagram, UdpPlatform, UdpRecvBatch,
    UdpRendezvousCandidate, UdpRendezvousCandidateKind, UdpRendezvousCandidateSet,
    UdpRendezvousSignature, UdpRendezvousValidationError, UdpSendAccelerationCapabilities,
    UdpSendBatchPath, UdpSendBatchPlan, UdpSendBatchStrategy, UdpSocket, UdpSocketCapabilities,
    classify_udp_nat, validate_udp_rendezvous_candidates,
};
#[cfg(unix)]
pub use unix::{
    Incoming as UnixIncoming, OwnedReadHalf as UnixOwnedReadHalf,
    OwnedWriteHalf as UnixOwnedWriteHalf, ReadHalf as UnixReadHalf,
    ReuniteError as UnixReuniteError, UnixListener, UnixStream, WriteHalf as UnixWriteHalf,
};
pub use websocket::{
    ClientHandshake, CloseCode, Frame, FrameCodec, HandshakeError, Message, Opcode, Role as WsRole,
    ServerHandshake, ServerWebSocket, WebSocket, WebSocketAcceptor, WebSocketConfig, WebSocketRead,
    WebSocketWrite, WsAcceptError, WsConnectError, WsError, WsReuniteError, WsUrl, apply_mask,
};
