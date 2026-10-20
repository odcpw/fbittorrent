//! The existing interface guard also owns private-indexer DNS and HTTP sockets,
//! and (through `crate::outbound`) every other app-owned HTTP request.
use super::*;
use crate::native_runtime::{Handle, NativeRuntime};
use anyhow::Context as _;
use rqbit_runtime::net::{BoundTcpStream, InterfaceBinding};
use std::{fmt, io, net::TcpStream};
use ureq::{
    config::Config,
    http::Uri,
    unversioned::{
        resolver::{ResolvedSocketAddrs, Resolver},
        transport::{
            Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, RustlsConnector,
            Transport,
        },
    },
};

pub(crate) struct Context {
    runtime: NativeRuntime,
    binding: InterfaceBinding,
}
impl Context {
    pub(crate) fn check_ready(&self) -> io::Result<()> {
        self.binding.check_ready()
    }

    /// Nova's local proxy uses exactly the HTTP transport's bound DNS/connect
    /// path. Keep the returned owner alive for the lifetime of its blocking fd.
    pub(crate) fn connect(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> io::Result<(TcpStream, BoundTcpStream)> {
        let socket = self.socket();
        let deadline = Instant::now() + timeout;
        let addresses = socket.addresses(host, port, timeout)?;
        socket.connect_addresses(addresses, deadline)
    }

    fn socket(&self) -> Bound {
        Bound {
            runtime: self.runtime.handle(),
            binding: self.binding.clone(),
        }
    }
    pub(super) fn configured() -> Result<Option<Self>> {
        Self::for_config(&crate::config::Config::load()?, "Torznab")
    }
    /// None without a VPN binding; an error (never an unbound fallback) when
    /// the configured interface is unavailable.
    pub(crate) fn for_config(cfg: &crate::config::Config, what: &str) -> Result<Option<Self>> {
        if cfg.network_interface.is_none() {
            return Ok(None);
        }
        let network = crate::network_app::state(cfg);
        anyhow::ensure!(
            network.ready,
            "{what} interface unavailable: {}",
            network.message
        );
        let binding = InterfaceBinding::new(
            network
                .interface
                .as_deref()
                .with_context(|| format!("{what} interface unavailable"))?,
        )?;
        Ok(Some(Self {
            runtime: NativeRuntime::new()?,
            binding,
        }))
    }
    pub(crate) fn close(self) -> Result<()> {
        self.runtime.shutdown_timeout(Duration::from_secs(2))
    }
    pub(crate) fn agent(&self, config: Config) -> ureq::Agent {
        let socket = self.socket();
        let resolver = socket.clone();
        ureq::Agent::with_parts(
            config,
            ().chain(socket).chain(RustlsConnector::default()),
            resolver,
        )
    }
}
#[derive(Clone)]
struct Bound {
    runtime: Handle,
    binding: InterfaceBinding,
}
impl Bound {
    fn addresses(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> io::Result<Vec<std::net::SocketAddr>> {
        self.binding.check_ready()?;
        if host.eq_ignore_ascii_case("localhost") {
            return Ok(vec![
                (std::net::Ipv4Addr::LOCALHOST, port).into(),
                (std::net::Ipv6Addr::LOCALHOST, port).into(),
            ]);
        }
        let cx = self.runtime.cx();
        self.runtime
            .block_on(rqbit_runtime::time::timeout(
                cx,
                timeout,
                rqbit_runtime::net::resolve_host(cx, host, port, Some(&self.binding)),
            ))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "bound DNS timed out"))?
    }

    fn connect_addresses(
        &self,
        addresses: impl IntoIterator<Item = std::net::SocketAddr>,
        deadline: Instant,
    ) -> io::Result<(TcpStream, BoundTcpStream)> {
        let mut last = io::Error::new(io::ErrorKind::NotConnected, "bound connect failed");
        for address in addresses.into_iter().take(16) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "bound connect timed out",
                ));
            }
            let cx = self.runtime.cx();
            match self.runtime.block_on(rqbit_runtime::time::timeout(
                cx,
                remaining,
                BoundTcpStream::connect(cx, address, Some(&self.binding), None),
            )) {
                Ok(Ok(owner)) => {
                    let stream = owner.try_as_std().try_clone()?;
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true)?;
                    return Ok((stream, owner));
                }
                Ok(Err(error)) => last = error,
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "bound connect timed out",
                    ))
                }
            }
        }
        Err(last)
    }
}
impl fmt::Debug for Bound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Torznab bound transport")
    }
}
fn budget(timeout: NextTimeout) -> Duration {
    timeout
        .not_zero()
        .map(|time| *time)
        .unwrap_or(Duration::from_secs(30))
}
impl Resolver for Bound {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &Config,
        timeout: NextTimeout,
    ) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().ok_or(ureq::Error::HostNotFound)?;
        let port = uri
            .port_u16()
            .unwrap_or(if uri.scheme_str() == Some("https") {
                443
            } else {
                80
            });
        let addresses = self
            .addresses(host, port, budget(timeout))
            .map_err(|error| failure(error, timeout))?;
        let mut result = self.empty();
        for address in addresses.into_iter().take(16) {
            result.push(address);
        }
        if result.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(result)
    }
}
impl Connector for Bound {
    type Out = Socket;
    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<()>,
    ) -> std::result::Result<Option<Self::Out>, ureq::Error> {
        let (stream, owner) = self
            .connect_addresses(
                details.addrs.iter().copied(),
                Instant::now() + budget(details.timeout),
            )
            .map_err(|error| failure(error, details.timeout))?;
        Ok(Some(Socket {
            stream,
            _owner: owner,
            binding: self.binding.clone(),
            buffers: LazyBuffers::new(
                details.config.input_buffer_size(),
                details.config.output_buffer_size(),
            ),
        }))
    }
}
struct Socket {
    stream: TcpStream,
    _owner: BoundTcpStream,
    binding: InterfaceBinding,
    buffers: LazyBuffers,
}
impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Torznab bound socket")
    }
}
fn failure(error: io::Error, timeout: NextTimeout) -> ureq::Error {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        ureq::Error::Timeout(timeout.reason)
    } else {
        error.into()
    }
}
impl Transport for Socket {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }
    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: NextTimeout,
    ) -> std::result::Result<(), ureq::Error> {
        self.binding.check_ready()?;
        self.stream.set_write_timeout(Some(budget(timeout)))?;
        self.stream
            .write_all(&self.buffers.output()[..amount])
            .map_err(|e| failure(e, timeout))?;
        self.binding.check_ready()?;
        Ok(())
    }
    fn await_input(&mut self, timeout: NextTimeout) -> std::result::Result<bool, ureq::Error> {
        self.binding.check_ready()?;
        self.stream.set_read_timeout(Some(budget(timeout)))?;
        let count = self
            .stream
            .read(self.buffers.input_append_buf())
            .map_err(|e| failure(e, timeout))?;
        self.buffers.input_appended(count);
        self.binding.check_ready()?;
        Ok(count > 0)
    }
    fn is_open(&mut self) -> bool {
        if self.binding.check_ready().is_err() || self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let result = self.stream.peek(&mut [0]);
        let restored = self.stream.set_nonblocking(false).is_ok();
        restored
            && match result {
                Ok(0) => false,
                Ok(_) => true,
                Err(e) => e.kind() == io::ErrorKind::WouldBlock,
            }
    }
}
