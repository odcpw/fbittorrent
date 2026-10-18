use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use socket2::{Domain, Socket};
use tracing::{debug, trace};

use crate::{
    Error,
    addr::{ToV6Mapped, TryToV4},
    bind_device::BindDevice,
};

#[derive(Clone, Copy, Debug)]
pub enum SocketAddrKind {
    V4(SocketAddrV4),
    V6 {
        addr: SocketAddrV6,
        is_dualstack: bool,
    },
}

impl SocketAddrKind {
    fn is_v6(&self) -> bool {
        matches!(self, SocketAddrKind::V6 { .. })
    }

    fn as_socketaddr(&self) -> SocketAddr {
        match *self {
            SocketAddrKind::V4(addr) => SocketAddr::V4(addr),
            SocketAddrKind::V6 { addr, .. } => SocketAddr::V6(addr),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BindOpts<'a> {
    pub request_dualstack: bool,
    pub reuseport: bool,
    pub device: Option<&'a BindDevice>,
}

impl Default for BindOpts<'_> {
    fn default() -> Self {
        Self {
            request_dualstack: true,
            reuseport: false,
            device: None,
        }
    }
}

pub struct MaybeDualstackSocket<S> {
    socket: S,
    addr_kind: SocketAddrKind,
}

impl<S> MaybeDualstackSocket<S> {
    pub fn socket(&self) -> &S {
        &self.socket
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.addr_kind.as_socketaddr()
    }

    pub fn is_dualstack(&self) -> bool {
        matches!(
            self.addr_kind,
            SocketAddrKind::V6 {
                is_dualstack: true,
                ..
            }
        )
    }

    pub(crate) fn convert_addr_for_send(&self, addr: SocketAddr) -> SocketAddr {
        if self.is_dualstack() {
            return SocketAddr::V6(addr.to_ipv6_mapped());
        }
        addr
    }
}

impl MaybeDualstackSocket<Socket> {
    fn bind(addr: SocketAddr, opts: BindOpts, is_udp: bool) -> crate::Result<Self> {
        let socket = Socket::new(
            if addr.is_ipv6() {
                Domain::IPV6
            } else {
                Domain::IPV4
            },
            if is_udp {
                socket2::Type::DGRAM
            } else {
                socket2::Type::STREAM
            },
            Some(if is_udp {
                socket2::Protocol::UDP
            } else {
                socket2::Protocol::TCP
            }),
        )
        .map_err(Error::SocketNew)?;

        let mut set_dualstack = false;

        let addr_kind = match (opts.request_dualstack, addr) {
            (request_dualstack, SocketAddr::V6(addr))
                if *addr.ip() == IpAddr::V6(Ipv6Addr::UNSPECIFIED) =>
            {
                let value = !request_dualstack;
                trace!(?addr, only_v6 = value, "setting only_v6");
                socket
                    .set_only_v6(value)
                    .map_err(|e| Error::OnlyV6 { value, source: e })?;
                #[cfg(not(windows))] // socket.only_v6() panics on windows somehow
                trace!(?addr, only_v6=?socket.only_v6());
                set_dualstack = true;
                SocketAddrKind::V6 {
                    addr,
                    is_dualstack: request_dualstack,
                }
            }
            (_, SocketAddr::V6(addr)) => SocketAddrKind::V6 {
                addr,
                is_dualstack: false,
            },
            (_, SocketAddr::V4(addr)) => SocketAddrKind::V4(addr),
        };

        if !set_dualstack {
            debug!(
                ?addr,
                "ignored dualstack request as it only applies to [::] address"
            );
        }

        #[cfg(not(windows))]
        {
            socket
                .set_reuse_address(true)
                .map_err(Error::ReuseAddress)?;
        }

        #[cfg(windows)]
        if opts.reuseport || !is_udp {
            socket
                .set_reuse_address(true)
                .map_err(Error::ReuseAddress)?;
        }

        #[cfg(not(windows))]
        if opts.reuseport {
            socket.set_reuse_port(true).map_err(Error::ReusePort)?;
            debug!(reuse_port=?socket.reuse_port());
            debug!(reuse_addr=?socket.reuse_address());
        }

        if let Some(bd) = opts.device {
            bd.bind_sref(&socket, addr_kind.is_v6())?;
        }

        socket.bind(&addr.into()).map_err(|e| {
            trace!(?addr, "error binding: {e:#}");
            Error::Bind(e)
        })?;

        let local_addr: SocketAddr = socket
            .local_addr()
            .map_err(Error::LocalAddr)?
            .as_socket()
            .ok_or(Error::AsSocket)?;

        let addr_kind = match (addr_kind, local_addr) {
            (SocketAddrKind::V4(..), SocketAddr::V4(received)) => SocketAddrKind::V4(received),
            (SocketAddrKind::V6 { is_dualstack, .. }, SocketAddr::V6(received)) => {
                SocketAddrKind::V6 {
                    addr: received,
                    is_dualstack,
                }
            }
            _ => {
                tracing::debug!(?local_addr, bind_addr=?addr, "mismatch between local_addr() and requested bind_addr");
                return Err(Error::LocalBindAddrMismatch);
            }
        };

        socket
            .set_nonblocking(true)
            .map_err(Error::SetNonblocking)?;

        Ok(Self { socket, addr_kind })
    }
}

impl MaybeDualstackSocket<rqbit_runtime::net::BoundTcpListener> {
    pub async fn bind_tcp(
        cx: &rqbit_runtime::Cx,
        addr: SocketAddr,
        opts: BindOpts<'_>,
    ) -> crate::Result<Self> {
        cx.checkpoint().map_err(|e| {
            Error::Bind(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                e.to_string(),
            ))
        })?;
        let socket = MaybeDualstackSocket::bind(addr, opts, false)?;
        socket.socket.listen(1024).map_err(Error::Listen)?;
        let listener = rqbit_runtime::net::BoundTcpListener::from_std(
            cx,
            socket.socket.into(),
            opts.device.map(|device| device.binding().clone()),
        )
        .map_err(Error::NativeFromStd)?;
        Ok(Self {
            socket: listener,
            addr_kind: socket.addr_kind,
        })
    }

    pub async fn accept(
        &self,
        cx: &rqbit_runtime::Cx,
    ) -> std::io::Result<(rqbit_runtime::net::BoundTcpStream, SocketAddr)> {
        cx.checkpoint()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e.to_string()))?;
        let (s, addr) = self.socket.accept().await?;
        Ok((s, addr.try_to_ipv4()))
    }
}

/// Native Asupersync receive registration plus an owned OS handle for socket options.
/// The lock is held for one poll only, never across an await.
pub struct NativeUdpSocket {
    binding: Option<rqbit_runtime::net::InterfaceBinding>,
    raw: std::net::UdpSocket,
    receive: std::sync::Mutex<rqbit_runtime::net::UdpSocket>,
}
impl NativeUdpSocket {
    pub fn join_multicast_v4(
        &self,
        group: std::net::Ipv4Addr,
        iface: std::net::Ipv4Addr,
    ) -> std::io::Result<()> {
        self.raw.join_multicast_v4(&group, &iface)
    }
    pub fn join_multicast_v6(&self, group: &std::net::Ipv6Addr, iface: u32) -> std::io::Result<()> {
        self.raw.join_multicast_v6(group, iface)
    }
}
#[cfg(unix)]
impl std::os::fd::AsFd for NativeUdpSocket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.raw.as_fd()
    }
}
#[cfg(windows)]
impl std::os::windows::io::AsSocket for NativeUdpSocket {
    fn as_socket(&self) -> std::os::windows::io::BorrowedSocket<'_> {
        self.raw.as_socket()
    }
}

impl MaybeDualstackSocket<NativeUdpSocket> {
    pub fn bind_udp(addr: SocketAddr, opts: BindOpts<'_>) -> crate::Result<Self> {
        let sock = MaybeDualstackSocket::bind(addr, opts, true)?;
        let raw = std::net::UdpSocket::from(sock.socket);
        let receive =
            rqbit_runtime::net::UdpSocket::from_std(raw.try_clone().map_err(Error::NativeFromStd)?)
                .map_err(Error::NativeFromStd)?;
        Ok(Self {
            socket: NativeUdpSocket {
                binding: opts.device.map(|device| device.binding().clone()),
                raw,
                receive: std::sync::Mutex::new(receive),
            },
            addr_kind: sock.addr_kind,
        })
    }
    pub async fn recv_from(
        &self,
        cx: &rqbit_runtime::Cx,
        buf: &mut [u8],
    ) -> std::io::Result<(usize, SocketAddr)> {
        cx.checkpoint()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e.to_string()))?;
        let (size, addr) = std::future::poll_fn(|task| {
            cx.checkpoint()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e.to_string()))?;
            if let Some(binding) = &self.socket.binding {
                binding.check_ready()?;
            }
            self.socket
                .receive
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .poll_recv_from(task, buf)
        })
        .await?;
        Ok((size, addr.try_to_ipv4()))
    }
    pub async fn send_to(
        &self,
        cx: &rqbit_runtime::Cx,
        buf: &[u8],
        target: SocketAddr,
    ) -> std::io::Result<usize> {
        cx.checkpoint()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e.to_string()))?;
        let mut sender = rqbit_runtime::net::UdpSocket::from_std(self.socket.raw.try_clone()?)?;
        let mut send = std::pin::pin!(sender.send_to(buf, self.convert_addr_for_send(target)));
        std::future::poll_fn(|task| {
            cx.checkpoint()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Interrupted, e.to_string()))?;
            if let Some(binding) = &self.socket.binding {
                binding.check_ready()?;
            }
            std::future::Future::poll(send.as_mut(), task)
        })
        .await
    }

    pub async fn resolve_host(
        &self,
        cx: &rqbit_runtime::Cx,
        host: &str,
        port: u16,
    ) -> std::io::Result<Vec<SocketAddr>> {
        rqbit_runtime::net::resolve_host(cx, host, port, self.socket.binding.as_ref()).await
    }
}
