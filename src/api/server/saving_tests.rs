use super::*;
use std::{future::{poll_fn, Future}, io::Write, task::Poll};

#[test]
fn saving_controls_survive_the_old_deadline_without_claiming_durability() {
    let root = std::env::temp_dir().join(format!("fbt-saving-{}-{}", std::process::id(), now()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::native_runtime::NativeRuntime::new().unwrap();
    for (op, args) in [
        ("torrents.start", json!({"hashes":["a".repeat(40)]})),
        ("torrents.stop", json!({"hashes":["a".repeat(40)]})),
        ("app.settings.set", json!({"network_interface":"lo", "upload":"off"})),
        ("transfer.set_alt", json!({"on":true})),
        ("transfer.set_limits", json!({"download_bps":16384})),
        ("torrents.set_download_limit", json!({"hashes":["a".repeat(40)],"bps":16384})),
    ] {
        let (connection, _peer) = super::tests::connection();
        let request = decode(&serde_json::to_vec(&json!({"id":1,"op":op,"args":args})).unwrap()).unwrap();
        let generation = connection.reserve(&request).unwrap();
        connection.pending.lock().unwrap().get_mut(&1).unwrap().deadline = Instant::now() - Duration::from_secs(60);
        connection.expire();
        assert!(connection.pending.lock().unwrap().contains_key(&1), "{op} must not falsely expire");
        let reference = MutationRef::new("saving-control").unwrap();
        let (ticket, _) = mutations::Ring::default().claim("saving-test", reference.clone(), &request.call).unwrap();
        ticket.attach(mutations::Waiter { connection:1,id:1,generation }, |_| panic!("not admitted yet"));
        let at = Instant::now();
        let mut task = Background {
            ticket:ticket.clone(), op:op.into(), deadline:None, saving_at:Some(at),
            confirmation:None, handler_add:false,
            _activity:Arc::new(crate::daemon::policy::Mutation::start(Arc::new(Mutex::new(
                crate::daemon::policy::Policy::new(0,None,at))))),
        };
        assert!(task.saving_reply(at-Duration::from_nanos(1)).is_none());
        let admission = task.saving_reply(at).unwrap();
        assert!(task.saving_reply(at+Duration::from_secs(120)).is_none(), "one admission");
        let mut file = std::fs::File::create(root.join("config.tmp")).unwrap();
        file.write_all(b"network_interface=lo\nupload=off\n").unwrap();
        let (gate, mut release) = UnixStream::pair().unwrap();
        let sync = crate::durable_sync::FileSync::gated_for_test(&file, &gate).unwrap();
        runtime.block_on(async {
            let mut durability = std::pin::pin!(sync.wait(runtime.cx()));
            poll_fn(|cx| {assert!(durability.as_mut().poll(cx).is_pending());Poll::Ready(())}).await;
            ticket.answer(Ok(admission), |delivery| {
                let mutations::Delivery::Response(waiter, answer) = delivery else {panic!("no completion before save")};
                assert!(connection.complete(waiter.id,waiter.generation,answer));
            });
            let frame = connection.out.lock().unwrap().frames.pop_front().unwrap();
            assert_eq!(frame["result"],json!({"state":"pending","ref":reference,"saving":true}));
            Reply::decode(op,frame["result"].clone()).unwrap();
            assert!(!root.join("config").exists(), "admission must never publish unsafe policy");
            poll_fn(|cx| {assert!(durability.as_mut().poll(cx).is_pending());Poll::Ready(())}).await;
            connection.expire();
            assert!(connection.out.lock().unwrap().frames.is_empty(), "no false timeout behind held fsync");
            release.write_all(b"durable now").unwrap();
            durability.await.unwrap();
        });
        std::fs::rename(root.join("config.tmp"),root.join("config")).unwrap();
        std::fs::File::open(&root).unwrap().sync_all().unwrap();
        let event = Event::MutationResult {seq:1,ts:now(),reference,op:op.into(),outcome:MutationOutcome::Done,error:None,result:None};
        let mut completions=0;
        ticket.finish(event, |delivery| {
            assert!(root.join("config").exists());
            assert!(matches!(delivery, mutations::Delivery::Event(1,Event::MutationResult{outcome:MutationOutcome::Done,..})));
            completions+=1;
        });
        assert_eq!(completions,1);
        std::fs::remove_file(root.join("config")).unwrap();
    }
    runtime.shutdown_timeout(Duration::from_secs(2)).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn saving_is_only_an_admission_for_supported_controls() {
    let result=json!({"state":"pending","ref":"saving","saving":true});
    assert!(Reply::decode("app.settings.set",result.clone()).is_ok());
    assert!(Reply::decode("torrents.start",result.clone()).is_ok());
    assert!(Reply::decode("app.info",result.clone()).is_err());
    let mut invalid=result;
    invalid["saving"]=json!(false);
    assert!(Reply::decode("app.settings.set",invalid).is_err());
}

#[test]
fn slow_controls_preserve_queue_order_while_reads_can_pass() {
    let pool=Pool::default();
    let (connection,_peer)=super::tests::connection();
    let connection=Arc::new(connection);
    for (id,op,args) in [
        (1,"torrents.start",json!({"hashes":["a".repeat(40)]})),
        (2,"torrents.stop",json!({"hashes":["a".repeat(40)]})),
        (3,"app.info",json!({})),
        (4,"app.settings.set",json!({"upload":"off"})),
    ] {
        assert!(pool.submit(Job {connection:connection.clone(),
            request:decode(&serde_json::to_vec(&json!({"id":id,"op":op,"args":args})).unwrap()).unwrap(),
            generation:id,mutation:None,_activity:None}).is_ok());
    }
    let first=pool.next().unwrap();
    assert_eq!(first.request.id.get(),1);
    let turn=ControlPermit(&pool);
    assert_eq!(pool.next().unwrap().request.id.get(),3, "a read passes the held write");
    assert!(pool.work.lock().unwrap().control_busy);
    drop(turn);
    assert_eq!(pool.next().unwrap().request.id.get(),2, "stop cannot overtake start");
    drop(ControlPermit(&pool));
    assert_eq!(pool.next().unwrap().request.id.get(),4, "settings retains its admission order");
    drop(ControlPermit(&pool));
    pool.close();
    assert!(pool.next().is_none());
}
