// Disposable native-runtime transfer tests against libtorrent-rasterbar.
use super::*;
use crate::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
};

struct Fixture {
    process: Child,
    line: serde_json::Value,
    directory: tempfile::TempDir,
}
impl Fixture {
    fn status(&mut self) -> serde_json::Value {
        self.process
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"status\n")
            .unwrap();
        let mut line = String::new();
        BufReader::new(self.process.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        serde_json::from_str(&line).expect("fixture status JSON")
    }
    fn start(
        role: &str,
        encryption: &str,
        connect: Option<std::net::SocketAddr>,
        torrent: Option<&std::path::Path>,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/mse/tests/libtorrent_fixture.py");
        let mut command = Command::new("python");
        command
            .arg(script)
            .arg(role)
            .arg(encryption)
            .arg(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(addr) = connect {
            command.arg(addr.to_string());
        }
        if let Some(torrent) = torrent {
            command.arg(torrent);
        }
        let mut process = command.spawn().expect("Python libtorrent fixture");
        let mut line = String::new();
        BufReader::new(process.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        let line =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("fixture startup {e}: {line}"));
        Self {
            process,
            line,
            directory,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

pub(super) fn run() {
    run_with_pool_max(2);
}

pub(super) fn run_with_pool_max(pool_max: usize) {
    for (policy, oracle, magnet) in [
        (PeerEncryption::Require, "forced", true),
        (PeerEncryption::Prefer, "forced", false),
        (PeerEncryption::Off, "disabled", false),
        (PeerEncryption::Prefer, "disabled", true),
    ] {
        let mut fixture = Fixture::start("seed", oracle, None, None);
        let peer: std::net::SocketAddr = fixture.line["addr"].as_str().unwrap().parse().unwrap();
        let torrent = std::fs::read(fixture.directory.path().join("fixture.torrent")).unwrap();
        let hash = fixture.line["hash"].as_str().unwrap().to_owned();
        let output = tempfile::tempdir().unwrap();
        let output_path = output.path().to_owned();
        let runtime = rqbit_runtime::RuntimeBuilder::new()
            .worker_threads(1)
            .blocking_threads(1, pool_max)
            .build()
            .unwrap();
        let actual_pool_max = runtime.config().blocking.max_threads;
        assert_eq!(actual_pool_max, pool_max, "outgoing runtime pool maximum");
        let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
            let session = Session::new_with_opts(
                &cx,
                output_path,
                SessionOptions {
                    peer_encryption: policy,
                    runtime_worker_threads: Some(2),
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let opts = AddTorrentOptions {
                initial_peers: Some(vec![peer]),
                ..Default::default()
            };
            let input = if magnet {
                AddTorrent::from_url(format!("magnet:?xt=urn:btih:{hash}"))
            } else {
                AddTorrent::from_bytes(torrent)
            };
            let handle = rqbit_runtime::time::timeout(
                &cx,
                Duration::from_secs(30),
                session.add_torrent(&cx, input, Some(opts)),
            )
            .await
            .expect("metadata deadline")
            .expect("add torrent")
            .into_handle()
            .unwrap();
            rqbit_runtime::time::timeout(
                &cx,
                Duration::from_secs(30),
                handle.wait_until_completed(&cx),
            )
            .await
            .expect("payload deadline")
            .expect("payload completion");
            session.stop(&cx).await.unwrap();
        });
        runtime.block_on(task).unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(3)));
        assert_eq!(
            std::fs::read(output.path().join("payload.bin")).unwrap(),
            std::fs::read(fixture.directory.path().join("payload.bin")).unwrap()
        );
        let status = fixture.status();
        if oracle == "forced" {
            assert_eq!(status["rc4"], true, "oracle must see RC4 payload: {status}");
        } else {
            assert_eq!(status["rc4"], false);
            assert_eq!(status["plaintext"], true);
        }
        println!(
            "libtorrent outgoing policy={policy:?}, oracle={oracle}, magnet={magnet}, blocking_pool_max={actual_pool_max}: {status}"
        );
    }
    for (policy, oracle) in [
        (PeerEncryption::Require, "forced"),
        (PeerEncryption::Prefer, "forced"),
        (PeerEncryption::Off, "disabled"),
        (PeerEncryption::Prefer, "disabled"),
    ] {
        let fixture = Fixture::start("seed", "forced", None, None);
        let source = fixture.directory.path().to_owned();
        let runtime = rqbit_runtime::RuntimeBuilder::new()
            .worker_threads(1)
            .blocking_threads(1, pool_max)
            .build()
            .unwrap();
        let actual_pool_max = runtime.config().blocking.max_threads;
        assert_eq!(actual_pool_max, pool_max, "incoming runtime pool maximum");
        let owner = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
        let task = rqbit_runtime::task::spawn_with(&owner, move |cx| async move {
            let session = Session::new_with_opts(
                &cx,
                source.clone(),
                SessionOptions {
                    peer_encryption: policy,
                    runtime_worker_threads: Some(2),
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    listen: Some(crate::ListenerOptions {
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            // Two managed torrents prove encrypted SKEY routing, not simply
            // taking the first or only torrent in the session.
            for name in ["other.torrent", "fixture.torrent"] {
                let handle = session
                    .add_torrent(
                        &cx,
                        AddTorrent::from_bytes(std::fs::read(source.join(name)).unwrap()),
                        Some(AddTorrentOptions { overwrite:true, ..Default::default() }),
                    )
                    .await
                    .unwrap()
                    .into_handle()
                    .unwrap();
                handle
                    .live_wait_initializing(&cx, Duration::from_secs(5))
                    .await
                    .unwrap();
                assert!(handle.stats().finished);
            }
            let mut leech = Fixture::start(
                "magnet-leech",
                oracle,
                session.listen_addr(),
                Some(&source.join("fixture.torrent")),
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let status = loop {
                let status = leech.status();
                if status["is_seeding"] == true {
                    break status;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "incoming transfer deadline {status}"
                );
                rqbit_runtime::time::sleep(&cx, Duration::from_millis(30)).await;
            };
            assert_eq!(status["has_metadata"], true);
            if oracle == "forced" {
                assert_eq!(status["rc4"], true);
            } else {
                assert_eq!(status["rc4"], false);
                assert_eq!(status["plaintext"], true);
            }
            assert_eq!(status["downloaded"].as_u64().unwrap(), 192 * 1024 + 113);
            assert_eq!(
                std::fs::read(leech.directory.path().join("payload.bin")).unwrap(),
                std::fs::read(source.join("payload.bin")).unwrap()
            );
            println!(
                "libtorrent incoming policy={policy:?}, oracle={oracle}, magnet=true, two torrents, blocking_pool_max={actual_pool_max}: {status}"
            );
            session.stop(&cx).await.unwrap();
        });
        runtime.block_on(task).unwrap();
        assert!(runtime.shutdown_timeout(Duration::from_secs(3)));
    }
}
