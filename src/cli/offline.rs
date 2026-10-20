//! Saved reads and offline intents use the daemon's process lock and durable store.
use crate::{
    api,
    config::{self, Config},
    engine::{
        ops::{EngineOp, EngineResult},
        ParkedLedger,
    },
    ipc::InstanceLock,
};
use anyhow::Result;
use std::time::Duration;

pub fn locked() -> Result<(InstanceLock, ParkedLedger)> {
    locked_for(Duration::from_secs(2))
}
fn locked_for(timeout:Duration)->Result<(InstanceLock,ParkedLedger)> {
    let directory = config::runtime_dir();
    std::fs::create_dir_all(&directory)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let lock = InstanceLock::acquire_wait(config::runtime_dir(), timeout)?;
    Ok((
        lock,
        ParkedLedger::new(config::data_dir().join("engine")),
    ))
}
pub fn list() -> Result<Vec<api::Torrent>> {
    list_after(Duration::from_secs(2))
}
pub(super) fn list_after(timeout:Duration)->Result<Vec<api::Torrent>> {
    let (_lock, ledger) = locked_for(timeout)?;
    let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info)? else {
        unreachable!()
    };
    rows.into_iter()
        .map(|row| api::ops::torrents::torrent(row).map_err(api::errors::app_error))
        .collect()
}
/// Resolve and persist under the same owner lock. Two hello/lock rounds bound
/// a simultaneous shutdown without racing its final durable ledger save.
pub fn mutation(
    cfg: &Config,
    selectors: &[String],
    stop: bool,
    json_mode: bool,
) -> Result<(Vec<String>, bool)> {
    for _ in 0..2 {
        if let Ok(client) = api::client::Client::connect(cfg, json_mode) {
            if client.hello.state != api::DaemonState::Stopping {
                if let Ok(value) = client.call("torrents.list", serde_json::json!({})) {
                    let rows: Vec<api::Torrent> =
                        serde_json::from_value(value["torrents"].clone())?;
                    let hashes =
                        crate::cli_sources::resolve_in(&rows, selectors)?;
                    for chunk in hashes.chunks(256) {
                        let args = if stop {
                            serde_json::json!({"hashes":chunk})
                        } else {
                            serde_json::json!({"hashes":chunk,"delete_files":false})
                        };
                        online_mutation(&client,
                            if stop {
                                "torrents.stop"
                            } else {
                                "torrents.remove"
                            },
                            args,
                        )?;
                    }
                    return Ok((hashes, true));
                }
            }
        }
        if let Ok((_lock, ledger)) = locked() {
            let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info)? else {
                unreachable!()
            };
            let rows = rows
                .into_iter()
                .map(api::ops::torrents::torrent)
                .collect::<Result<Vec<_>, _>>()
                .map_err(api::errors::app_error)?;
            let hashes = crate::cli_sources::resolve_in(&rows, selectors)?;
            use crate::engine::ops::HashSelection;
            let selection = HashSelection(hashes.clone());
            ledger.execute(if stop {
                EngineOp::Stop { hashes: selection }
            } else {
                EngineOp::Delete {
                    hashes: selection,
                    files: false,
                }
            })?;
            return Ok((hashes, false));
        }
    }
    Err(crate::agent::AppError::new(
        crate::agent::Code::Backend,
        "daemon is starting or draining; saved intent was not changed",
    )
    .hint("fbittorrent status --json")
    .into())
}

fn online_mutation(client: &api::client::Client, op: &str, args: serde_json::Value) -> Result<()> {
    let reply=client.call(op,args)?;
    if reply["state"]=="pending" { super::native::await_mutation(client,reply)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{BufRead,BufReader,Write},os::unix::{fs::DirBuilderExt,net::UnixListener}};
    use serde_json::{json,Value};

    #[test]
    fn online_stop_does_not_report_success_from_saving_admission() {
        let root=std::env::temp_dir().join(format!("fbt-cli-saving-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path=root.join("api.sock");
        let listener=UnixListener::bind(&path).unwrap();
        let owner=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();
            let mut read=BufReader::new(stream.try_clone().unwrap());
            let mut line=String::new();read.read_line(&mut line).unwrap();
            let hello:Value=serde_json::from_str(&line).unwrap();
            let mut response=include_str!("../../contracts/ipc/v1/hello.ndjson").lines()
                .map(|line|serde_json::from_str::<Value>(line).unwrap())
                .find(|line|line["dir"]=="s2c" && line["frame"]["result"]["proto"]==1).unwrap()["frame"].clone();
            response["id"]=hello["id"].clone();
            writeln!(stream,"{response}").unwrap();
            line.clear();read.read_line(&mut line).unwrap();
            let request:Value=serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"],"torrents.stop");
            writeln!(stream,"{}",json!({"id":request["id"],"ok":true,"result":{
                "state":"pending","ref":request["ref"],"saving":true}})).unwrap();
            // A genuine storage failure after admission must reach the caller.
            // No delay assumption: returning merely because pending arrived is wrong.
            writeln!(stream,"{}",json!({"ev":"mutation.result","seq":1,"ts":1,
                "ref":request["ref"],"op":"torrents.stop","outcome":"failed",
                "error":{"code":"internal","message":"private fixture disk full","retryable":false}})).unwrap();
        });
        let client=api::client::Client::connect_at(&Config::default(),true,&path).unwrap();
        let result=online_mutation(&client,"torrents.stop",json!({"hashes":["a".repeat(40)]}));
        assert!(result.unwrap_err().to_string().contains("private fixture disk full"));
        drop(client);owner.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
