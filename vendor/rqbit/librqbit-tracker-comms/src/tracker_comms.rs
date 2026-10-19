use rqbit_runtime::Cx;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::SocketAddrV6;
use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use anyhow::Context;
use futures::future::Either;
use futures::stream::BoxStream;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::trace_span;
use tracing::Instrument;
use url::Url;

use crate::tracker_comms_http;
use crate::tracker_comms_udp;
use crate::tracker_comms_udp::UdpTrackerClient;
use librqbit_core::hash_id::Id20;

pub struct TrackerComms {
    info_hash: Id20,
    peer_id: Id20,
    stats: Box<dyn TorrentStatsProvider>,
    force_tracker_interval: Option<Duration>,
    tx: Sender,
    // This MUST be set as trackers don't work with 0 port.
    announce_port: u16,
    http_client: rqbit_runtime::http::Client,
    key: u32,
}

#[derive(Default)]
pub enum TrackerCommsStatsState {
    #[default]
    None,
    Initializing,
    Paused,
    Live,
}

#[derive(Default)]
pub struct TrackerCommsStats {
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub torrent_state: TrackerCommsStatsState,
}

impl TrackerCommsStats {
    pub fn get_left_to_download_bytes(&self) -> u64 {
        let total = self.total_bytes;
        let down = self.downloaded_bytes;
        if total >= down {
            return total - down;
        }
        0
    }

    pub fn is_completed(&self) -> bool {
        self.downloaded_bytes >= self.total_bytes
    }
}

pub trait TorrentStatsProvider: Send + Sync {
    fn get(&self) -> TrackerCommsStats;
}

impl TorrentStatsProvider for () {
    fn get(&self) -> TrackerCommsStats {
        Default::default()
    }
}

type Sender = rqbit_runtime::sync::mpsc::Sender<SocketAddr>;

enum SupportedTracker {
    Udp(Url),
    Http(Url),
}

impl std::fmt::Debug for SupportedTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedTracker::Udp(u) => f.write_str(&crate::tracker_set::redact_url(u)),
            SupportedTracker::Http(u) => f.write_str(&crate::tracker_set::redact_url(u)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum UdpTrackerResolveResult {
    One(SocketAddr),
    Two(SocketAddrV4, SocketAddrV6),
}

async fn udp_tracker_to_socket_addrs(
    cx: &Cx,
    client: &rqbit_runtime::http::Client,
    host: url::Host<&str>,
    port: u16,
) -> anyhow::Result<UdpTrackerResolveResult> {
    cx.checkpoint()?;
    let res = match host {
        url::Host::Domain(name) => {
            // Use the first IPv4 and the first IPv6 addresses only.

            let mut v4: Option<SocketAddrV4> = None;
            let mut v6: Option<SocketAddrV6> = None;
            for addr in client
                .resolve(cx, name, port)
                .await
                .with_context(|| format!("error looking up hostname {name}"))?
            {
                match (v4, v6, addr) {
                    (None, _, SocketAddr::V4(addr)) => v4 = Some(addr),
                    (_, None, SocketAddr::V6(addr)) => v6 = Some(addr),
                    _ => continue,
                }
            }
            let res = match (v4, v6) {
                (Some(v4), Some(v6)) => UdpTrackerResolveResult::Two(v4, v6),
                (Some(v4), None) => UdpTrackerResolveResult::One(v4.into()),
                (None, Some(v6)) => UdpTrackerResolveResult::One(v6.into()),
                _ => anyhow::bail!("zero addresses returned looking up {name}"),
            };
            trace!(?res, "resolved");
            res
        }
        url::Host::Ipv4(addr) => UdpTrackerResolveResult::One((addr, port).into()),
        url::Host::Ipv6(addr) => UdpTrackerResolveResult::One((addr, port).into()),
    };
    Ok(res)
}

impl TrackerComms {
    // TODO: fix too many args
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        cx: &Cx,
        info_hash: Id20,
        peer_id: Id20,
        trackers: Arc<crate::TrackerSet>,
        first_only: bool,
        extra: Vec<Url>,
        stats: Box<dyn TorrentStatsProvider>,
        force_interval: Option<Duration>,
        announce_port: u16,
        http_client: rqbit_runtime::http::Client,
        udp_client: UdpTrackerClient,
    ) -> Option<BoxStream<'static, SocketAddr>> {
        let (tx, mut rx) = rqbit_runtime::sync::mpsc::channel::<SocketAddr>(16);

        let cx = cx.clone();
        let s = async_stream::stream! {
            use futures::StreamExt;
            let comms = Arc::new(Self {
                info_hash,
                peer_id,
                stats,
                force_tracker_interval: force_interval,
                tx,
                announce_port,
                http_client,
                key: rand::random(),
            });
            let mut changes = trackers.subscribe();
            let mut futures = FuturesUnordered::new();
            let mut active = HashMap::new();
            let mut dirty = true;
            loop {
                if cx.checkpoint().is_err() { break; }
                if dirty {
                    dirty = false;
                    // Mark the observed watch generation before reading the set, so
                    // an edit concurrent with the snapshot cannot be lost.
                    drop(changes.borrow_and_update());
                    let mut urls = trackers.snapshot();
                    if first_only { urls.truncate(1); } else { urls.extend(extra.iter().cloned()); }
                    urls.retain(|u| matches!(u.scheme(), "http" | "https" | "udp"));
                    active.retain(|url, abort: &mut futures::future::AbortHandle| {
                        if urls.contains(url) { true } else { abort.abort(); false }
                    });
                    // Reap aborted requests before admitting replacements: memory is
                    // bounded by the configured tracker set, even under rapid edits.
                    while futures.next().now_or_never().flatten().is_some() {}
                    for url in urls {
                        if active.contains_key(&url) { continue; }
                        let (abort, registration) = futures::future::AbortHandle::new_pair();
                        let tracker = if url.scheme() == "udp" { SupportedTracker::Udp(url.clone()) }
                            else { SupportedTracker::Http(url.clone()) };
                        active.insert(url, abort);
                        futures.push(futures::future::Abortable::new(
                            comms.add_tracker(&cx, tracker, &udp_client), registration));
                    }
                }
                enum Next { Addr(Result<SocketAddr, rqbit_runtime::sync::mpsc::RecvError>), Changed, Done }
                let next = {
                    let addr = rx.recv(&cx).fuse();
                    let changed = changes.changed(&cx).fuse();
                    let finished = async { if futures.is_empty() { futures::future::pending().await }
                        else { futures.next().await } }.fuse();
                    futures::pin_mut!(addr, changed, finished);
                    futures::select! { value = addr => Next::Addr(value), _ = changed => Next::Changed,
                        _ = finished => Next::Done }
                };
                match next {
                    Next::Addr(Ok(addr)) => yield addr,
                    Next::Addr(Err(_)) => break,
                    Next::Changed => dirty = true,
                    Next::Done => {},
                }
            }

        };

        Some(s.boxed())
    }

    fn add_tracker<'a>(
        &'a self,
        cx: &'a Cx,
        url: SupportedTracker,
        client: &'a UdpTrackerClient,
    ) -> Either<
        impl std::future::Future<Output = anyhow::Result<()>> + 'a + Send,
        impl std::future::Future<Output = anyhow::Result<()>> + 'a + Send,
    > {
        let info_hash = self.info_hash;
        match url {
            SupportedTracker::Udp(url) => {
                let span = debug_span!(parent: None, "udp_tracker", tracker = %crate::tracker_set::redact_url(&url), info_hash = ?info_hash);
                self.task_single_tracker_monitor_udp(cx, url, client.clone())
                    .instrument(span)
                    .right_future()
            }
            SupportedTracker::Http(url) => {
                let span = debug_span!(
                    parent: None,
                    "http_tracker",
                    tracker = %crate::tracker_set::redact_url(&url),
                    info_hash = ?info_hash
                );
                self.task_single_tracker_monitor_http(cx, url)
                    .instrument(span)
                    .left_future()
            }
        }
    }

    async fn task_single_tracker_monitor_http(
        &self,
        cx: &Cx,
        tracker_url: Url,
    ) -> anyhow::Result<()> {
        trace!(url=%crate::tracker_set::redact_url(&tracker_url), "starting monitor");
        let mut event = Some(tracker_comms_http::TrackerRequestEvent::Started);

        loop {
            cx.checkpoint()?;
            let mut retry = Duration::from_secs(10);
            let interval = loop {
                match self.tracker_one_request_http(cx, &tracker_url, event).await {
                    Ok(interval) => break interval,
                    Err(err) => {
                        debug!(?retry, "error calling tracker: {}", crate::tracker_set::sanitize(&tracker_url, &format!("{err:#}")));
                        rqbit_runtime::time::sleep(cx, retry).await;
                        cx.checkpoint()?;
                        retry = (retry * 2).min(Duration::from_secs(600));
                    }
                }
            };

            event = None;
            let interval = self.force_tracker_interval.unwrap_or(interval);
            debug!("sleeping for {:?} after calling tracker", interval);
            rqbit_runtime::time::sleep(cx, interval).await;
        }
    }

    async fn tracker_one_request_http(
        &self,
        cx: &Cx,
        tracker_url: &Url,
        event: Option<tracker_comms_http::TrackerRequestEvent>,
    ) -> anyhow::Result<Duration> {
        cx.checkpoint()?;
        let stats = self.stats.get();
        let request = tracker_comms_http::TrackerRequest {
            info_hash: &self.info_hash,
            peer_id: &self.peer_id,
            port: self.announce_port,
            uploaded: stats.uploaded_bytes,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            compact: true,
            no_peer_id: false,
            event,
            ip: None,
            numwant: None,
            key: Some(self.key),
            trackerid: None,
        };

        let mut url = tracker_url.clone();

        let mut queries = request.as_querystring();
        if let Some(url_query) = url.query() {
            queries.push_str(&format!("&{}", url_query));
        }
        url.set_query(Some(&queries));

        let bytes = self.http_client.get(cx, url.as_str()).await?;
        if let Ok((error, _)) =
            bencode::from_bytes_with_rest::<tracker_comms_http::TrackerError>(&bytes)
        {
            anyhow::bail!(
                "tracker returned failure. Failure reason: {}",
                error.failure_reason
            )
        };
        let response = bencode::from_bytes_with_rest::<tracker_comms_http::TrackerResponse>(&bytes)
            .map_err(|e| {
                tracing::trace!("error deserializing TrackerResponse: {e:#}");
                e.into_kind()
            })?
            .0;

        for peer in response.iter_peers() {
            self.tx.send(cx, peer).await?;
        }
        Ok(Duration::from_secs(
            response.min_interval.unwrap_or(response.interval),
        ))
    }

    async fn task_single_tracker_monitor_udp(
        &self,
        cx: &Cx,
        url: Url,
        client: UdpTrackerClient,
    ) -> anyhow::Result<()> {
        cx.checkpoint()?;
        if url.scheme() != "udp" {
            bail!("expected UDP scheme in {}", url);
        }
        let (host, port) = (
            url.host().context("missing host")?,
            url.port().context("missing port")?,
        );

        let mut sleep_interval: Option<Duration> = None;
        let mut prev_addrs: Option<UdpTrackerResolveResult> = None;
        loop {
            cx.checkpoint()?;
            if let Some(i) = sleep_interval {
                trace!(interval=?sleep_interval, "sleeping");
                rqbit_runtime::time::sleep(cx, i).await;
            }

            cx.checkpoint()?;
            let mut retry = Duration::from_secs(1);
            let addrs = loop {
                match udp_tracker_to_socket_addrs(cx, &self.http_client, host.clone(), port)
                    .await
                    .or_else(|err| prev_addrs.ok_or(err))
                {
                    Ok(addrs) => break addrs,
                    Err(err) => {
                        debug!(?retry, "error resolving tracker: {}", crate::tracker_set::sanitize(&url, &format!("{err:#}")));
                        rqbit_runtime::time::sleep(cx, retry).await;
                        cx.checkpoint()?;
                        retry = (retry * 2).min(Duration::from_secs(60));
                    }
                }
            };

            prev_addrs = Some(addrs);

            match addrs {
                UdpTrackerResolveResult::One(addr) => {
                    match self
                        .tracker_one_request_udp(cx, addr, &client)
                        .instrument(trace_span!("udp request", ?addr))
                        .await
                    {
                        Ok(sleep) => sleep_interval = Some(sleep),
                        Err(_) => {
                            sleep_interval = Some(sleep_interval.unwrap_or(Duration::from_secs(60)))
                        }
                    }
                }
                UdpTrackerResolveResult::Two(v4, v6) => {
                    let (r4, r6) = futures::join!(
                        self.tracker_one_request_udp(cx, v4.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v4)),
                        self.tracker_one_request_udp(cx, v6.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v6))
                    );
                    sleep_interval = Some(
                        r4.or(r6)
                            .ok()
                            .or(sleep_interval)
                            .unwrap_or(Duration::from_secs(60)),
                    )
                }
            }
        }
    }

    async fn tracker_one_request_udp(
        &self,
        cx: &Cx,
        addr: SocketAddr,
        client: &UdpTrackerClient,
    ) -> anyhow::Result<Duration> {
        cx.checkpoint()?;
        use tracker_comms_udp::*;

        let stats = self.stats.get();
        let request = AnnounceFields {
            info_hash: self.info_hash,
            peer_id: self.peer_id,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            uploaded: stats.uploaded_bytes,
            event: match stats.torrent_state {
                TrackerCommsStatsState::None => EVENT_NONE,
                TrackerCommsStatsState::Initializing => EVENT_STARTED,
                TrackerCommsStatsState::Paused => EVENT_STOPPED,
                TrackerCommsStatsState::Live => {
                    if stats.is_completed() {
                        EVENT_COMPLETED
                    } else {
                        EVENT_STARTED
                    }
                }
            },
            key: self.key,
            port: self.announce_port,
        };

        match client.announce(cx, addr, request).await {
            Ok(response) => {
                trace!(len = response.addrs.len(), "received announce response");
                for addr in response.addrs {
                    self.tx.send(cx, addr).await.context("rx closed")?;
                }
                let sleep = response.interval.max(5);
                let sleep = Duration::from_secs(sleep as u64);
                Ok(sleep)
            }
            Err(e) => {
                debug!(?addr, "error reading announce response: {e:#}");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use rqbit_runtime::{
        io::{AsyncReadExt, AsyncWriteExt},
        Budget, RuntimeBuilder,
    };
    use std::net::Ipv4Addr;

    #[test]
    fn native_http_tracker_announces_and_reads_compact_peers() {
        check_native_http_tracker(RuntimeBuilder::current_thread(), false);
    }

    #[test]
    fn native_http_tracker_runs_on_default_worker_stack() {
        check_native_http_tracker(RuntimeBuilder::multi_thread().worker_threads(1), true);
    }

    fn check_native_http_tracker(builder: RuntimeBuilder, expect_worker: bool) {
        let runtime = builder.build().unwrap();
        assert_eq!(runtime.config().thread_stack_size, 2 * 1024 * 1024);
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&cx, move |cx| async move {
            if expect_worker {
                assert!(std::thread::current()
                    .name()
                    .unwrap()
                    .starts_with("asupersync-worker-"));
            }
            let region = cx
                .open_child_region(rqbit_runtime::cx::ChildRegionSpec::inherit())
                .await
                .unwrap();
            let cx = region.cx().clone();
            let listener = rqbit_runtime::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let listen_addr = listener.local_addr().unwrap();
            let tracker = Url::parse(&format!(
                "http://{}/announce?passkey=test",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let (tx, mut rx) = rqbit_runtime::sync::mpsc::channel(2);
            let server = async {
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut buf = [0; 1024];
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let read = socket.read(&mut buf).await.unwrap();
                        assert!(read > 0);
                        request.extend_from_slice(&buf[..read]);
                        assert!(request.len() < 8192);
                    }
                    let request = String::from_utf8(request).unwrap();
                    assert!(request.starts_with("GET /announce?"));
                    assert!(request.contains("passkey=test"));
                    assert!(request.contains("port=7777"));
                    assert!(request.contains("info_hash=%03%03"));
                    let mut body = b"d8:intervali30e5:peers6:".to_vec();
                    body.extend_from_slice(&[127, 0, 0, 1, 0x11, 0x5c]);
                    body.push(b'e');
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(headers.as_bytes()).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                }
            };
            let client = async {
                let comms = TrackerComms {
                    info_hash: Id20::new([3; 20]),
                    peer_id: Id20::new([8; 20]),
                    stats: Box::new(()),
                    force_tracker_interval: None,
                    tx,
                    announce_port: 7777,
                    http_client: rqbit_runtime::http::Client::new("rqbit native test"),
                    key: 1,
                };
                let interval = comms
                    .tracker_one_request_http(
                        &cx,
                        &tracker,
                        Some(tracker_comms_http::TrackerRequestEvent::Started),
                    )
                    .await
                    .unwrap();
                assert_eq!(interval, Duration::from_secs(30));
                assert_eq!(
                    rx.recv(&cx).await.unwrap(),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 4444))
                );
                // Run the actual periodic monitor, observe its first announce,
                // then cancel while the advertised thirty-second timer is parked.
                let monitor = rqbit_runtime::task::spawn_with(&cx, move |child_cx| async move {
                    comms
                        .task_single_tracker_monitor_http(&child_cx, tracker)
                        .await
                });
                assert_eq!(
                    rx.recv(&cx).await.unwrap(),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 4444))
                );
                rqbit_runtime::time::sleep(&cx, Duration::from_millis(10)).await;
                assert!(!monitor.is_finished());
                monitor.abort();
                assert!(monitor
                    .await
                    .expect("HTTP monitor acknowledges native cancellation")
                    .is_err());
            };
            rqbit_runtime::time::timeout(&cx, Duration::from_secs(3), async {
                futures::join!(server, client);
            })
            .await
            .unwrap();
            region.close().await.unwrap();
            drop(listener);
            let rebound = std::net::TcpListener::bind(listen_addr)
                .expect("HTTP fixture releases its listening socket");
            drop(rebound);
        });
        runtime.block_on(task).unwrap();
    }
}
