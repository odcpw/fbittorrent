use rqbit_runtime::Cx;
use std::net::SocketAddr;

use anyhow::bail;
use librqbit_dualstack_sockets::BindDevice;
use librqbit_dualstack_sockets::ConnectOpts;
use serde::Serialize;
use tracing::debug;

use crate::{
    Error, PeerConnectionOptions, Result,
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
};

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ConnectionKind {
    #[serde(rename = "tcp")]
    Tcp,
    #[serde(rename = "utp")]
    Utp,
    #[serde(rename = "socks")]
    Socks,
}

impl std::fmt::Display for ConnectionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionKind::Tcp => f.write_str("tcp"),
            ConnectionKind::Utp => f.write_str("uTP"),
            ConnectionKind::Socks => f.write_str("socks"),
        }
    }
}

pub struct ConnectionOptions {
    // socks5://[username:password@]host:port
    // If set, all outgoing connections will go through the proxy over TCP.
    pub proxy_url: Option<String>,
    // TCP outgoing connections are enabled by default
    pub enable_tcp: bool,
    pub peer_opts: Option<PeerConnectionOptions>,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            enable_tcp: true,
            proxy_url: None,
            peer_opts: None,
        }
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct StreamConnectorArgs {
    pub enable_tcp: bool,
    pub bind_device: Option<BindDevice>,
    pub ipv4_only: bool,
}

gen_stats!(SingleStatAtomic SingleStatSnapshot, [
    attempts u64,
    successes u64,
    errors u64
], []);
gen_stats!(PerFamilyAtomic PerFamilySnapshot, [], [
    v4 SingleStatAtomic SingleStatSnapshot,
    v6 SingleStatAtomic SingleStatSnapshot
]);
gen_stats!(ConnectStatsAtomic ConnectStatsSnapshot, [], [
    socks PerFamilyAtomic PerFamilySnapshot,
    tcp PerFamilyAtomic PerFamilySnapshot,
    utp PerFamilyAtomic PerFamilySnapshot
]);

#[derive(Debug)]
pub(crate) struct StreamConnector {
    bind_device: Option<BindDevice>,
    stats: ConnectStatsAtomic,
    ipv4_only: bool,
}
impl StreamConnector {
    pub async fn new(cx: &Cx, config: StreamConnectorArgs) -> anyhow::Result<Self> {
        cx.checkpoint()?;
        if !config.enable_tcp {
            bail!("native peer transport requires TCP");
        }
        Ok(Self {
            bind_device: config.bind_device,
            stats: Default::default(),
            ipv4_only: config.ipv4_only,
        })
    }
    pub fn stats(&self) -> &ConnectStatsAtomic {
        &self.stats
    }
    pub async fn connect(
        &self,
        cx: &Cx,
        addr: SocketAddr,
    ) -> Result<(ConnectionKind, BoxAsyncReadVectored, BoxAsyncWrite)> {
        cx.checkpoint().map_err(|e| Error::Anyhow(e.into()))?;
        if addr.port() == 0 || (self.ipv4_only && addr.is_ipv6()) {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "invalid or disabled peer address {addr}"
            )));
        }
        let stat = if addr.is_ipv6() {
            &self.stats.tcp.v6
        } else {
            &self.stats.tcp.v4
        };
        stat.attempts(1);
        let stream = librqbit_dualstack_sockets::tcp_connect(
            cx,
            addr,
            ConnectOpts {
                source_port: None,
                bind_device: self.bind_device.as_ref(),
            },
        )
        .await
        .inspect(|_| stat.successes(1))
        .inspect_err(|_| stat.errors(1))
        .map_err(Error::TcpConnect)?;
        debug!(?addr, "connected over TCP");
        let (r, w) = stream.into_split();
        Ok((ConnectionKind::Tcp, Box::new(r), Box::new(w)))
    }
}
