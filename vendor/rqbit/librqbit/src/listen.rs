use std::net::{Ipv6Addr, SocketAddr};

use anyhow::Context;
use librqbit_dualstack_sockets::BindDevice;
use librqbit_dualstack_sockets::{BindOpts, TcpListener};
use rqbit_runtime::CancellationToken;
use rqbit_runtime::Cx;
use rqbit_runtime::io::AsyncWrite;
use tracing::info;

use crate::{stream_connect::ConnectionKind, vectored_traits::AsyncReadVectored};

pub(crate) struct ListenResult {
    pub tcp_socket: Option<TcpListener>,
    pub enable_upnp_port_forwarding: bool,
    pub addr: SocketAddr,
    pub announce_port: Option<u16>,
    pub max_pending_incoming_handshake_checks: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum ListenerMode {
    TcpOnly,
    UtpOnly,
    TcpAndUtp,
}

pub const DEFAULT_MAX_PENDING_INCOMING_HANDSHAKE_CHECKS: usize = 256;

impl ListenerMode {
    pub fn tcp_enabled(&self) -> bool {
        match self {
            ListenerMode::TcpOnly => true,
            ListenerMode::UtpOnly => false,
            ListenerMode::TcpAndUtp => true,
        }
    }

    pub fn utp_enabled(&self) -> bool {
        match self {
            ListenerMode::TcpOnly => false,
            ListenerMode::UtpOnly => true,
            ListenerMode::TcpAndUtp => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListenerOptions {
    pub mode: ListenerMode,
    pub listen_addr: SocketAddr,
    pub enable_upnp_port_forwarding: bool,
    pub announce_port: Option<u16>,
    pub ipv4_only: bool,
    pub max_pending_incoming_handshake_checks: usize,
}

impl Default for ListenerOptions {
    fn default() -> Self {
        Self {
            // TODO: once uTP is stable upgrade default to both
            mode: ListenerMode::TcpOnly,
            listen_addr: (Ipv6Addr::UNSPECIFIED, 0).into(),
            enable_upnp_port_forwarding: false,
            announce_port: None,
            ipv4_only: false,
            max_pending_incoming_handshake_checks: DEFAULT_MAX_PENDING_INCOMING_HANDSHAKE_CHECKS,
        }
    }
}

impl ListenerOptions {
    pub(crate) async fn start(
        self,
        cx: &Cx,
        _parent_span: Option<tracing::Id>,
        _cancellation_token: CancellationToken,
        bind_device: Option<&BindDevice>,
    ) -> anyhow::Result<ListenResult> {
        cx.checkpoint()?;
        if self.mode.utp_enabled() {
            anyhow::bail!("uTP is unavailable in the native Asupersync backend; use TCP");
        }
        let mut listen_addr = if self.ipv4_only {
            if self.listen_addr.is_ipv6() && self.listen_addr.ip().is_unspecified() {
                // Force to IPv4 unspecified if IPv6 unspecified was requested but we are v4 only
                SocketAddr::from(([0, 0, 0, 0], self.listen_addr.port()))
            } else {
                self.listen_addr
            }
        } else {
            self.listen_addr
        };

        let tcp_socket = if self.mode.tcp_enabled() {
            let listener = TcpListener::bind_tcp(
                cx,
                listen_addr,
                BindOpts {
                    request_dualstack: !self.ipv4_only,
                    reuseport: false,
                    device: bind_device,
                },
            )
            .await
            .context("error starting TCP listener")?;
            listen_addr = listener.bind_addr();
            info!(
                "Listening on TCP {:?} for incoming peer connections",
                listen_addr
            );
            Some(listener)
        } else {
            None
        };

        let announce_port = if let Some(p) = self.announce_port {
            Some(p)
        } else if listen_addr.ip().is_loopback() {
            None
        } else {
            Some(listen_addr.port())
        };
        Ok(ListenResult {
            tcp_socket,
            announce_port,
            addr: listen_addr,
            enable_upnp_port_forwarding: self.enable_upnp_port_forwarding,
            max_pending_incoming_handshake_checks: self.max_pending_incoming_handshake_checks,
        })
    }
}

pub(crate) trait Accept {
    const KIND: ConnectionKind;

    async fn accept(
        &self,
        cx: &Cx,
    ) -> anyhow::Result<(
        SocketAddr,
        (
            impl AsyncReadVectored + Send + 'static,
            impl AsyncWrite + Unpin + Send + 'static,
        ),
    )>;
}

impl Accept for TcpListener {
    const KIND: ConnectionKind = ConnectionKind::Tcp;
    async fn accept(
        &self,
        cx: &Cx,
    ) -> anyhow::Result<(
        SocketAddr,
        (
            impl AsyncReadVectored + Send + 'static,
            impl AsyncWrite + Send + 'static,
        ),
    )> {
        let (stream, addr) = self.accept(cx).await.context("error accepting TCP")?;
        let (read, write) = stream.into_split();
        Ok((addr, (read, write)))
    }
}
