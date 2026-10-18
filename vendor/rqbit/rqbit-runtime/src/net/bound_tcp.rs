use super::InterfaceBinding;
use crate::{
    io::{AsyncRead, AsyncReadVectored, AsyncWrite, ReadBuf},
    Cx,
};
use asupersync::runtime::{io_driver::IoRegistration, reactor::Interest};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::{self, IoSlice, IoSliceMut, Read, Write},
    net::{Shutdown, SocketAddr},
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};

fn checkpoint(cx: &Cx) -> io::Result<()> {
    cx.checkpoint()
        .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
    if let Some(current) = Cx::current() {
        current
            .checkpoint()
            .map_err(|error| io::Error::new(io::ErrorKind::Interrupted, error))?;
    }
    Ok(())
}

struct SocketIo {
    // Registration is dropped before the OS handle.
    registration: Option<IoRegistration>,
    socket: std::net::TcpStream,
    owner: Cx,
    binding: Option<InterfaceBinding>,
    shutdown: Shutdown,
}
impl SocketIo {
    fn check(&self) -> io::Result<()> {
        let result = checkpoint(&self.owner).and_then(|_| {
            self.binding
                .as_ref()
                .map_or(Ok(()), InterfaceBinding::check_ready)
        });
        if result.is_err() {
            let _ = self.socket.shutdown(Shutdown::Both);
        }
        result
    }
    fn arm(&mut self, task: &Context<'_>, interest: Interest) -> io::Result<()> {
        #[cfg(unix)]
        {
            if let Some(registration) = &mut self.registration {
                if registration.rearm(interest, task.waker())? {
                    return Ok(());
                }
                self.registration = None;
            }
            let mut registration = self.owner.register_io(&self.socket, interest)?;
            registration.rearm(interest, task.waker())?;
            self.registration = Some(registration);
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (task, interest);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native configured TCP requires Unix",
            ))
        }
    }
    fn poll_read_vectored(
        &mut self,
        task: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        self.check()?;
        match (&self.socket).read_vectored(bufs) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.arm(task, Interest::READABLE)?;
                // Retry after arming to close the readiness registration race.
                match (&self.socket).read_vectored(bufs) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
                    result => Poll::Ready(result),
                }
            }
            result => Poll::Ready(result),
        }
    }
    fn poll_write_vectored(
        &mut self,
        task: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.check()?;
        match (&self.socket).write_vectored(bufs) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.arm(task, Interest::WRITABLE)?;
                match (&self.socket).write_vectored(bufs) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
                    result => Poll::Ready(result),
                }
            }
            result => Poll::Ready(result),
        }
    }
}
impl Drop for SocketIo {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(self.shutdown);
    }
}

/// A configured OS socket registered with the owning Asupersync reactor.
/// Interface enforcement precedes the first SYN and applies to both halves.
pub struct BoundTcpStream {
    read: BoundReadHalf,
    write: BoundWriteHalf,
}
pub struct BoundReadHalf(SocketIo);
pub struct BoundWriteHalf(SocketIo);

impl BoundTcpStream {
    pub fn from_std(
        cx: &Cx,
        stream: std::net::TcpStream,
        binding: Option<InterfaceBinding>,
    ) -> io::Result<Self> {
        checkpoint(cx)?;
        if let Some(binding) = &binding {
            binding.check_ready()?;
        }
        stream.set_nonblocking(true)?;
        let read = stream.try_clone()?;
        Ok(Self {
            read: BoundReadHalf(SocketIo {
                registration: None,
                socket: read,
                owner: cx.clone(),
                binding: binding.clone(),
                shutdown: Shutdown::Read,
            }),
            write: BoundWriteHalf(SocketIo {
                registration: None,
                socket: stream,
                owner: cx.clone(),
                binding,
                shutdown: Shutdown::Write,
            }),
        })
    }
    pub async fn connect(
        cx: &Cx,
        addr: SocketAddr,
        binding: Option<&InterfaceBinding>,
        source_port: Option<u16>,
    ) -> io::Result<Self> {
        checkpoint(cx)?;
        let socket = Socket::new(
            if addr.is_ipv4() {
                Domain::IPV4
            } else {
                Domain::IPV6
            },
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        if let Some(binding) = binding {
            binding.bind_socket(&socket)?;
        }
        socket.set_nonblocking(true)?;
        if let Some(port) = source_port {
            let local = if addr.is_ipv4() {
                SocketAddr::from(([0, 0, 0, 0], port))
            } else {
                SocketAddr::from(([0u16; 8], port))
            };
            socket.bind(&local.into())?;
        }
        match socket.connect(&addr.into()) {
            Ok(()) => {}
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.raw_os_error() == Some(libc::EINPROGRESS) => {}
            Err(e) => return Err(e),
        }
        let mut stream = Self::from_std(cx, socket.into(), binding.cloned())?;
        std::future::poll_fn(|task| {
            stream.write.0.check()?;
            stream.write.0.arm(task, Interest::WRITABLE)?;
            if let Some(error) = stream.write.0.socket.take_error()? {
                return Poll::Ready(Err(error));
            }
            match stream.write.0.socket.peer_addr() {
                Ok(_) => Poll::Ready(Ok(())),
                Err(e) if e.kind() == io::ErrorKind::NotConnected => Poll::Pending,
                Err(e) => Poll::Ready(Err(e)),
            }
        })
        .await?;
        stream.write.0.registration = None;
        Ok(stream)
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.write.0.socket.local_addr()
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.write.0.socket.peer_addr()
    }
    pub fn into_split(self) -> (BoundReadHalf, BoundWriteHalf) {
        (self.read, self.write)
    }
    pub fn try_as_std(&self) -> &std::net::TcpStream {
        &self.write.0.socket
    }
}

impl AsyncRead for BoundReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let size = std::task::ready!(self
            .0
            .poll_read_vectored(task, &mut [IoSliceMut::new(buf.unfilled())]))?;
        buf.advance(size);
        Poll::Ready(Ok(()))
    }
}
impl AsyncReadVectored for BoundReadHalf {
    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        self.0.poll_read_vectored(task, bufs)
    }
}
impl AsyncWrite for BoundWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.poll_write_vectored(task, &[IoSlice::new(buf)])
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.0.poll_write_vectored(task, bufs)
    }
    fn poll_flush(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.0.check())
    }
    fn poll_shutdown(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.0.socket.shutdown(Shutdown::Write))
    }
}
impl AsyncRead for BoundTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.read).poll_read(task, buf)
    }
}
impl AsyncWrite for BoundTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(task, buf)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        task: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write_vectored(task, bufs)
    }
    fn poll_flush(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(task)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(task)
    }
}

pub struct BoundTcpListener {
    registration: Mutex<Option<IoRegistration>>,
    socket: std::net::TcpListener,
    owner: Cx,
    binding: Option<InterfaceBinding>,
}
impl BoundTcpListener {
    pub fn from_std(
        cx: &Cx,
        socket: std::net::TcpListener,
        binding: Option<InterfaceBinding>,
    ) -> io::Result<Self> {
        checkpoint(cx)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            registration: Mutex::new(None),
            socket,
            owner: cx.clone(),
            binding,
        })
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    pub async fn accept(&self) -> io::Result<(BoundTcpStream, SocketAddr)> {
        std::future::poll_fn(|task| {
            checkpoint(&self.owner)?;
            if let Some(binding) = &self.binding {
                binding.check_ready()?;
            }
            let mut registration = self.registration.lock().unwrap_or_else(|e| e.into_inner());
            match self.socket.accept() {
                Ok((stream, addr)) => Poll::Ready(
                    BoundTcpStream::from_std(&self.owner, stream, self.binding.clone())
                        .map(|stream| (stream, addr)),
                ),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    #[cfg(unix)]
                    {
                        if registration.as_mut().is_none_or(|r| !r.is_active()) {
                            *registration =
                                Some(self.owner.register_io(&self.socket, Interest::READABLE)?);
                        }
                        registration
                            .as_mut()
                            .unwrap()
                            .rearm(Interest::READABLE, task.waker())?;
                        match self.socket.accept() {
                            Ok((stream, addr)) => Poll::Ready(
                                BoundTcpStream::from_std(&self.owner, stream, self.binding.clone())
                                    .map(|stream| (stream, addr)),
                            ),
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
                            Err(e) => Poll::Ready(Err(e)),
                        }
                    }
                    #[cfg(not(unix))]
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "native configured TCP requires Unix",
                    )))
                }
                Err(e) => Poll::Ready(Err(e)),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        io::{AsyncReadExt, AsyncWriteExt},
        Budget, RuntimeBuilder,
    };
    use std::time::Duration;

    #[test]
    fn parked_native_tcp_read_cancels_and_releases_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let owner = runtime.request_cx_with_budget(Budget::INFINITE);
        let connect = crate::task::spawn_with(&owner, move |cx| async move {
            BoundTcpStream::connect(&cx, address, None, None)
                .await
                .unwrap()
        });
        let mut stream = runtime.block_on(connect).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let (ready, mut observed) = crate::sync::oneshot::channel();
        let read = crate::task::spawn_with(&owner, move |cx| async move {
            ready.send(&cx, ()).unwrap();
            let mut buffer = [0; 4];
            stream.read_exact(&mut buffer).await
        });
        runtime.block_on(observed.recv(&owner)).unwrap();
        read.abort();
        let error = runtime
            .block_on(read)
            .expect("native acknowledged I/O result retained")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        let mut bytes = [0; 4];
        assert_eq!(
            peer.read(&mut bytes).unwrap(),
            0,
            "cancelled owned stream closes its connection"
        );
        assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
    }
    #[test]
    fn isolated_tcp_binding_loss_and_wrong_device() {
        if !super::super::interface::isolated_netns() {
            return;
        }
        let ip = super::super::interface::ip;
        ip(&["link", "set", "lo", "up"]);
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let owner = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = crate::task::spawn_with(&owner, move |cx| async move {
            let binding = InterfaceBinding::new("lo").unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let mut client = BoundTcpStream::connect(&cx, addr, Some(&binding), None)
                .await
                .unwrap();
            assert_eq!(
                socket2::SockRef::from(client.try_as_std())
                    .device()
                    .unwrap()
                    .as_deref(),
                Some(b"lo".as_slice())
            );
            let (server, _) = listener.accept().unwrap();
            let server = BoundTcpStream::from_std(&cx, server, Some(binding.clone())).unwrap();
            let (mut read, mut write) = server.into_split();
            client.write_all(b"before").await.unwrap();
            let mut bytes = [0; 6];
            read.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"before");
            ip(&["link", "set", "lo", "down"]);
            assert!(client.write_all(b"after").await.is_err());
            assert!(write.write_all(b"after").await.is_err());
            let mut slices = [IoSliceMut::new(&mut bytes)];
            assert!(std::future::poll_fn(
                |task| Pin::new(&mut read).poll_read_vectored(task, &mut slices)
            )
            .await
            .is_err());
            assert!(BoundTcpStream::connect(&cx, addr, Some(&binding), None)
                .await
                .is_err());
            ip(&["link", "set", "lo", "up"]);
            ip(&["link", "add", "fbt-wrong", "type", "dummy"]);
            ip(&["link", "set", "fbt-wrong", "up"]);
            let wrong = InterfaceBinding::new("fbt-wrong").unwrap();
            let wrong_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            wrong_listener.set_nonblocking(true).unwrap();
            let addr = wrong_listener.local_addr().unwrap();
            let result = crate::time::timeout(
                &cx,
                Duration::from_millis(100),
                BoundTcpStream::connect(&cx, addr, Some(&wrong), None),
            )
            .await;
            assert!(
                !matches!(result, Ok(Ok(_))),
                "wrong interface must not fall back to loopback"
            );
            assert_eq!(
                wrong_listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            ip(&["link", "delete", "fbt-wrong"]);
        });
        runtime.block_on(task).unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
    }

    #[test]
    #[ignore = "explicit native throughput measurement in a disposable network namespace"]
    fn isolated_binding_throughput_measurement() {
        assert!(super::super::interface::isolated_netns());
        super::super::interface::ip(&["link", "set", "lo", "up"]);
        fn cpu_ns() -> u128 {
            let mut time = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: time points to an initialized timespec of the expected size.
            assert_eq!(
                unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) },
                0
            );
            time.tv_sec as u128 * 1_000_000_000 + time.tv_nsec as u128
        }
        let runtime = RuntimeBuilder::new().worker_threads(2).build().unwrap();
        let owner = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = crate::task::spawn_with(&owner, move |cx| async move {
            const BYTES: usize = 32 * 1024 * 1024;
            for bound in [false, true, false, true] {
                let binding = bound.then(|| InterfaceBinding::new("lo").unwrap());
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client = BoundTcpStream::connect(
                    &cx,
                    listener.local_addr().unwrap(),
                    binding.as_ref(),
                    None,
                )
                .await
                .unwrap();
                let (server, _) = listener.accept().unwrap();
                let mut server = BoundTcpStream::from_std(&cx, server, binding.clone()).unwrap();
                let started = std::time::Instant::now();
                let cpu = cpu_ns();
                let receiver = crate::task::spawn_with(&cx, move |_| async move {
                    let mut buffer = [0; 16 * 1024];
                    for _ in 0..BYTES / buffer.len() {
                        server.read_exact(&mut buffer).await.unwrap();
                        assert!(buffer.iter().all(|&byte| byte == 0x5a));
                    }
                    BYTES
                });
                let buffer = [0x5a; 16 * 1024];
                for _ in 0..BYTES / buffer.len() {
                    client.write_all(&buffer).await.unwrap();
                }
                assert_eq!(receiver.await.unwrap(), BYTES);
                println!(
                    "binding_measurement bound={bound} bytes={BYTES} wall_ms={:.3} cpu_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.0,
                    (cpu_ns() - cpu) as f64 / 1_000_000.0
                );
            }
        });
        runtime.block_on(task).unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
    }
}
