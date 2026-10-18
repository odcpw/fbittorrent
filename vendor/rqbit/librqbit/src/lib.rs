//! Asupersync port of rqbit for the fbittorrent application.
//! The application owns persistence, UI and runtime lifecycle.

#![warn(clippy::cast_possible_truncation)]

macro_rules! aframe {
    ($e:expr) => {{
        #[cfg(any())]
        {
            async_backtrace::frame!($e)
        }
        #[cfg(not(any()))]
        {
            $e
        }
    }};
}

#[macro_use]
mod stat_gen;

pub mod api;
mod api_error;
mod bitv;
mod bitv_factory;
mod chunk_tracker;
#[cfg(feature = "chunk-diagnostics")]
mod chunk_diagnostics;
#[cfg(feature = "chunk-diagnostics")]
pub use chunk_diagnostics::snapshot as chunk_diagnostic_snapshot;
mod create_torrent_file;
mod dht_utils;
mod error;
pub mod file_info;
mod file_ops;
#[cfg(any())]
pub mod http_api;
#[cfg(any())]
pub mod http_api_client;
#[cfg(any(any(), any()))]
pub mod http_api_types;
mod ip_ranges;
pub mod limits;
mod listen;
mod merge_streams;
mod mse;
mod peer_connection;
mod peer_info_reader;
mod piece_tracker;
mod read_buf;
mod session;
pub mod session_stats;
pub mod spawn_utils;

pub mod storage;
mod stream_connect;
mod torrent_state;
#[cfg(any())]
pub mod tracing_subscriber_config_utils;
mod type_aliases;
#[cfg(all(any(), any()))]
pub mod upnp_server_adapter;
mod vectored_traits;
#[cfg(any())]
pub mod watch;

pub use error::{Error, Result};

pub use api_error::{ApiError, WithStatus, WithStatusError};
pub use create_torrent_file::{CreateTorrentOptions, CreateTorrentResult, create_torrent};
pub use dht;
pub use librqbit_core::spawn_utils::spawn as librqbit_spawn;
pub use listen::{ListenerMode, ListenerOptions};
pub use peer_connection::PeerConnectionOptions;
pub use mse::PeerEncryption;
pub use session::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListOnlyResponse,
    SUPPORTED_SCHEMES, Session, SessionOptions, SessionPersistenceConfig,
};
pub use stream_connect::ConnectionOptions;
pub use torrent_state::{
    ManagedTorrent, ManagedTorrentShared, ManagedTorrentState, TorrentMetadata, TorrentStats,
    TorrentStatsState,
};
pub use type_aliases::FileInfos;

pub use buffers::*;
pub use clone_to_owned::CloneToOwned;
pub use librqbit_core::magnet::*;
pub use librqbit_core::peer_id::*;
pub use librqbit_core::torrent_metainfo::*;

#[cfg(test)]
mod tests;

/// The cargo version of librqbit.
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub const fn client_name_and_version() -> &'static str {
    concat!("rqbit ", env!("CARGO_PKG_VERSION"))
}

pub fn try_increase_nofile_limit() -> anyhow::Result<u64> {
    Ok(rlimit::increase_nofile_limit(1024 * 1024)?)
}
