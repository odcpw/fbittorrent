use super::*;
fn hello() -> Vec<u8> {
    br#"{"id":1,"op":"hello","args":{"proto":1,"client":{"name":"test","version":"0","kind":"test"}}}"#.to_vec()
}
#[test]
fn peercred_rejects_other_uid() {
    let (socket, _) = UnixStream::pair().unwrap();
    let owner = unsafe { libc::geteuid() };
    assert_eq!(peer_uid(&socket).unwrap(), owner);
    assert!(allowed_uid(owner, owner));
    assert!(!allowed_uid(owner.wrapping_add(1), owner));
    println!(
        "peer credentials: kernel same uid={owner} accepted; injected foreign uid={} refused",
        owner.wrapping_add(1)
    );
}
#[test]
fn garbage_table() {
    use ErrorCode::*;
    let table = vec![
        ("invalid UTF-8", vec![0xff, b'\n'], InvalidMessage, true),
        ("empty line", b"\n".to_vec(), InvalidMessage, true),
        ("65537 bytes", vec![b' '; 65_537], InvalidMessage, true),
        (
            "10000-deep nesting",
            ("[".repeat(10_000) + &"]".repeat(10_000) + "\n").into_bytes(),
            InvalidMessage,
            true,
        ),
        ("non-object", b"[]\n".to_vec(), InvalidMessage, true),
        (
            "duplicate keys",
            br#"{"id":1,"id":2,"op":"app.info"}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "nested duplicate keys",
            br#"{"id":1,"op":"app.client","args":{"glance":true,"glance":false}}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "id zero",
            br#"{"id":0,"op":"app.info"}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "negative id",
            br#"{"id":-1,"op":"app.info"}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "fractional id",
            br#"{"id":1.5,"op":"app.info"}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "string id",
            br#"{"id":"1","op":"app.info"}"#.to_vec(),
            InvalidArgs,
            false,
        ),
        (
            "unknown top-level key",
            br#"{"id":1,"op":"app.info","other":true}"#.to_vec(),
            InvalidArgs,
            false,
        ),
    ];
    for (name, input, code, close) in table {
        let failure = decode(&input).unwrap_err();
        println!(
            "api garbage {name}: code={:?}; closes={}; id={:?}",
            failure.error.code, failure.close, failure.id
        );
        assert_eq!(failure.error.code, code, "{name}");
        assert_eq!(failure.close, close, "{name}");
    }
    let mut crlf = hello();
    crlf.extend_from_slice(b"\r\n");
    assert!(matches!(decode(&crlf).unwrap().call, Call::Hello { .. }));
    println!("api garbage CRLF hello: accepted");
    let error = decode(br#"{"id":1,"op":"torrents.lis"}"#)
        .unwrap_err()
        .error;
    assert_eq!(error.code, UnknownOp);
    assert!(error.details.unwrap()["nearest"]
        .as_array()
        .unwrap()
        .contains(&json!("torrents.list")));
}
pub(super) fn connection() -> (Connection, UnixStream) {
    let (stream, peer) = UnixStream::pair().unwrap();
    (
        Connection {
            number: 1,
            stream,
            accepted: Instant::now(),
            closed: AtomicBool::new(false),
            read_done: AtomicBool::new(false),
            client: Mutex::new(ClientState::default()),
            pending: Mutex::new(BTreeMap::new()),
            background: Mutex::new(BTreeMap::new()),
            sequence: AtomicU64::new(1),
            out: Mutex::new(Outbox::default()),
            ready: Condvar::new(),
            emission: Mutex::new(()),
        },
        peer,
    )
}
#[test]
fn timed_out_id_is_reusable_and_late_result_is_dropped() {
    let (connection, _) = connection();
    let request = decode(br#"{"id":7,"op":"app.info"}"#).unwrap();
    let first = connection.reserve(&request).unwrap();
    connection
        .pending
        .lock()
        .unwrap()
        .get_mut(&7)
        .unwrap()
        .deadline = Instant::now();
    connection.expire();
    assert!(connection.pending.lock().unwrap().is_empty());
    let second = connection.reserve(&request).unwrap();
    assert_ne!(first, second);
    connection.complete(7, first, Err(stopping()));
    assert_eq!(connection.out.lock().unwrap().frames.len(), 1);
    connection.complete(7, second, Ok(Reply::AppClient(EmptyResult {})));
    let frames = &connection.out.lock().unwrap().frames;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["error"]["code"], "timeout");
    assert_eq!(frames[1]["ok"], true);
    println!("timeout released id=7; generation {first} late result dropped; generation {second} delivered");
}
#[test]
fn inflight_and_outgoing_limits_are_bounded() {
    let (connection, _) = connection();
    for id in 1..=8 {
        connection
            .reserve(&decode(format!("{{\"id\":{id},\"op\":\"app.info\"}}").as_bytes()).unwrap())
            .unwrap();
    }
    let failure = connection
        .reserve(&decode(br#"{"id":9,"op":"app.info"}"#).unwrap())
        .unwrap_err();
    assert_eq!(failure.error.details.unwrap()["limit"], "inflight");
    let reused = connection
        .reserve(&decode(br#"{"id":1,"op":"app.info"}"#).unwrap())
        .unwrap_err();
    assert_eq!(reused.id, None);
    assert_eq!(reused.error.code, ErrorCode::InvalidArgs);
    for _ in 0..=512 {
        connection.send(json!({"ev":"heartbeat","ts":1}));
    }
    let out = connection.out.lock().unwrap();
    assert!(out.frames.is_empty());
    assert!(out.ending);
    assert_eq!(out.terminal.as_ref().unwrap()["reason"], "slow_client");
    println!("inflight=8; ninth busy; reused id invalid_args; outgoing=512 overflow ends with slow_client");
}
#[test]
fn output_cap_is_checked_before_writing_bytes() {
    let (mut stream, mut peer) = UnixStream::pair().unwrap();
    let result = write_frame(&mut stream, &json!({"result":"x".repeat(LINE_OUT)}));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    println!("oversized output rejected before any socket bytes; hard cap includes newline");
}
#[test]
fn file_sort_key_is_natural_and_preserves_digit_runs() {
    let key = ops::torrents::sort_key;
    assert!(key("Dir/E2.mkv") < key("dir/e10.mkv"));
    assert_eq!(key("a12b3"), "a0000000012b0000000003");
    assert_eq!(key("a12345678901"), "a12345678901");
}
