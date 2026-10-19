use super::*;
use std::{io::{BufRead,BufReader,Write},os::unix::net::UnixListener};

fn hello()->Value {
    include_str!("../../contracts/ipc/v1/hello.ndjson").lines()
        .map(|line|serde_json::from_str::<Value>(line).unwrap())
        .find(|line|line["frame"]["result"]["proto"]==1&&line["dir"]=="s2c").unwrap()["frame"].clone()
}
fn exchange(cut:Option<&str>,state:&str,initially_stopping:bool)->Result<Value> {
    let directory=std::env::temp_dir().join(format!("fbt-status-{}-{}",std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir(&directory).unwrap();let path=directory.join("api.sock");
    let listener=UnixListener::bind(&path).unwrap();let cut=cut.map(str::to_owned);let state=state.to_owned();
    let server=std::thread::spawn(move|| {
        let (mut socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut reader=BufReader::new(socket.try_clone().unwrap());let mut line=String::new();
        let mut send=|frame:&Value|{writeln!(socket,"{frame}").unwrap();socket.flush().unwrap();};
        let mut observed=Vec::new();
        while reader.read_line(&mut line).unwrap()>0 {
            let request:Value=serde_json::from_str(&line).unwrap();line.clear();let op=request["op"].as_str().unwrap();
            if op=="hello" {
                let mut frame=hello();frame["id"]=request["id"].clone();frame["result"]["state"]=json!(if initially_stopping{"stopping"}else{"running"});
                frame["result"]["caps"]=json!(["torrents","subscribe"]);send(&frame);continue;
            }
            observed.push(op.to_owned());
            if cut.as_deref()==Some(op) {
                send(&json!({"id":request["id"],"ok":false,"error":{"code":"engine_unavailable","message":"engine unavailable","retryable":true,"details":{"state":state}}}));
                continue;
            }
            let result=match op {
                "torrents.list"=>json!({"torrents":[]}),
                "app.info"=>json!({"state":"running","upgrade_pending":false}),
                "transfer.info"=>json!({"dl_bps":0,"ul_bps":0}),
                "app.stats"=>json!({"all_time":{"downloaded":1,"uploaded":0,"completed":0},"session":{"downloaded":1,"uploaded":0,"started_at":123}}),
                "subscribe"=>json!({"sub":11}),
                "unsubscribe"=>json!({}),
                other=>panic!("unexpected status request {other}"),
            };
            send(&json!({"id":request["id"],"ok":true,"result":result}));
            if op=="subscribe" {
                let mut summary=include_str!("../../contracts/ipc/v1/subscribe.ndjson").lines()
                    .map(|line|serde_json::from_str::<Value>(line).unwrap())
                    .find(|line|line["frame"]["ev"]=="state"&&line["frame"]["topic"]=="summary").unwrap()["frame"].clone();
                summary["sub"]=json!(11);send(&summary);
            }
        }
        if initially_stopping {assert!(observed.is_empty(),"a stopping hello needs no engine requests");}
        else if let Some(cut)=cut {assert_eq!(observed.last(),Some(&cut),"no reads after the stopping response");}
    });
    let cfg=Config{autostart:false,timeout_secs:2,..Config::default()};
    let client=Client::connect_at(&cfg,true,&path).unwrap();let result=live(&client,&cfg);
    drop(client);server.join().unwrap();std::fs::remove_dir_all(directory).unwrap();result
}
#[test]
fn status_observes_shutdown_between_every_pair_of_requests() {
    for op in ["torrents.list","app.info","transfer.info","app.stats","subscribe","unsubscribe"] {
        let value=exchange(Some(op),"stopping",false).unwrap();
        assert_eq!(value["ok"],true,"{op}");assert_eq!(value["running"],true,"{op}");
        assert_eq!(value["daemon"]["state"],"stopping","{op}");
        for key in ["total","active","dl","up","torrents","all_time","session","app_open"] {
            assert!(value.get(key).is_none(),"{op}: unavailable {key} must not be invented");
        }
    }
}
#[test]
fn status_stopping_hello_does_not_wait_for_or_open_the_engine() {
    assert_eq!(exchange(None,"stopping",true).unwrap()["daemon"]["state"],"stopping");
}
#[test]
fn status_does_not_misreport_engine_restart_as_daemon_shutdown() {
    let error=exchange(Some("transfer.info"),"starting",false).unwrap_err();
    assert_eq!(crate::agent::classify(&error).0,Code::Backend);assert!(!error.is::<Stopping>());
}
#[test]
fn status_running_snapshot_keeps_real_counts_and_statistics() {
    let value=exchange(None,"running",false).unwrap();
    assert_eq!(value["running"],true);assert_eq!(value["total"],0);
    assert_eq!(value["session"],json!({"downloaded":1,"uploaded":0,"started_at":123}));
    assert_eq!(value["all_time"]["downloaded"],1);assert_eq!(value["torrents"],json!([]));
}
