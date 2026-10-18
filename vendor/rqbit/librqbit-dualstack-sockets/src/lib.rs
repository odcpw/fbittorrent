#[cfg(test)]
mod tests;

mod bind_device;
mod connect;
mod error;
mod multicast;
pub use error::{Error, Result};

use crate::socket::MaybeDualstackSocket;

pub mod addr;
pub mod socket;

pub type TcpListener = MaybeDualstackSocket<rqbit_runtime::net::BoundTcpListener>;
pub type UdpSocket = MaybeDualstackSocket<socket::NativeUdpSocket>;
pub use bind_device::BindDevice;
pub use connect::{ConnectOpts, tcp_connect};
pub use multicast::{MulticastOpts, MulticastUdpSocket};
pub use socket::BindOpts;
