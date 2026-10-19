//! The existing interface guard also owns private-indexer DNS and HTTP sockets.
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

pub(super) struct Context {
    runtime: NativeRuntime,
    binding: InterfaceBinding,
}
impl Context {
    pub(super) fn configured() -> Result<Option<Self>> {
        let cfg = crate::config::Config::load()?;
        if cfg.network_interface.is_none() {
            return Ok(None);
        }
        let network = crate::network_app::state(&cfg);
        anyhow::ensure!(network.ready, "Torznab interface unavailable");
        let binding = InterfaceBinding::new(
            network
                .interface
                .as_deref()
                .context("Torznab interface unavailable")?,
        )?;
        Ok(Some(Self {
            runtime: NativeRuntime::new()?,
            binding,
        }))
    }
    pub(super) fn close(self) -> Result<()> {
        self.runtime.shutdown_timeout(Duration::from_secs(2))
    }
    pub(super) fn agent(&self, config: Config) -> ureq::Agent {
        let socket = Bound {
            runtime: self.runtime.handle(),
            binding: self.binding.clone(),
        };
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
        if host.eq_ignore_ascii_case("localhost") {
            self.binding.check_ready()?;
            let mut addresses = self.empty();
            addresses.push(std::net::SocketAddr::new(
                std::net::Ipv4Addr::LOCALHOST.into(),
                port,
            ));
            addresses.push(std::net::SocketAddr::new(
                std::net::Ipv6Addr::LOCALHOST.into(),
                port,
            ));
            return Ok(addresses);
        }
        let cx = self.runtime.cx();
        let addresses = self
            .runtime
            .block_on(rqbit_runtime::time::timeout(
                cx,
                budget(timeout),
                rqbit_runtime::net::resolve_host(cx, host, port, Some(&self.binding)),
            ))
            .map_err(|_| ureq::Error::Timeout(timeout.reason))??;
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
        let deadline = Instant::now() + budget(details.timeout);
        let mut last = io::Error::new(io::ErrorKind::NotConnected, "Torznab connect failed");
        for address in &details.addrs {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ureq::Error::Timeout(details.timeout.reason));
            }
            let cx = self.runtime.cx();
            match self.runtime.block_on(rqbit_runtime::time::timeout(
                cx,
                remaining,
                BoundTcpStream::connect(cx, *address, Some(&self.binding), None),
            )) {
                Ok(Ok(owner)) => {
                    // Keep the bound owner alive: dropping it shuts both halves.
                    // The blocking clone is used only by this worker's HTTP stack.
                    let stream = owner.try_as_std().try_clone()?;
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true)?;
                    return Ok(Some(Socket {
                        stream,
                        _owner: owner,
                        binding: self.binding.clone(),
                        buffers: LazyBuffers::new(
                            details.config.input_buffer_size(),
                            details.config.output_buffer_size(),
                        ),
                    }));
                }
                Ok(Err(error)) => last = error,
                Err(_) => return Err(ureq::Error::Timeout(details.timeout.reason)),
            }
        }
        Err(last.into())
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
