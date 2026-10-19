use super::*;
#[test]
fn restart_completion_obeys_the_pending_only_owner_reply_contract() {
    use crate::api::validation::Validate;
    let mut event = Event::MutationResult {
        seq: 1,
        ts: 1,
        reference: MutationRef::new("restart-fixture").unwrap(),
        op: "app.restart".into(),
        outcome: MutationOutcome::Done,
        error: None,
        result: Some(serde_json::Map::from_iter([("state".into(), json!("done"))])),
    };
    assert!(event.validate().is_err(), "the old invented done payload is invalid");
    let Event::MutationResult { result, .. } = &mut event else { unreachable!() };
    *result = completed_payload("app.restart", &Reply::TorrentsSetUploadLimit(MutationResult::Done));
    assert!(result.is_none());
    event.validate().unwrap();
    let roundtrip: Event = serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
    assert_eq!(roundtrip, event);
    for op in ["app.restart", "app.shutdown"] {
        assert!(completed_payload(op, &Reply::TorrentsSetUploadLimit(MutationResult::Done)).is_none());
    }
    let payload = completed_payload("torrents.set_upload_limit", &Reply::TorrentsSetUploadLimit(MutationResult::Done)).unwrap();
    assert_eq!(payload, serde_json::Map::from_iter([("state".into(), json!("done"))]));
    println!("owner completion keeps outcome=done, omits optional pending-only result, and round-trips the frozen event type; synchronous result payloads remain intact");
}
#[test]
fn dispatch_queue_is_64_and_closed_pool_refuses_work() {
    let pool = Pool::default();
    let (connection, _) = super::tests::connection();
    let connection = Arc::new(connection);
    let job = || Job {
        connection: connection.clone(),
        request: decode(br#"{"id":1,"op":"app.info"}"#).unwrap(),
        generation: 1,
        mutation: None,
    };
    for _ in 0..64 {
        assert!(pool.submit(job()).is_ok());
    }
    assert!(pool.submit(job()).is_err());
    assert!(pool.next().is_some());
    assert!(pool.submit(job()).is_ok());
    pool.close();
    assert!(pool.submit(job()).is_err());
    let mut remaining = 0;
    while pool.next().is_some() {
        remaining += 1;
    }
    assert_eq!(remaining, 64);
    println!("dispatch queue: 64 admitted; 65th refused; consuming frees capacity; close drains accepted work only");
}
#[test]
fn completion_after_deadline_is_timeout_without_writer_poll() {
    let (connection, _) = super::tests::connection();
    let request = decode(br#"{"id":1,"op":"app.info"}"#).unwrap();
    let generation = connection.reserve(&request).unwrap();
    connection
        .pending
        .lock()
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .deadline = Instant::now();
    connection.complete(1, generation, Ok(Reply::AppClient(EmptyResult {})));
    assert_eq!(
        connection.out.lock().unwrap().frames[0]["error"]["code"],
        "timeout"
    );
    assert!(connection.pending.lock().unwrap().is_empty());
    println!("completion at/after deadline yields timeout even before writer's timer poll");
}

#[test]
fn file_removal_has_no_operation_timeout() {
    let (connection, _) = super::tests::connection();
    let request = decode(&serde_json::to_vec(&json!({"id":1,"op":"torrents.remove","args":{"hashes":["a".repeat(40)],"delete_files":true}})).unwrap()).unwrap();
    let generation = connection.reserve(&request).unwrap();
    connection
        .pending
        .lock()
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .deadline = Instant::now() - Duration::from_secs(60);
    connection.expire();
    assert!(connection.pending.lock().unwrap().contains_key(&1));
    assert!(connection.complete(
        1,
        generation,
        Ok(Reply::TorrentsRemove(RemoveResult {
            status: MutationResult::Done,
            undo_until: None
        }))
    ));
    assert_eq!(
        connection.out.lock().unwrap().frames.back().unwrap()["result"]["state"],
        "done"
    );
}

#[test]
fn recheck_pending_has_no_operation_deadline_and_is_not_idempotent() {
    let (connection, _) = super::tests::connection();
    let request = decode(br#"{"id":1,"op":"torrents.recheck","args":{"hashes":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}}"#).unwrap();
    let generation = connection.reserve(&request).unwrap();
    connection.pending.lock().unwrap().get_mut(&1).unwrap().deadline = Instant::now()-Duration::from_secs(60);
    assert!(connection.pending.lock().unwrap()[&1].no_timeout);
    assert!(connection.complete(1,generation,Ok(Reply::TorrentsRecheck(RecheckReply::Pending(PendingResult {
        state:PendingState::Pending,reference:MutationRef::new("recheck-no-timeout").unwrap(),
    })))));
    let out=connection.out.lock().unwrap();
    assert_eq!(out.frames[0]["result"]["state"],"pending");
    assert!(ops::TABLE.iter().any(|row|row.name=="torrents.recheck" && row.mutating && !row.idempotent));
}
