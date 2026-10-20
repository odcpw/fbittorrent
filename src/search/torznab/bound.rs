//! The existing interface guard also owns private-indexer DNS and HTTP sockets,
//! and (through `crate::outbound`) every other app-owned HTTP request.
use super::*;
use crate::native_runtime::{Handle, NativeRuntime};
use anyhow::Context as _;
use rqbit_runtime::net::{BoundTcpStream, InterfaceBinding};
use std::{fmt, io, net::TcpStream};
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};
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
        stopped: &AtomicBool,
    ) -> io::Result<(TcpStream, BoundTcpStream)> {
        let socket = self.socket();
        let deadline = Instant::now() + timeout;
        let addresses = socket.addresses(host, port, timeout, Some(stopped))?;
        socket.connect_addresses(addresses, deadline, Some(stopped))
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
    fn wait<T>(
        &self,
        timeout: Duration,
        future: impl Future<Output = io::Result<T>>,
        stopped: Option<&AtomicBool>,
    ) -> io::Result<T> {
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            let mut operation = std::pin::pin!(rqbit_runtime::time::timeout(cx, timeout, future));
            loop {
                let mut tick =
                    std::pin::pin!(rqbit_runtime::time::sleep(cx, Duration::from_millis(50)));
                let result = std::future::poll_fn(|task| {
                    if stopped.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                        return Poll::Ready(Some(Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Nova proxy stopped",
                        ))));
                    }
                    if let Poll::Ready(result) = operation.as_mut().poll(task) {
                        return Poll::Ready(Some(
                            result
                                .map_err(|_| {
                                    io::Error::new(
                                        io::ErrorKind::TimedOut,
                                        "bound request timed out",
                                    )
                                })
                                .and_then(|result| result),
                        ));
                    }
                    if stopped.is_some() && tick.as_mut().poll(task).is_ready() {
                        return Poll::Ready(None);
                    }
                    Poll::Pending
                })
                .await;
                if let Some(result) = result {
                    return result;
                }
            }
        })
    }

    fn addresses(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
        stopped: Option<&AtomicBool>,
    ) -> io::Result<Vec<std::net::SocketAddr>> {
        self.binding.check_ready()?;
        if host.eq_ignore_ascii_case("localhost") {
            return Ok(vec![
                (std::net::Ipv4Addr::LOCALHOST, port).into(),
                (std::net::Ipv6Addr::LOCALHOST, port).into(),
            ]);
        }
        let cx = self.runtime.cx();
        self.wait(
            timeout,
            rqbit_runtime::net::resolve_host(cx, host, port, Some(&self.binding)),
            stopped,
        )
    }

    fn connect_addresses(
        &self,
        addresses: impl IntoIterator<Item = std::net::SocketAddr>,
        deadline: Instant,
        stopped: Option<&AtomicBool>,
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
            match self.wait(
                remaining,
                BoundTcpStream::connect(cx, address, Some(&self.binding), None),
                stopped,
            ) {
                Ok(owner) => {
                    let stream = owner.try_as_std().try_clone()?;
                    stream.set_nonblocking(false)?;
                    stream.set_nodelay(true)?;
                    return Ok((stream, owner));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) =>
                {
                    return Err(error)
                }
                Err(error) => last = error,
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
            .addresses(host, port, budget(timeout), None)
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
                None,
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

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancelling_a_pending_bound_operation_does_not_wait_for_its_dns_budget() {
        let config = crate::config::Config {
            network_interface: Some("lo".into()),
            ..Default::default()
        };
        let context = Context::for_config(&config, "Nova VPN").unwrap().unwrap();
        let connector = context.socket();
        let stopped = std::sync::Arc::new(AtomicBool::new(false));
        let closing = stopped.clone();
        let (entered, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            connector.wait(
                Duration::from_secs(20),
                std::future::poll_fn(|_| {
                    let _ = entered.send(());
                    Poll::<io::Result<()>>::Pending
                }),
                Some(&closing),
            )
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let began = Instant::now();
        stopped.store(true, Ordering::Release);
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "job cancellation waited for DNS timeout"
        );
    }
}
