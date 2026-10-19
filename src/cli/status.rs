//! Status observes the owner's lifetime even when its engine is draining.
use super::{native, offline};
use crate::{agent::Code, api::{self, client::Client}, config::Config};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{io::ErrorKind, time::Duration};

#[derive(Debug)]
struct Stopping;
impl std::fmt::Display for Stopping {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str("daemon is stopping")}
}
impl std::error::Error for Stopping {}

fn gone(error:&anyhow::Error)->bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error|matches!(error.kind(),
        ErrorKind::NotFound|ErrorKind::ConnectionRefused|ErrorKind::ConnectionReset|ErrorKind::UnexpectedEof|ErrorKind::BrokenPipe))
}
fn read(client:&Client,op:&str,args:Value)->Result<Value> {
    let frame=client.envelope(op,args,None)?;
    if frame["ok"]==true {return frame.get("result").cloned().context("API success omitted result");}
    let failure:api::Error=serde_json::from_value(frame["error"].clone()).context("invalid API failure")?;
    // Keep the typed state: the public error message alone says only
    // "engine unavailable", which cannot distinguish a restart from a stop.
    if failure.code==api::ErrorCode::EngineUnavailable
        &&failure.details.as_ref().is_some_and(|details|details.get("state")==Some(&json!("stopping"))) {
        return Err(Stopping.into());
    }
    Err(api::errors::app_error(failure))
}
fn draining(cfg:&Config)->Value {
    // Do not fabricate engine counts, rates or a torrent list after quiesce.
    json!({"ok":true,"running":true,"backend":"native","engine":cfg.engine,
        "connection":"api","daemon":{"state":"stopping"}})
}
fn stopped(cfg:&Config)->Result<Value> {
    // The listener closes before the final ledger/statistics save. Observe
    // that save under the owner lock, within the daemon's shutdown budget.
    let rows=offline::list_after(Duration::from_secs(10))?;
    Ok(json!({"ok":true,"running":false,"app_open":false,"backend":"stopped","engine":cfg.engine,
        "total":rows.len(),"all_time":crate::daemon::stats::read(&crate::daemon::stats::path())?}))
}
fn collect(client:&Client,cfg:&Config)->Result<Value> {
    let rows:Vec<api::Torrent>=serde_json::from_value(read(client,"torrents.list",json!({}))?["torrents"].clone())?;
    let info=read(client,"app.info",json!({}))?;
    let transfer=read(client,"transfer.info",json!({}))?;
    let stats=read(client,"app.stats",json!({}))?;
    let summary=native::summary_with(client,cfg,|op,args|read(client,op,args))?;
    let mut value=native::counts(&rows);
    value["ok"]=json!(true);value["running"]=json!(true);value["all_time"]=stats["all_time"].clone();
    value["session"]=stats["session"].clone();
    value["connection"]=json!("api");value["backend"]=json!("native");value["engine"]=json!(cfg.engine);
    if let Some(history)=summary.as_ref().and_then(|s|s.get("history")) {value["history"]=history.clone();}
    value["app_open"]=json!(summary.as_ref().is_some_and(|v|v["daemon"]["busy_reasons"].as_array()
        .is_some_and(|reasons|reasons.iter().any(|reason|reason=="interactive"))));
    value["dl"]=json!(transfer["dl_bps"].as_u64().unwrap_or(0));
    value["up"]=json!(transfer["ul_bps"].as_u64().unwrap_or(0));
    value["daemon"]=summary.as_ref().map(|v|v["daemon"].clone()).unwrap_or_else(||native::daemon_status(&info));
    value["torrents"]=json!(super::shapes::rows(&rows));
    Ok(value)
}
fn live(client:&Client,cfg:&Config)->Result<Value> {
    // Status never retries a read through ensure_daemon. A lost connection is
    // probed explicitly below, and only a confirmed closed owner is read offline.
    client.limit_reconnects(0);
    if client.hello.state==api::DaemonState::Stopping {return Ok(draining(cfg));}
    match collect(client,cfg) {
        Err(error) if error.is::<Stopping>()=>Ok(draining(cfg)),
        result=>result,
    }
}
pub(super) fn run(cfg:&Config,json_mode:bool,quiet:bool)->Result<Code> {
    let value=match Client::connect(cfg,json_mode) {
        Ok(_) if quiet=>return Ok(Code::Ok),
        Ok(client)=>match live(&client,cfg) {
            Ok(value)=>value,
            Err(error) if gone(&error)||crate::agent::classify(&error).0==Code::Backend=>{
                drop(client);
                // One read-only probe covers bye/EOF between any two status
                // requests. Permission/protocol failures stay failures.
                match Client::connect(cfg,json_mode) {
                    Ok(client) if client.hello.state==api::DaemonState::Stopping=>draining(cfg),
                    Err(probe) if gone(&probe)=>stopped(cfg)?,
                    _=>return Err(error),
                }
            }
            Err(error)=>return Err(error),
        },
        Err(error) if gone(&error)&&quiet=>return Ok(Code::Backend),
        Err(error) if gone(&error)=>stopped(cfg)?,
        Err(error)=>return Err(error),
    };
    if quiet {return Ok(if value["running"]==true {Code::Ok}else{Code::Backend});}
    native::emit(json_mode,value);
    Ok(Code::Ok)
}

#[cfg(test)]
#[path="status_tests.rs"] mod tests;
