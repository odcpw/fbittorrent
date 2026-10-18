//! Small sd_notify transport; this is independent of desktop notifications.
use std::ffi::OsStr;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};

pub struct Notify(Option<SocketAddr>);

impl Notify {
    pub fn from_env() -> io::Result<Self> {
        Self::new(std::env::var_os("NOTIFY_SOCKET").as_deref())
    }

    fn new(path: Option<&OsStr>) -> io::Result<Self> {
        Ok(Self(
            path.map(|path| {
                let bytes = path.as_bytes();
                if bytes.first() == Some(&b'@') {
                    SocketAddr::from_abstract_name(&bytes[1..])
                } else {
                    SocketAddr::from_pathname(path)
                }
            })
            .transpose()?,
        ))
    }

    pub fn send(&self, text: &str) -> io::Result<()> {
        if let Some(address) = &self.0 {
            let socket = UnixDatagram::unbound()?;
            socket.set_write_timeout(Some(std::time::Duration::from_millis(200)))?;
            socket.send_to_addr(text.as_bytes(), address)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn datagram_text() {
        let path = std::env::temp_dir().join(format!("fbt-notify-{}.sock", std::process::id()));
        let socket = UnixDatagram::bind(&path).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let notify = Notify::new(Some(path.as_os_str())).unwrap();
        for expected in ["READY=1", "STATUS=Waiting for fbmissing0", "STOPPING=1"] {
            notify.send(expected).unwrap();
            let mut bytes = [0; 256];
            let len = socket.recv(&mut bytes).unwrap();
            println!("notify expected={expected:?} observed={:?}", &bytes[..len]);
            assert_eq!(&bytes[..len], expected.as_bytes());
        }
        Notify::new(None).unwrap().send("READY=1").unwrap();
        std::fs::remove_file(path).unwrap();
        let name = format!("fbt-notify-{}", std::process::id());
        let socket =
            UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(name.as_bytes()).unwrap())
                .unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        Notify::new(Some(OsStr::new(&format!("@{name}"))))
            .unwrap()
            .send("READY=1")
            .unwrap();
        let mut bytes = [0; 7];
        assert_eq!(socket.recv(&mut bytes).unwrap(), 7);
        assert_eq!(&bytes, b"READY=1");
    }
}
