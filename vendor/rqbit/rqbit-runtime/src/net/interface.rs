use socket2::Socket;
use std::{ffi::CString, io, num::NonZeroU32};

/// An exact kernel interface identity. A replacement with the same name is a
/// new binding and requires a new session; an existing binding never follows it.
#[derive(Clone, Debug)]
pub struct InterfaceBinding {
    name: CString,
    index: NonZeroU32,
    #[cfg(target_os = "linux")]
    control: std::sync::Arc<Socket>,
}

impl InterfaceBinding {
    pub fn new(name: &str) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            if name.is_empty() || name.len() >= libc::IFNAMSIZ || name.contains('/') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid interface name",
                ));
            }
            let name = CString::new(name).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid interface name")
            })?;
            // SAFETY: name is a live, NUL-terminated C string.
            let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
            let index = NonZeroU32::new(index).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!(
                        "required network interface {} is unavailable",
                        name.to_string_lossy()
                    ),
                )
            })?;
            let control = std::sync::Arc::new(Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )?);
            let binding = Self {
                name,
                index,
                control,
            };
            binding.check_ready()?;
            Ok(binding)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = name;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "required interface binding is supported on Linux",
            ))
        }
    }

    pub fn name(&self) -> &str {
        self.name
            .to_str()
            .expect("interface was created from UTF-8")
    }
    pub fn index(&self) -> NonZeroU32 {
        self.index
    }

    pub fn check_ready(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let request = self.query(libc::SIOCGIFINDEX)?;
            // SAFETY: SIOCGIFINDEX initializes the ifru_ifindex union member.
            let index = unsafe { request.ifr_ifru.ifru_ifindex } as u32;
            if index != self.index.get() {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!(
                        "required network interface {} disappeared or changed",
                        self.name()
                    ),
                ));
            }
            let request = self.query(libc::SIOCGIFFLAGS)?;
            // SAFETY: SIOCGIFFLAGS initializes the ifru_flags union member.
            let flags = unsafe { request.ifr_ifru.ifru_flags } as u32;
            if flags & libc::IFF_UP as u32 == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("required network interface {} is down", self.name()),
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "interface binding requires Linux",
        ))
    }

    #[cfg(target_os = "linux")]
    fn query(&self, operation: libc::Ioctl) -> io::Result<libc::ifreq> {
        use std::os::fd::AsRawFd;
        // SAFETY: ifreq consists of integer/byte fields and a C union; zero is valid.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (dest, &source) in request
            .ifr_name
            .iter_mut()
            .zip(self.name.as_bytes_with_nul())
        {
            *dest = source as libc::c_char;
        }
        // This control socket performs kernel queries only. It never sends
        // datagrams and uses the current network namespace, unlike mounted sysfs.
        // SAFETY: request has the size/layout required by these named ioctls.
        if unsafe { libc::ioctl(self.control.as_raw_fd(), operation, &mut request) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!(
                    "required interface {} is unavailable: {}",
                    self.name(),
                    io::Error::last_os_error()
                ),
            ));
        }
        Ok(request)
    }

    /// Apply before bind/connect/send. Permission failures are errors, never a
    /// reason to retry on an ordinary socket.
    pub fn bind_socket(&self, socket: &Socket) -> io::Result<()> {
        self.check_ready()?;
        #[cfg(target_os = "linux")]
        {
            socket
                .bind_device(Some(self.name.as_bytes_with_nul()))
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!(
                            "cannot bind socket to required interface {}: {e}",
                            self.name()
                        ),
                    )
                })?;
            self.check_ready()
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = socket;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "interface binding requires Linux",
            ))
        }
    }
}

#[cfg(test)]
pub(crate) fn isolated_netns() -> bool {
    if std::env::var("FBT_TEST_NETNS").as_deref() != Ok("1") {
        return false;
    }
    let ours = std::fs::read_link("/proc/self/ns/net").unwrap();
    let host = std::env::var("FBT_TEST_HOST_NETNS")
        .expect("runner must capture its network namespace before unshare");
    assert_ne!(
        ours.to_string_lossy(),
        host,
        "network mutation tests must run in a disposable namespace"
    );
    println!("exercising disposable network namespace {}", ours.display());
    true
}
#[cfg(test)]
pub(crate) fn ip(args: &[&str]) {
    assert!(isolated_netns());
    let result = std::process::Command::new("ip")
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "ip {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_interface_and_replacement_are_rejected() {
        assert!(InterfaceBinding::new("fbt-no-vpn").is_err());
        assert!(InterfaceBinding::new("../lo").is_err());
        let mut binding = InterfaceBinding::new("lo").unwrap();
        binding.index = NonZeroU32::new(u32::MAX).unwrap();
        assert_eq!(
            binding.check_ready().unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
        let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
        assert!(binding.bind_socket(&socket).is_err());
        assert!(
            socket.device().unwrap().is_none(),
            "failed binding must not become an admitted socket"
        );
    }

    #[test]
    fn isolated_interface_loss_and_kernel_binding() {
        if !isolated_netns() {
            return;
        }
        ip(&["link", "set", "lo", "up"]);
        ip(&["link", "add", "fbt-test", "type", "dummy"]);
        assert!(
            InterfaceBinding::new("fbt-test").is_err(),
            "down interface is never ready"
        );
        ip(&["link", "set", "fbt-test", "up"]);
        let binding = InterfaceBinding::new("fbt-test").unwrap();
        let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
        binding.bind_socket(&socket).unwrap();
        assert_eq!(
            socket.device().unwrap().as_deref(),
            Some(b"fbt-test".as_slice())
        );
        let v6 = Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None).unwrap();
        binding.bind_socket(&v6).unwrap();
        assert_eq!(
            v6.device().unwrap().as_deref(),
            Some(b"fbt-test".as_slice())
        );
        ip(&["link", "set", "fbt-test", "down"]);
        assert!(binding.check_ready().is_err());
        assert!(binding.bind_socket(&socket).is_err());
        ip(&["link", "delete", "fbt-test"]);
        assert!(binding.check_ready().is_err());
        ip(&["link", "add", "fbt-test", "type", "dummy"]);
        ip(&["link", "set", "fbt-test", "up"]);
        assert!(
            binding.check_ready().is_err(),
            "same name must not adopt replacement identity"
        );
        assert!(InterfaceBinding::new("fbt-test").is_ok());
        ip(&["link", "delete", "fbt-test"]);
    }
}
