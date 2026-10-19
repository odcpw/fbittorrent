use super::*;
use std::{os::unix::net::UnixListener, sync::Barrier};
fn hello() -> Value {
    include_str!("../../../../contracts/ipc/v1/hello.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["frame"]["result"]["proto"] == 1 && v["dir"] == "s2c")
        .unwrap()["frame"]
        .clone()
}
#[test]
fn bounded_reader_matches_out_of_order_requests_and_separates_events() {
    let path = std::env::temp_dir().join(format!(
        "fbt-client-{}-{}.sock",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut bytes = Vec::new();
        let request = read_frame(&mut reader, &mut bytes, None).unwrap();
        assert_eq!(request["args"]["client"]["kind"], "agent");
        let mut response = hello();
        response["id"] = request["id"].clone();
        send(&mut stream, &response).unwrap();
        let mut requests = Vec::new();
        for _ in 0..8 {
            requests.push(read_frame(&mut reader, &mut bytes, None).unwrap());
        }
        send(&mut stream,&json!({"ev":"engine.changed","seq":1,"ts":1792382400,"epoch":2,"state":"running","reason":"restart"})).unwrap();
        for request in requests.iter().rev() {
            send(
                &mut stream,
                &json!({"id":request["id"],"ok":true,"result":{"index":request["args"]["index"]}}),
            )
            .ok();
        }
    });
    let client = Arc::new(Client::connect_at(&Config::default(), true, &path).unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let workers = (0..8)
        .map(|index| {
            let c = client.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                (index, c.call("app.info", json!({"index":index})))
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        let (index, result) = worker.join().unwrap();
        assert_eq!(result.unwrap()["index"], index);
    }
    assert_eq!(
        client.event(Duration::from_secs(1)).unwrap().unwrap()["ev"],
        "engine.changed"
    );
    drop(client);
    server.join().unwrap();
    std::fs::remove_file(path).unwrap();
}
#[test]
fn reconnect_policy_preserves_add_refs_and_does_not_replay_file_deletion() {
    assert!(Client::idempotent("torrents.list"));
    assert!(Client::idempotent("torrents.stop"));
    assert!(Client::idempotent("torrents.add"));
    assert!(!Client::idempotent("torrents.remove"));
    assert_eq!(
        Client::reconnect_delays()
            .take(7)
            .map(|d| d.as_millis())
            .collect::<Vec<_>>(),
        [500, 1000, 2000, 4000, 8000, 10000, 10000]
    );
}

#[test]
fn subscription_fixture_revisions_and_gap_are_enforced() {
    let (writer, _peer) = UnixStream::pair().unwrap();
    let (_, receiver) = mpsc::sync_channel(1);
    let hello: HelloResult = serde_json::from_value(hello()["result"].clone()).unwrap();
    let connection = Connection {
        writer: Mutex::new(writer),
        pending: Arc::default(),
        events: Mutex::new(receiver),
        hello: hello.clone(),
        reader: None,
        end_reason: Arc::default(),
    };
    let client = Client {
        connection: Mutex::new(Arc::new(connection)),
        cfg: Config::default(),
        agent: false,
        path: PathBuf::new(),
        next: AtomicU64::new(2),
        hello,
        reconnects: AtomicU64::new(0),
        reconnect_limit: AtomicU64::new(u64::MAX),
    };
    let mut subscription = Subscription::new(&client);
    let mut count = 0;
    for line in include_str!("../../../../contracts/ipc/v1/subscribe.ndjson").lines() {
        let line: Value = serde_json::from_str(line).unwrap();
        let frame = &line["frame"];
        if line["dir"] == "s2c" && frame.get("ev").is_some() {
            subscription.accept(frame.clone()).unwrap();
            count += 1;
        }
    }
    assert!(count > 12);
    let mut subscription = Subscription::new(&client);
    subscription.accept(json!({"ev":"snapshot.begin","sub":1,"topic":"torrents","rev":1,"count":0,"reason":"subscribe"})).unwrap();
    let missing = json!({"ev":"snapshot.end","sub":1,"topic":"torrents","rev":3});
    assert!(subscription
        .accept(missing.clone())
        .unwrap_err()
        .to_string()
        .contains("rev"));
    subscription
        .accept(json!({"ev":"snapshot.chunk","sub":1,"topic":"torrents","rev":2,"items":[]}))
        .unwrap();
    subscription.accept(missing).unwrap();
}

#[test]
fn interrupted_add_resends_same_ref_and_observed_instance_once() {
    let path = std::env::temp_dir().join(format!("fbt-client-retry-{}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let mut prior = None;
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut bytes = Vec::new();
            let request = read_frame(&mut reader, &mut bytes, None).unwrap();
            let mut response = hello();
            response["id"] = request["id"].clone();
            send(&mut stream, &response).unwrap();
            let request = read_frame(&mut reader, &mut bytes, None).unwrap();
            if attempt == 0 {
                prior = Some((request["ref"].clone(), request["instance"].clone()));
                stream.shutdown(std::net::Shutdown::Both).unwrap();
            } else {
                assert_eq!(
                    prior.as_ref().unwrap(),
                    &(request["ref"].clone(), request["instance"].clone())
                );
                send(&mut stream,&json!({"id":request["id"],"ok":true,"result":{"state":"done","duplicate":true}})).unwrap();
            }
        }
    });
    let client = Client::connect_at(&Config::default(), true, &path).unwrap();
    let result=client.call("torrents.add",json!({"source":{"magnet":"magnet:?xt=urn:btih:abababababababababababababababababababab"}})).unwrap();
    assert_eq!(result["duplicate"], true);
    drop(client);
    server.join().unwrap();
    std::fs::remove_file(path).unwrap();
}
