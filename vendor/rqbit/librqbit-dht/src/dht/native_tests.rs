use super::*;
use librqbit_core::compact_ip::Compact;
use rqbit_runtime::{Budget, RuntimeBuilder};
use std::{future::Future, net::Ipv4Addr};

#[test]
fn native_dht_bootstrap_and_get_peers_use_udp() {
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
        let region = cx
            .open_child_region(rqbit_runtime::cx::ChildRegionSpec::inherit())
            .await
            .unwrap();
        let cx = region.cx().clone();
        let seed =
            UdpSocket::bind_udp((Ipv4Addr::LOCALHOST, 0).into(), Default::default()).unwrap();
        let seed_addr = seed.bind_addr();
        let hash = Id20::new([3; 20]);
        let peer = SocketAddr::from((Ipv4Addr::LOCALHOST, 4567));
        let (discovered, mut observed) = rqbit_runtime::sync::oneshot::channel();
        let seed_task = rqbit_runtime::task::spawn_with(&cx, move |cx| async move {
            let mut discovered = Some(discovered);
            let mut buffer = [0; 2048];
            loop {
                let (size, source) = match seed.recv_from(&cx, &mut buffer).await {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return,
                    Err(error) => panic!("DHT seed receive failed: {error}"),
                };
                let message =
                    bprotocol::deserialize_message::<ByteBufOwned>(&buffer[..size]).unwrap();
                let values = match message.kind {
                    MessageKind::GetPeersRequest(request) => {
                        assert_eq!(request.info_hash, hash);
                        if let Some(tx) = discovered.take() {
                            tx.send(&cx, ()).unwrap();
                        }
                        Some(vec![Compact(peer)])
                    }
                    MessageKind::FindNodeRequest(_) | MessageKind::PingRequest(_) => None,
                    other => panic!("unexpected DHT request {other:?}"),
                };
                let response = Response::<ByteBufOwned> {
                    id: Id20::new([4; 20]),
                    values,
                    nodes: Some(CompactNodeInfo::new_from_iter(std::iter::once(Node {
                        id: Id20::new([4; 20]),
                        addr: match seed_addr {
                            SocketAddr::V4(addr) => addr,
                            _ => unreachable!(),
                        },
                    }))),
                    token: Some(ByteBufOwned::from(b"seed".as_slice())),
                    ..Default::default()
                };
                let mut bytes = Vec::new();
                bprotocol::serialize_message(
                    &mut bytes,
                    message.transaction_id,
                    None,
                    None,
                    MessageKind::Response(response),
                )
                .unwrap();
                seed.send_to(&cx, &bytes, source).await.unwrap();
            }
        });
        let dht = DhtState::with_config(
            &cx,
            DhtConfig {
                listen_addr: Some((Ipv4Addr::LOCALHOST, 0).into()),
                bootstrap_addrs: Some(vec![seed_addr.to_string()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // A bootstrap response puts the seed in the routing table. get_peers
        // starts immediately and retries roots after one second if needed.
        let mut peers = dht.get_peers(&cx, hash, None);
        let found = rqbit_runtime::time::timeout(&cx, Duration::from_secs(3), peers.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, peer);
        observed.recv(&cx).await.unwrap();
        peers.cancel_join_handle_v4.abort();
        peers.cancel_join_handle_v6.abort();
        let (v4, v6) = futures::join!(
            &mut peers.cancel_join_handle_v4,
            &mut peers.cancel_join_handle_v6
        );
        v4.expect("IPv4 discovery drains after cancellation acknowledgement");
        v6.expect("IPv6 discovery drains after cancellation acknowledgement");
        drop(peers);
        dht.cancellation_token().cancel();
        seed_task.abort();
        seed_task
            .await
            .expect("seed shutdown preserves its acknowledged completion");
        region.close().await.unwrap();
        assert_eq!(
            Arc::strong_count(&dht),
            1,
            "all protocol child owners are drained"
        );
        let rebound = std::net::UdpSocket::bind(dht.listen_addr())
            .expect("drained DHT releases its socket");
        drop(rebound);
    });
    runtime.block_on(task).unwrap();
}

#[test]
fn dropped_dht_request_releases_pending_transaction() {
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
        let (sender, _receiver) = unbounded_channel();
        let state = DhtState::new_internal(
            Id20::new([5; 20]),
            sender,
            None,
            None,
            (Ipv4Addr::LOCALHOST, 0).into(),
            PeerStore::new(Id20::new([5; 20])),
            CancellationToken::new(),
        );
        let mut request =
            Box::pin(state.request(&cx, Request::Ping, (Ipv4Addr::LOCALHOST, 9999).into()));
        std::future::poll_fn(|task| {
            assert!(request.as_mut().poll(task).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(state.inflight_by_transaction_id.len(), 1);
        drop(request);
        assert!(state.inflight_by_transaction_id.is_empty());
    });
    runtime.block_on(task).unwrap();
}
