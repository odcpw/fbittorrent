use crate::{BindOpts, ConnectOpts, TcpListener, UdpSocket, tcp_connect};
use rqbit_runtime::{
    Budget, RuntimeBuilder,
    io::{AsyncReadExt, AsyncWriteExt},
};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

#[test]
fn native_tcp_accept_split_and_udp_dualstack() {
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let handle = rqbit_runtime::task::spawn_with(&cx, |cx| async move {
        let listener =
            TcpListener::bind_tcp(&cx, (Ipv4Addr::LOCALHOST, 0).into(), BindOpts::default())
                .await
                .unwrap();
        let addr = listener.bind_addr();
        let server = async {
            let (stream, peer) = listener.accept(&cx).await.unwrap();
            assert!(peer.is_ipv4());
            let (mut read, mut write) = stream.into_split();
            let mut input = [0; 4];
            read.read_exact(&mut input).await.unwrap();
            assert_eq!(&input, b"ping");
            write.write_all(b"pong").await.unwrap();
        };
        let client = async {
            let mut stream = tcp_connect(&cx, addr, ConnectOpts::default())
                .await
                .unwrap();
            stream.write_all(b"ping").await.unwrap();
            let mut input = [0; 4];
            stream.read_exact(&mut input).await.unwrap();
            assert_eq!(&input, b"pong");
        };
        rqbit_runtime::time::timeout(&cx, Duration::from_secs(3), async {
            futures::join!(server, client);
        })
        .await
        .unwrap();

        let v6 =
            UdpSocket::bind_udp((Ipv6Addr::UNSPECIFIED, 0).into(), BindOpts::default()).unwrap();
        let v4 = UdpSocket::bind_udp((Ipv4Addr::LOCALHOST, 0).into(), BindOpts::default()).unwrap();
        v4.send_to(
            &cx,
            b"request",
            SocketAddr::from((Ipv4Addr::LOCALHOST, v6.bind_addr().port())),
        )
        .await
        .unwrap();
        let mut buffer = [0; 32];
        let (size, peer) = rqbit_runtime::time::timeout(
            &cx,
            Duration::from_secs(3),
            v6.recv_from(&cx, &mut buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buffer[..size], b"request");
        assert!(peer.is_ipv4(), "mapped IPv4 source is normalized");
        v6.send_to(&cx, b"response", peer).await.unwrap();
        let (size, _) = rqbit_runtime::time::timeout(
            &cx,
            Duration::from_secs(3),
            v4.recv_from(&cx, &mut buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buffer[..size], b"response");
    });
    runtime.block_on(handle).unwrap();
}

#[test]
fn parked_udp_receive_can_be_aborted_and_joined() {
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    let (started, mut observed) = rqbit_runtime::sync::oneshot::channel();
    let task = rqbit_runtime::task::spawn_with(&cx, move |cx| async move {
        let socket =
            UdpSocket::bind_udp((Ipv4Addr::LOCALHOST, 0).into(), BindOpts::default()).unwrap();
        started.send(&cx, socket.bind_addr()).unwrap();
        let mut buffer = [0; 16];
        socket.recv_from(&cx, &mut buffer).await
    });
    let released_addr = runtime.block_on(observed.recv(&cx)).unwrap();
    // Receiving the signal confirms the child has been polled before abort.
    task.abort();
    let read_error = runtime
        .block_on(task)
        .expect("acknowledged I/O error is retained by the native join")
        .expect_err("parked receive is cancelled");
    assert_eq!(read_error.kind(), std::io::ErrorKind::Interrupted);
    let rebound = std::net::UdpSocket::bind(released_addr)
        .expect("cancelled receive releases its owned socket");
    drop(rebound);
}

#[test]
fn isolated_bound_listener_udp_and_interface_loss() {
    if std::env::var("FBT_TEST_NETNS").as_deref() != Ok("1") {
        return;
    }
    assert_ne!(
        std::fs::read_link("/proc/self/ns/net")
            .unwrap()
            .to_string_lossy(),
        std::env::var("FBT_TEST_HOST_NETNS").expect("runner must capture its original namespace"),
        "test must own a disposable namespace"
    );
    println!("exercising bound TCP/UDP and interface loss inside disposable namespace");
    let set_lo = |state: &str| {
        assert!(
            std::process::Command::new("ip")
                .args(["link", "set", "lo", state])
                .status()
                .unwrap()
                .success()
        );
    };
    set_lo("up");
    let runtime = RuntimeBuilder::current_thread().build().unwrap();
    let owner = runtime.request_cx_with_budget(Budget::INFINITE);
    let task = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
        let device = crate::BindDevice::new_from_name("lo").unwrap();
        let opts = BindOpts {
            device: Some(&device),
            ..Default::default()
        };
        let listener = TcpListener::bind_tcp(&cx, (Ipv6Addr::UNSPECIFIED, 0).into(), opts)
            .await
            .unwrap();
        let mut client = tcp_connect(
            &cx,
            (Ipv4Addr::LOCALHOST, listener.bind_addr().port()).into(),
            ConnectOpts {
                bind_device: Some(&device),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (mut accepted, _) = listener.accept(&cx).await.unwrap();
        assert_eq!(
            socket2::SockRef::from(accepted.try_as_std())
                .device()
                .unwrap()
                .as_deref(),
            Some(b"lo".as_slice()),
            "accepted socket inherits the required device"
        );
        client.write_all(b"test").await.unwrap();
        let mut bytes = [0; 4];
        accepted.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"test");
        let udp = UdpSocket::bind_udp((Ipv6Addr::UNSPECIFIED, 0).into(), opts).unwrap();
        let sender = UdpSocket::bind_udp((Ipv4Addr::UNSPECIFIED, 0).into(), opts).unwrap();
        assert_eq!(
            socket2::SockRef::from(udp.socket())
                .device()
                .unwrap()
                .as_deref(),
            Some(b"lo".as_slice())
        );
        sender
            .send_to(
                &cx,
                b"data",
                (Ipv4Addr::LOCALHOST, udp.bind_addr().port()).into(),
            )
            .await
            .unwrap();
        let (size, _) = udp.recv_from(&cx, &mut bytes).await.unwrap();
        assert_eq!(size, 4);
        assert_eq!(&bytes, b"data");
        set_lo("down");
        assert!(
            sender
                .send_to(
                    &cx,
                    b"blocked",
                    (Ipv4Addr::LOCALHOST, udp.bind_addr().port()).into()
                )
                .await
                .is_err()
        );
        assert!(udp.recv_from(&cx, &mut bytes).await.is_err());
        assert!(accepted.write_all(b"blocked").await.is_err());
        assert!(listener.accept(&cx).await.is_err());
        set_lo("up");
    });
    runtime.block_on(task).unwrap();
    assert!(runtime.shutdown_timeout(Duration::from_secs(1)));
}
