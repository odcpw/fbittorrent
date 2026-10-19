//! Same-user socket listener and connection ownership across engine restarts.

use super::*;

pub(super) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::other("invalid peer credentials"));
    }
    Ok(credentials.uid)
}
pub(super) fn allowed_uid(peer: u32, owner: u32) -> bool {
    peer == owner
}


impl Server {
    pub(crate) fn start(
        directory: &Path,
        lock: Arc<InstanceLock>,
        daemon: Arc<Daemon>,
    ) -> anyhow::Result<Self> {
        let path = directory.join("api.sock");
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            daemon,
            pool: Pool::default(),
            clients: Mutex::new(HashMap::new()),
            counts: Mutex::new(ClientCounts {
                cli: 0,
                agent: 0,
                tui: 0,
                bar: 0,
                test: 0,
            }),
            began: Instant::now(),
            stopping: AtomicBool::new(false),
            event_sequence: AtomicU64::new(1),
        });
        let connections = Arc::new(Mutex::new(Vec::new()));
        // Own partially-created workers too: an OS thread-spawn failure must
        // close the pool and join everything already started.
        let mut server = Self {
            path,
            shared: shared.clone(),
            accept: None,
            connections: connections.clone(),
            dispatch: Vec::new(),
            _lock: lock,
        };
        for index in 0..DISPATCH {
            let state = shared.clone();
            server.dispatch.push(
                thread::Builder::new()
                    .name(format!("fbt-api-dispatch-{index}"))
                    .spawn(move || {
                        while let Some(job) = state.pool.next() {
                            state.dispatch(job);
                        }
                    })?,
            );
        }
        let workers = connections.clone();
        let state = shared.clone();
        let accept = thread::Builder::new()
            .name("fbt-api-accept".into())
            .spawn(move || {
                let mut sequence = 0;
                while !state.stopping.load(Ordering::Acquire) {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(e) => {
                            eprintln!("WARN api accept: {e}");
                            continue;
                        }
                    };
                    let accepted = Instant::now();
                    match peer_uid(&stream) {
                        Ok(uid) if allowed_uid(uid, unsafe { libc::geteuid() }) => {}
                        observed => {
                            eprintln!(
                                "WARN api foreign or unavailable peer credential: {observed:?}"
                            );
                            continue;
                        }
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    if state.clients.lock().unwrap().len() >= CONNECTIONS {
                        let mut stream = stream;
                        let _ = write_frame(
                            &mut stream,
                            &json!({"ev":"error","error":busy("connections")}),
                        );
                        let _ = write_frame(&mut stream, &bye(ByeReason::InvalidMessage));
                        continue;
                    }
                    sequence += 1;
                    let Ok(reader) = stream.try_clone() else {
                        continue;
                    };
                    let Ok(writer) = stream.try_clone() else {
                        continue;
                    };
                    let connection = Arc::new(Connection {
                        number: sequence,
                        stream,
                        accepted,
                        closed: AtomicBool::new(false),
                        read_done: AtomicBool::new(false),
                        client: Mutex::new(ClientState::default()),
                        pending: Mutex::new(BTreeMap::new()),
                        background: Mutex::new(BTreeMap::new()),
                        sequence: AtomicU64::new(1),
                        out: Mutex::new(Outbox::default()),
                        ready: Condvar::new(),
                        emission: Mutex::new(()),
                    });
                    state
                        .clients
                        .lock()
                        .unwrap()
                        .insert(sequence, connection.clone());
                    let c = connection.clone();
                    let s = state.clone();
                    let reading = thread::Builder::new()
                        .name("fbt-api-reader".into())
                        .spawn(move || read_client(s, c, reader));
                    let c = connection.clone();
                    let s = state.clone();
                    let writing = thread::Builder::new()
                        .name("fbt-api-writer".into())
                        .spawn(move || write_client(s, c, writer));
                    let mut threads = workers.lock().unwrap();
                    for result in [reading, writing] {
                        match result {
                            Ok(thread) => threads.push(thread),
                            Err(e) => {
                                eprintln!("WARN api thread: {e}");
                                connection.closed.store(true, Ordering::Release);
                                connection.ready.notify_one();
                                let _ = connection.stream.shutdown(Shutdown::Both);
                                state.clients.lock().unwrap().remove(&connection.number);
                            }
                        }
                    }
                    let mut index = 0;
                    while index < threads.len() {
                        if threads[index].is_finished() {
                            let _ = threads.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                }
            })?;
        server.accept = Some(accept);
        Ok(server)
    }
    /// P1-08 owns subscriptions; these handles are its bounded event transport.
    pub(crate) fn event_sinks(&self) -> Vec<EventSink> {
        self.shared
            .clients
            .lock()
            .unwrap()
            .values()
            .filter(|connection| connection.client.lock().unwrap().client.is_some())
            .map(|connection| EventSink(connection.clone()))
            .collect()
    }
}
