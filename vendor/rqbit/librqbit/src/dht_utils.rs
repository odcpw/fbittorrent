use rqbit_runtime::Cx;
use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use anyhow::Context;
use buffers::ByteBufOwned;
use futures::{FutureExt, Stream, StreamExt, stream::FuturesUnordered};
use librqbit_core::torrent_metainfo::TorrentMetaV1Info;
use tracing::{Instrument, debug, debug_span};

use crate::{
    peer_connection::PeerConnectionOptions, peer_info_reader, spawn_utils::BlockingSpawner,
    stream_connect::StreamConnector,
};
use librqbit_core::hash_id::Id20;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ReadMetainfoResult<Rx> {
    Found {
        info: TorrentMetaV1Info<ByteBufOwned>,
        info_bytes: ByteBufOwned,
        rx: Rx,
        seen: HashSet<SocketAddr>,
    },
    ChannelClosed {
        #[allow(dead_code)]
        seen: HashSet<SocketAddr>,
    },
}

pub async fn read_metainfo_from_peer_receiver<A: Stream<Item = SocketAddr> + Unpin>(
    cx: &Cx,
    peer_id: Id20,
    info_hash: Id20,
    initial_addrs: Vec<SocketAddr>,
    addrs_stream: A,
    peer_connection_options: Option<PeerConnectionOptions>,
    connector: Arc<StreamConnector>,
    client_name_and_version: String,
) -> ReadMetainfoResult<A> {
    let mut seen = HashSet::<SocketAddr>::new();
    let mut addrs = addrs_stream;

    let semaphore = rqbit_runtime::sync::Semaphore::new(128);

    let read_info_guarded = |addr| {
        let semaphore = &semaphore;
        let connector = connector.clone();
        let client_name_and_version = client_name_and_version.clone();
        async move {
            let token = semaphore.acquire(cx, 1).await?;
            let ret = peer_info_reader::read_metainfo_from_peer(
                cx,
                addr,
                peer_id,
                info_hash,
                peer_connection_options,
                // This shouldn't be called anyway as we aren't reading/writing to disk, so it's
                // ok not to use a shared one.
                BlockingSpawner::new(1),
                connector,
                client_name_and_version,
            )
            .instrument(debug_span!("read_metainfo_from_peer", ?addr))
            .await
            .with_context(|| format!("error reading metainfo from {addr}"));
            drop(token);
            ret
        }
    };

    let mut unordered = FuturesUnordered::new();

    for a in initial_addrs {
        seen.insert(a);
        unordered.push(read_info_guarded(a));
    }

    let mut addrs_completed = false;

    loop {
        if cx.checkpoint().is_err() || (addrs_completed && unordered.is_empty()) {
            return ReadMetainfoResult::ChannelClosed { seen };
        }

        enum Next<T> {
            Done(Option<T>),
            Addr(Option<SocketAddr>),
        }
        let next = {
            let done = async {
                if unordered.is_empty() {
                    futures::future::pending().await
                } else {
                    unordered.next().await
                }
            }
            .fuse();
            let next_addr = async {
                if addrs_completed {
                    futures::future::pending().await
                } else {
                    addrs.next().await
                }
            }
            .fuse();
            futures::pin_mut!(done, next_addr);
            futures::select! { value = done => Next::Done(value), value = next_addr => Next::Addr(value) }
        };
        match next {
            Next::Done(Some(Ok((info, info_bytes)))) => {
                return ReadMetainfoResult::Found {
                    info,
                    info_bytes,
                    seen,
                    rx: addrs,
                };
            }
            Next::Done(Some(Err(e))) => debug!("{e:#}"),
            Next::Done(None) => {}
            Next::Addr(Some(addr)) => {
                if seen.insert(addr) {
                    unordered.push(read_info_guarded(addr));
                }
            }
            Next::Addr(None) => addrs_completed = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_peer_discovery_preserves_seen_addresses() {
        let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
            .build()
            .unwrap();
        let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
            let connector = Arc::new(
                StreamConnector::new(
                    &cx,
                    crate::stream_connect::StreamConnectorArgs {
                        enable_tcp: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap(),
            );
            let result = read_metainfo_from_peer_receiver(
                &cx,
                Id20::new([1; 20]),
                Id20::new([2; 20]),
                vec![],
                futures::stream::empty(),
                None,
                connector,
                "rqbit test".into(),
            )
            .await;
            assert!(
                matches!(result, ReadMetainfoResult::ChannelClosed { seen } if seen.is_empty())
            );
        });
        runtime.block_on(task).unwrap();
    }
}
