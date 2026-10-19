//! Temporary engine.sock adapter, removed in P4-09. ipc::Server owns its
//! existing 4-byte big-endian framing, 32 MiB cap and joined request workers.
use crate::{
    agent::{AppError, Code},
    daemon::lifecycle::Lifecycle,
    engine::{
        ops::{EngineOp, EngineResult},
        EngineDispatcher, ParkedLedger,
    },
    ipc::Request,
    network_app::NetworkState,
};
use anyhow::{anyhow, Result};

pub(crate) fn parse(request: &Request) -> Result<EngineOp> {
    EngineOp::parse(&request.path, request)?.ok_or_else(|| {
        anyhow!(
            "unsupported native engine request {}",
            request.path.chars().take(80).collect::<String>()
        )
    })
}

pub(crate) fn control(
    op: &EngineOp,
    network: &NetworkState,
    lifecycle: &Lifecycle,
) -> Option<Result<String>> {
    op.control(network, lifecycle)
        .map(|result| result.and_then(|reply| reply.to_legacy_json()))
}

pub(crate) fn running(dispatcher: &EngineDispatcher, op: EngineOp) -> Result<String> {
    dispatcher.execute(op)?.to_legacy_json()
}

pub(crate) fn parked(ledger: &ParkedLedger, op: EngineOp, interface: &str) -> Result<String> {
    ledger
        .execute(op)
        .and_then(|reply| {
            let wire = reply.to_legacy_json()?;
            Ok(match reply {
                // The old parked facade used Vec<Value>, whereas live lists used
                // structs. Preserve member order as well as the legacy field shape.
                EngineResult::Torrents(_) | EngineResult::Files(_) => {
                    serde_json::from_str::<serde_json::Value>(&wire)?.to_string()
                }
                EngineResult::Version(_) => wire.replace("ProtonVPN", interface),
                _ => wire,
            })
        })
        .map_err(|error| {
            let message = format!("{error:#}");
            if message.starts_with("connect ProtonVPN")
                || message.starts_with("saved torrent metadata is unavailable; connect ProtonVPN")
            {
                AppError::new(
                    Code::Backend,
                    format!(
                        "engine_unavailable state=parked; {}",
                        message.replace("ProtonVPN", interface)
                    ),
                )
                .hint(format!("connect {interface} then retry"))
                .into()
            } else {
                error
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::lifecycle::{Request as LifecycleRequest, RestartReason, StopReason};
    use crate::engine::ops::{Route, LEGACY_ROUTES};
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    fn request(route: Route) -> Request {
        Request {
            method: "POST".into(),
            path: route.name().into(),
            args: [
                ("hashes", "all"),
                ("hash", "0000000000000000000000000000000000000001"),
                (
                    "urls",
                    "magnet:?xt=urn:btih:0000000000000000000000000000000000000001",
                ),
                ("max_active_downloads", "3"),
                ("id", "0|1"),
                ("priority", "1"),
                ("limit", "32768"),
                ("ratioLimit", "0"),
                ("shareLimitAction", "Stop"),
                ("tags", "alpha, beta"),
                ("deleteFiles", "true"),
                ("skip_complete", "true"),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
            file: None,
        }
    }

    #[test]
    fn route_parse_roundtrip() {
        assert_eq!(LEGACY_ROUTES.len(), 22);
        for (name, route) in LEGACY_ROUTES {
            let body = request(*route);
            let original = parse(&body).unwrap();
            let bytes = serde_json::to_vec(&body).unwrap();
            let decoded: Request = serde_json::from_slice(&bytes).unwrap();
            let parsed = parse(&decoded).unwrap();
            assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
            assert_eq!(format!("{original:?}"), format!("{parsed:?}"));
            // Every route names its expected typed variant, independently of
            // the parser's string table. A new Route must extend this match.
            let matches = match route {
                Route::Version => matches!(parsed, EngineOp::Version),
                Route::Shutdown => matches!(parsed, EngineOp::Shutdown),
                Route::Restart => matches!(parsed, EngineOp::Restart),
                Route::Network => matches!(parsed, EngineOp::Network),
                Route::Settings => matches!(parsed, EngineOp::Settings),
                Route::SetPreferences => matches!(parsed, EngineOp::SetPreferences),
                Route::Queue => matches!(parsed, EngineOp::Queue),
                Route::SetQueue => matches!(parsed, EngineOp::SetQueue { limit: 3 }),
                Route::TransferInfo => matches!(parsed, EngineOp::TransferInfo),
                Route::Info => matches!(parsed, EngineOp::Info),
                Route::Files => matches!(parsed, EngineOp::Files { .. }),
                Route::Add => matches!(parsed, EngineOp::Add(_)),
                Route::Start => matches!(
                    parsed,
                    EngineOp::Start {
                        skip_complete: true,
                        ..
                    }
                ),
                Route::Stop => matches!(parsed, EngineOp::Stop { .. }),
                Route::Delete => matches!(parsed, EngineOp::Delete { files: true, .. }),
                Route::FilePriority => matches!(parsed, EngineOp::FilePriority { priority: 1, .. }),
                Route::SetUploadLimit => {
                    matches!(parsed, EngineOp::SetUploadLimit { limit: 32768, .. })
                }
                Route::SetShareLimits => {
                    matches!(parsed, EngineOp::SetShareLimits { ratio: 0.0, .. })
                }
                Route::AddTags => matches!(parsed, EngineOp::AddTags { .. }),
                Route::RemoveTags => matches!(parsed, EngineOp::RemoveTags { .. }),
                Route::ToggleSequentialDownload => {
                    matches!(parsed, EngineOp::ToggleSequentialDownload)
                }
                Route::ToggleFirstLastPiecePrio => {
                    matches!(parsed, EngineOp::ToggleFirstLastPiecePrio)
                }
            };
            println!(
                "legacy route {name}: parsed {parsed:?}; request JSON round-trip={} bytes",
                bytes.len()
            );
            assert!(matches, "{name}: {parsed:?}");
        }
        let mut upload = request(Route::Add);
        upload.file = Some(crate::ipc::Upload {
            name: "fixture.torrent".into(),
            bytes: vec![0, 255, b'\n'],
        });
        let decoded: Request =
            serde_json::from_slice(&serde_json::to_vec(&upload).unwrap()).unwrap();
        let EngineOp::Add(add) = parse(&decoded).unwrap() else {
            panic!("typed upload")
        };
        assert_eq!(
            add.torrent.unwrap(),
            ("fixture.torrent".into(), vec![0, 255, b'\n'])
        );
        let mut wrong_method = request(Route::Restart);
        wrong_method.method = "GET".into();
        assert!(parse(&wrong_method).is_err());
        wrong_method.path = "unknown".into();
        assert!(parse(&wrong_method).is_err());
    }

    fn exchange(stream: &mut UnixStream, body: &Request) -> serde_json::Value {
        let bytes = serde_json::to_vec(body).unwrap();
        // Fragment the header and body, like a stream rather than datagrams.
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes()[..2])
            .unwrap();
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes()[2..])
            .unwrap();
        for chunk in bytes.chunks(7) {
            stream.write_all(chunk).unwrap();
        }
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let mut reply = vec![0; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut reply).unwrap();
        serde_json::from_slice(&reply).unwrap()
    }

    #[test]
    fn socket_keeps_legacy_bytes_and_control_lifecycle() {
        let directory = std::env::temp_dir().join(format!(
            "fbt-p110-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let lock = crate::ipc::InstanceLock::acquire_in(directory.clone()).unwrap();
        let ledger = ParkedLedger::new(directory.join("ledger"), Lifecycle::default());
        let lifecycle = Lifecycle::default();
        let network = crate::network_app::state(&crate::config::Config::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let owner = lifecycle.clone();
        let server =
            crate::ipc::Server::start(&crate::config::Config::default(), &lock, move |body| {
                count.fetch_add(1, Ordering::AcqRel);
                let op = parse(body)?;
                if let Some(reply) = control(&op, &network, &owner) {
                    return reply;
                }
                parked(&ledger, op, "lo")
            })
            .unwrap();
        let path = directory.join("engine.sock");
        let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&path).unwrap());
        let mut pending = UnixStream::connect(&path).unwrap();
        pending
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        for (route, expected) in [
            (
                Route::Restart,
                LifecycleRequest::Restart(RestartReason::Explicit),
            ),
            (Route::Shutdown, LifecycleRequest::Stop(StopReason::Quit)),
        ] {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let reply = exchange(&mut stream, &request(route));
            assert_eq!(reply["value"], "Ok.");
            assert!(reply["error"].is_null());
            assert_eq!(lifecycle.wait(Duration::ZERO), Some(expected));
            assert_eq!(
                std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&path).unwrap()),
                inode
            );
            println!(
                "legacy socket {route:?}: reply={reply}; lifecycle={expected:?}; inode={inode}"
            );
            if route == Route::Restart {
                // Delivery leaves an already-connected reader owned by the
                // persistent listener; real engine replacement is qualified by
                // daemon.py::engine_restart_retains_owner.
                let reply = exchange(&mut pending, &request(Route::Info));
                assert_eq!(reply["value"], "[]");
                assert!(reply["error"].is_null());
            }
        }
        assert_eq!(calls.load(Ordering::Acquire), 3);
        drop(pending);
        drop(server);
        drop(lock);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parked_result_matches_existing_facade_byte_for_byte() {
        let directory = std::env::temp_dir().join(format!(
            "fbt-p110-bytes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut bytes =
            b"d4:infod6:lengthi5e4:name10:legacy.txt12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        std::fs::write(directory.join("torrents.json"), serde_json::json!({"migrated_legacy":true,"records":[{
            "hash":hash,"name":"legacy fixture","source":"","output_folder":directory.join("downloads"),
            "only_files":null,"upload_limit":32768,"ratio_limit":0.0,"category":"fixture","tags":"alpha,beta",
            "added_on":1,"completion_on":0,"paused":true,"resume_after_migration":false,
        }]}).to_string()).unwrap();
        let ledger = ParkedLedger::new(directory.clone(), Lifecycle::default());
        for route in [Route::Info, Route::Files, Route::TransferInfo] {
            let mut body = request(route);
            body.args
                .iter_mut()
                .find(|(key, _)| key == "hash")
                .unwrap()
                .1 = hash.clone();
            let before = ledger.dispatch(&body).unwrap().unwrap();
            let after = parked(&ledger, parse(&body).unwrap(), "ProtonVPN").unwrap();
            assert_eq!(
                after.as_bytes(),
                before.as_bytes(),
                "{route:?} member order and values"
            );
            let value: serde_json::Value = serde_json::from_str(&after).unwrap();
            match route {
                Route::Info => {
                    assert_eq!(value[0]["state"], "stoppedDL");
                    assert_eq!(value[0]["progress_known"], false);
                }
                Route::Files => {
                    assert_eq!(value[0]["name"], "legacy.txt");
                    assert_eq!(value[0]["progress_known"], false);
                }
                Route::TransferInfo => assert_eq!(value["connection_status"], "vpn_waiting"),
                _ => unreachable!(),
            }
            println!(
                "legacy parked {route:?}: byte-identical {} bytes; {after}",
                after.len()
            );
        }
        assert!(parked(&ledger, EngineOp::Version, "fbmissing0")
            .unwrap()
            .contains("waiting for fbmissing0"));
        let failure = parked(
            &ledger,
            EngineOp::Add(crate::engine::ops::AddRequest {
                sources: vec![
                    "magnet:?xt=urn:btih:0000000000000000000000000000000000000001".into(),
                ],
                torrent: None,
                save_path: None,
                upload_limit: 32768,
                ratio_limit: 0.0,
                stopped: true,
                category: String::new(),
                tags: String::new(), origin: None,
            }),
            "fbmissing0",
        )
        .unwrap_err();
        let failure = failure.downcast_ref::<AppError>().unwrap();
        assert_eq!(failure.code, Code::Backend);
        assert!(failure.to_string().contains("fbmissing0"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
