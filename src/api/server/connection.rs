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

impl Connection {
    pub(super) fn send(&self, frame: Frame) {
        let mut out = self.out.lock().unwrap();
        if out.ending {
            return;
        }
        if out.frames.len() >= OUTGOING {
            out.frames.clear();
            out.ending = true;
            out.terminal = Some(bye(ByeReason::SlowClient));
            let _ = self.stream.shutdown(Shutdown::Read);
        } else {
            out.frames.push_back(frame);
        }
        self.ready.notify_one();
    }
    pub(super) fn finish(&self, reason: ByeReason) {
        let mut out = self.out.lock().unwrap();
        if !out.ending {
            out.ending = true;
            out.terminal = Some(bye(reason));
        }
        self.ready.notify_one();
    }
    pub(super) fn protocol(&self, failure: Error, reason: ByeReason) {
        self.send(json!({"ev":"error","error":failure}));
        self.finish(reason);
    }
    pub(super) fn reserve(&self, request: &Request) -> Result<u64, Invalid> {
        self.expire();
        let id = request.id.get();
        let mut pending = self.pending.lock().unwrap();
        if pending.contains_key(&id) {
            return Err(Invalid {
                id: None,
                error: error(
                    ErrorCode::InvalidArgs,
                    "id is already in flight",
                    Some("id"),
                ),
                close: false,
            });
        }
        if pending.len() >= INFLIGHT {
            return Err(Invalid {
                id: Some(id),
                error: busy("inflight"),
                close: false,
            });
        }
        let generation = self.sequence.fetch_add(1, Ordering::Relaxed);
        let after = if matches!(request.call, Call::TorrentsAdd { .. }) {
            95
        } else {
            15
        };
        pending.insert(
            id,
            Pending {
                generation,
                deadline: Instant::now() + Duration::from_secs(after),
                no_timeout: matches!(&request.call, Call::TorrentsRecheck { .. }) || matches!(&request.call, Call::TorrentsRemove { args } if args.delete_files),
                after,
            },
        );
        Ok(generation)
    }
    pub(super) fn complete(&self, id: u64, generation: u64, result: ApiResult<Reply>) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if !pending
            .get(&id)
            .is_some_and(|entry| entry.generation == generation)
        {
            return false;
        }
        let entry = pending.remove(&id).unwrap();
        drop(pending);
        let timely = entry.no_timeout || entry.deadline > Instant::now();
        let result = if !timely {
            let mut failure = error(
                ErrorCode::Timeout,
                "operation timed out; outcome is unknown",
                None,
            );
            failure.details = Some(serde_json::Map::from_iter([(
                "after_s".into(),
                json!(entry.after),
            )]));
            Err(failure)
        } else {
            result
        };
        let frame = match result {
            Ok(result) => json!({"id":id,"ok":true,"result":result}),
            Err(failure) => response(Some(id), failure),
        };
        // A collection that cannot fit is a request failure, not malformed
        // client input. Keep its ID and the connection usable (P1-06 ruling 2).
        let frame = if encode_frame(&frame).is_err() {
            let mut failure = error(
                ErrorCode::InvalidMessage,
                "response exceeds the 1048576-byte frame cap",
                None,
            );
            failure.hint =
                Some("request a smaller collection; chunked subscriptions follow in P1-08".into());
            response(Some(id), failure)
        } else {
            frame
        };
        self.send(frame);
        timely
    }
    pub(super) fn expire(&self) {
        let mut pending = self.pending.lock().unwrap();
        let expired: Vec<_> = pending
            .iter()
            .filter(|(_, entry)| !entry.no_timeout && entry.deadline <= Instant::now())
            .map(|(id, entry)| (*id, entry.after))
            .collect();
        for (id, after) in expired {
            pending.remove(&id);
            let mut failure = error(
                ErrorCode::Timeout,
                "operation timed out; outcome is unknown",
                None,
            );
            failure.details = Some(serde_json::Map::from_iter([(
                "after_s".into(),
                json!(after),
            )]));
            self.send(response(Some(id), failure));
        }
    }
}
pub(super) fn read_client(shared: Arc<Shared>, connection: Arc<Connection>, stream: UnixStream) {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    let mut staged: Vec<Job> = Vec::new();
    loop {
        if !reader.buffer().contains(&b'\n') {
            for job in staged.drain(..) {
                if let Err(job) = shared.pool.submit(job) {
                    shared.answer(
                        &job,
                        Err(if shared.stopping.load(Ordering::Acquire) {
                            stopping()
                        } else {
                            busy("dispatch")
                        }),
                    );
                }
            }
        }
        if connection.closed.load(Ordering::Acquire) {
            break;
        }
        if connection.client.lock().unwrap().client.is_none()
            && connection.accepted.elapsed() >= HELLO
        {
            connection.protocol(
                error(
                    ErrorCode::HelloRequired,
                    "hello required within five seconds of accept",
                    None,
                ),
                ByeReason::HelloRequired,
            );
            return;
        }
        let available = match reader.fill_buf() {
            Ok([]) => {
                if !bytes.is_empty() {
                    connection.protocol(
                        error(ErrorCode::InvalidMessage, "incomplete JSON line", None),
                        ByeReason::InvalidMessage,
                    );
                    return;
                }
                break;
            }
            Ok(bytes) => bytes,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => break,
        };
        let length = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(available.len());
        if bytes.len() + length > LINE_IN {
            connection.protocol(
                line_overflow(bytes.len() + length),
                ByeReason::InvalidMessage,
            );
            return;
        }
        bytes.extend_from_slice(&available[..length]);
        reader.consume(length);
        if bytes.last() != Some(&b'\n') {
            continue;
        }
        let decoded = decode(&bytes);
        bytes.clear();
        let request = match decoded {
            Ok(request) => request,
            Err(failure) => {
                if failure.close {
                    connection.protocol(failure.error, ByeReason::InvalidMessage);
                    return;
                }
                if failure.error.code == ErrorCode::UnknownOp
                    && connection.client.lock().unwrap().client.is_none()
                {
                    connection.protocol(
                        error(
                            ErrorCode::HelloRequired,
                            "first request must be hello",
                            None,
                        ),
                        ByeReason::HelloRequired,
                    );
                    return;
                }
                connection.send(response(failure.id, failure.error));
                continue;
            }
        };
        let hello_done = connection.client.lock().unwrap().client.is_some();
        if !hello_done {
            if connection.accepted.elapsed() >= HELLO {
                connection.protocol(
                    error(ErrorCode::HelloRequired, "hello deadline expired", None),
                    ByeReason::HelloRequired,
                );
                return;
            }
            let Call::Hello { args } = &request.call else {
                connection.protocol(
                    error(
                        ErrorCode::HelloRequired,
                        "first request must be hello",
                        None,
                    ),
                    ByeReason::HelloRequired,
                );
                return;
            };
            if args.proto != PROTO {
                let mut failure = error(ErrorCode::ProtoUnsupported, "proto unsupported", None);
                failure.details = Some(serde_json::Map::from_iter([(
                    "server_proto".into(),
                    json!(PROTO),
                )]));
                connection.send(response(Some(request.id.get()), failure));
                connection.finish(ByeReason::InvalidMessage);
                return;
            }
            let mut counts = shared.counts.lock().unwrap();
            match args.client.kind {
                ClientKind::Cli => counts.cli += 1,
                ClientKind::Agent => counts.agent += 1,
                ClientKind::Tui => counts.tui += 1,
                ClientKind::Bar => counts.bar += 1,
                ClientKind::Test => counts.test += 1,
            }
            drop(counts);
            let hello = ops::app::hello(&shared.daemon);
            *connection.client.lock().unwrap() = ClientState {
                client: Some(args.client.clone()),
                interactive: args.interactive.unwrap_or(false),
                glance: false,
                caps: validation::NegotiatedCaps::from_hello(&hello),
            };
            connection.send(json!({"id":request.id,"ok":true,"result":hello}));
            continue;
        }
        if matches!(request.call, Call::Hello { .. }) {
            connection.send(response(
                Some(request.id.get()),
                error(ErrorCode::InvalidState, "hello already completed", None),
            ));
            continue;
        }
        let generation = match connection.reserve(&request) {
            Ok(generation) => generation,
            Err(e) => {
                connection.send(response(e.id, e.error));
                continue;
            }
        };
        if let Call::AppClient { args } = &request.call {
            if shared.daemon.is_stopping() {
                connection.complete(request.id.get(), generation, Err(stopping()));
                continue;
            }
            let mut client = connection.client.lock().unwrap();
            if let Some(value) = args.interactive {
                client.interactive = value;
            }
            if let Some(value) = args.glance {
                client.glance = value;
            }
            drop(client);
            connection.complete(
                request.id.get(),
                generation,
                if shared.daemon.is_stopping() {
                    Err(stopping())
                } else {
                    Ok(Reply::AppClient(EmptyResult {}))
                },
            );
            continue;
        }
        let control = matches!(
            request.call,
            Call::AppShutdown { .. } | Call::AppRestart { .. }
        );
        let mut job = Job {
            connection: connection.clone(),
            request,
            generation,
            mutation: None,
        };
        if !shared.prepare_mutation(&mut job) {
            continue;
        }
        // Owner control remains responsive even when all dispatch workers are
        // waiting for upstream work. The owner still performs the drain.
        if control {
            shared.dispatch(job);
            continue;
        }
        staged.push(job);
    }
    // A half-closed reader may still be waiting for its accepted replies. The
    // writer closes after those replies drain (or their deadlines expire).
    connection.read_done.store(true, Ordering::Release);
    connection.ready.notify_one();
}
pub(super) fn write_frame(stream: &mut UnixStream, frame: &Frame) -> io::Result<()> {
    stream.write_all(&encode_frame(frame)?)
}
pub(super) fn encode_frame(frame: &Frame) -> io::Result<Vec<u8>> {
    struct Capped(Vec<u8>);
    impl Write for Capped {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len() + bytes.len() + 1 > LINE_OUT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "output frame exceeds 1 MiB",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = Capped(Vec::new());
    serde_json::to_writer(&mut encoded, frame)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    encoded.0.push(b'\n');
    Ok(encoded.0)
}
pub(super) fn write_client(
    shared: Arc<Shared>,
    connection: Arc<Connection>,
    mut stream: UnixStream,
) {
    let mut last = Instant::now();
    loop {
        connection.expire();
        let disconnected = connection.read_done.load(Ordering::Acquire)
            && connection.pending.lock().unwrap().is_empty()
            && !shared
                .mutation_active
                .lock()
                .unwrap()
                .keys()
                .any(|(number, _)| *number == connection.number);
        let mut out = connection.out.lock().unwrap();
        let (frame, terminal) = if let Some(frame) = out.frames.pop_front() {
            (frame, false)
        } else if let Some(frame) = out.terminal.take() {
            (frame, true)
        } else if connection.closed.load(Ordering::Acquire) || disconnected {
            break;
        } else if last.elapsed() >= Duration::from_secs(15)
            && connection.client.lock().unwrap().client.is_some()
        {
            (json!({"ev":"heartbeat","ts":now()}), false)
        } else {
            let _guard = connection
                .ready
                .wait_timeout(out, Duration::from_millis(25))
                .unwrap();
            continue;
        };
        drop(out);
        match write_frame(&mut stream, &frame) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let _ = write_frame(
                    &mut stream,
                    &json!({"ev":"error","error":error(ErrorCode::InvalidMessage,"output frame exceeds 1 MiB; use chunked subscriptions",None)}),
                );
                let _ = write_frame(&mut stream, &bye(ByeReason::InvalidMessage));
                break;
            }
            Err(_) => {
                // The peer may already be gone or its kernel buffer full. Try
                // once without adding another blocking write deadline.
                let _ = stream.set_nonblocking(true);
                let _ = write_frame(&mut stream, &bye(ByeReason::SlowClient));
                break;
            }
        }
        last = Instant::now();
        if terminal {
            break;
        }
    }
    connection.closed.store(true, Ordering::Release);
    let previous = std::mem::take(&mut *connection.client.lock().unwrap());
    if let Some(client) = previous.client {
        let mut counts = shared.counts.lock().unwrap();
        let count = match client.kind {
            ClientKind::Cli => &mut counts.cli,
            ClientKind::Agent => &mut counts.agent,
            ClientKind::Tui => &mut counts.tui,
            ClientKind::Bar => &mut counts.bar,
            ClientKind::Test => &mut counts.test,
        };
        *count = count.saturating_sub(1);
    }
    connection.pending.lock().unwrap().clear();
    let _ = stream.shutdown(Shutdown::Both);
    shared.clients.lock().unwrap().remove(&connection.number);
    shared.detach_mutation_client(connection.number);
}
