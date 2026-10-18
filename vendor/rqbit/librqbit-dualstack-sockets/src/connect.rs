use crate::{Error, bind_device::BindDevice};
use std::net::SocketAddr;
#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectOpts<'a> {
    pub source_port: Option<u16>,
    pub bind_device: Option<&'a BindDevice>,
}
pub async fn tcp_connect(
    cx: &rqbit_runtime::Cx,
    addr: SocketAddr,
    opts: ConnectOpts<'_>,
) -> crate::Result<rqbit_runtime::net::BoundTcpStream> {
    cx.checkpoint()
        .map_err(|e| Error::Connect(std::io::Error::other(e.to_string())))?;
    rqbit_runtime::net::BoundTcpStream::connect(
        cx,
        addr,
        opts.bind_device.map(BindDevice::binding),
        opts.source_port,
    )
    .await
    .map_err(Error::Connect)
}
