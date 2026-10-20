//! Native catalogue transport shared by the RSS and Shows commands.
use crate::{agent::{AppError,Code},api::{self,client::Client,validation::Validate},config::Config};
use anyhow::{Context,Result};
use serde::de::DeserializeOwned;
use serde_json::{json,Value};
use std::time::{Duration,Instant};

pub(super) fn validate(call:&api::Call)->Result<()> {
    api::Request {id:api::RequestId::new(1).unwrap(),call:call.clone(),reference:None,instance:None}
        .validate().map_err(|error|AppError::new(Code::Usage,error.to_string()))?;
    Ok(())
}
pub(super) fn args(call:&api::Call)->Result<Value> {
    Ok(serde_json::to_value(call)?.get("args").filter(|args|!args.is_null()).cloned().unwrap_or_else(||json!({})))
}
pub(super) fn typed<T:DeserializeOwned>(value:Value)->Result<T> {
    serde_json::from_value(value).context("invalid catalogue result")
}
pub(super) fn write_owner(cfg:&Config,no_start:bool,json:bool)->Result<Client> {
    api::client::ensure_daemon(cfg,no_start||!cfg.autostart)?;
    let client=Client::connect(cfg,json)?;client.limit_reconnects(0);Ok(client)
}
struct Subscription<'a> {client:&'a Client,sub:u64}
impl Drop for Subscription<'_> {fn drop(&mut self){let _=self.client.call("unsubscribe",json!({"sub":self.sub}));}}
fn subscribe<'a>(client:&'a Client,topic:&str)->Result<Subscription<'a>> {
    let sub=client.call("subscribe",json!({"topics":[topic]}))?["sub"].as_u64().context("subscription omitted id")?;
    Ok(Subscription{client,sub})
}
fn unknown(op:&str)->anyhow::Error {
    AppError::new(Code::Timeout,format!("{op} result is unknown; inspect the current list"))
        .hint(format!("fbittorrent {} list --json",op.split('.').next().unwrap_or("shows"))).into()
}
fn receive(client:&Client,deadline:Instant,op:&str)->Result<Option<Value>> {
    if Instant::now()>=deadline{return Err(unknown(op));}
    let value=client.event(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(200))).map_err(|_|unknown(op))?;
    if value.as_ref().is_some_and(|frame|matches!(frame["ev"].as_str(),Some("bye"|"client.disconnected"))){return Err(unknown(op));}
    Ok(value)
}
pub(super) fn collection<T:DeserializeOwned>(client:&Client,topic:&str,timeout:u64)->Result<Vec<T>> {
    let scope=subscribe(client,topic)?;
    let deadline=Instant::now()+Duration::from_secs(timeout.max(5));
    let mut stream=api::client::Subscription::new(client);
    let(mut rows,mut expected)=(Vec::new(),None);
    loop {
        let Some(frame)=receive(client,deadline,topic)?else{continue;};
        if frame["sub"]!=scope.sub || frame["topic"]!=topic{continue;}
        let frame=stream.accept(frame)?;
        match frame["ev"].as_str() {
            Some("snapshot.begin")=>{rows.clear();expected=frame["count"].as_u64();},
            Some("snapshot.chunk")=>{
                let chunk:Vec<T>=typed(frame["items"].clone())?;
                anyhow::ensure!(rows.len()+chunk.len()<=20_000,"catalogue exceeds 20000 rows");rows.extend(chunk);
            },
            Some("snapshot.end")=>{anyhow::ensure!(expected==Some(rows.len() as u64),"catalogue snapshot count differs");return Ok(rows);},
            _=>{},
        }
    }
}
/// Prefer a fresh operation result after durable catalogue edits. Large
/// collections use the bounded subscription transport instead of a large frame.
pub(super) fn fresh<T:DeserializeOwned>(client:&Client,call:&api::Call,key:&str,topic:&str,timeout:u64)->Result<Vec<T>> {
    let frame=client.envelope(call.op(),args(call)?,None)?;
    if frame["ok"]==true{return typed(frame["result"][key].clone());}
    let error:api::Error=typed(frame["error"].clone())?;
    if error.code==api::ErrorCode::InvalidMessage && error.message=="response exceeds the 1048576-byte frame cap" {
        return collection(client,topic,timeout);
    }
    Err(api::errors::app_error(error))
}
fn events(frame:&Value)->impl Iterator<Item=&Value> {
    std::iter::once(frame).chain(frame.get("items").or_else(||frame.get("added")).and_then(Value::as_array).into_iter().flatten())
}
/// Mutations keep their own ref; a lost pending connection is never resubmitted.
/// Checks also observe the pushed shows.checked event beyond the initial history.
pub(super) fn execute(client:&Client,call:&api::Call,timeout:u64)->Result<std::result::Result<Value,api::Error>> {
    validate(call)?;
    let watch=match call {api::Call::RssRefresh{..}=>Some("rss.feeds"),api::Call::ShowsCheck{..}=>Some("events"),_=>None};
    let scope=watch.map(|topic|subscribe(client,topic)).transpose()?;
    let deadline=Instant::now()+Duration::from_secs(timeout.max(45));
    let mut stream=api::client::Subscription::new(client);
    let mut baseline=0;
    if let(Some(scope),Some(topic))=(&scope,watch) {
        loop {
            let Some(frame)=receive(client,deadline,call.op())?else{continue;};
            if frame["sub"]!=scope.sub{continue;}
            let frame=stream.accept(frame)?;
            baseline=baseline.max(events(&frame).filter_map(|entry|entry["seq"].as_u64()).max().unwrap_or(0));
            if frame["topic"]==topic && matches!(frame["ev"].as_str(),Some("snapshot.end"|"state")){break;}
        }
    }
    let value=client.call(call.op(),args(call)?)?;
    if value["state"]!="pending"{return Ok(Ok(value));}
    let reference=value["ref"].as_str().context("pending catalogue operation omitted ref")?.to_owned();
    let(mut completed,mut checked)=(None,!matches!(call,api::Call::ShowsCheck{..}));
    loop {
        let Some(frame)=receive(client,deadline,call.op())?else{continue;};
        if frame["ev"]=="mutation.result" && frame["ref"]==reference {
            completed=Some(if frame["outcome"]=="done"{Ok(frame["result"].clone())}else{Err(typed(frame["error"].clone())?)});
        }
        if scope.as_ref().is_some_and(|scope|frame["sub"]==scope.sub) {
            let frame=stream.accept(frame)?;
            if let api::Call::ShowsCheck{args}=call {
                let selected=args.as_ref().and_then(|args|args.imdb.as_deref());
                checked|=events(&frame).any(|entry|entry["ev"]=="shows.checked" && entry["seq"].as_u64().is_some_and(|seq|seq>baseline) && entry["imdb"].as_str()==selected);
            }
        }
        if checked {if let Some(result)=completed.take(){return Ok(result);}}
    }
}
pub(super) fn success(client:&Client,call:&api::Call,timeout:u64)->Result<Value> {
    execute(client,call,timeout)?.map_err(api::errors::app_error)
}

pub(super) fn saved_torrents()->Result<Vec<api::Torrent>> {
    // Read-only atomic ledger snapshots need no live engine or process lock.
    let ledger=crate::engine::ParkedLedger::new(crate::config::data_dir().join("engine"));
    let crate::engine::ops::EngineResult::Torrents(rows)=ledger.execute(crate::engine::ops::EngineOp::Info)?else{unreachable!()};
    rows.into_iter().map(|row|api::ops::torrents::torrent(row).map_err(api::errors::app_error)).collect()
}
