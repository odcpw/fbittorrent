//! An explicit restart never starts an absent owner. Only a new instance counts.
use crate::{agent::{AppError, Code}, api::client::Client, config::Config};
use anyhow::Result;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
pub(crate) fn daemon(cfg: &Config, json_mode: bool) -> Result<Value> {
    let old = Client::connect(cfg,json_mode)?;
    old.limit_reconnects(0);
    let instance = old.hello.instance.clone();
    old.call("app.restart",json!({"scope":"daemon"}))?;
    drop(old);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut observation: String;
    loop {
        match Client::connect(cfg,json_mode) {
            Ok(client) if client.hello.instance != instance => {
                return Ok(json!({"ok":true,"action":"restart","scope":"daemon","count":0,"hashes":[],
                    "instance":client.hello.instance,"server":client.hello.server}));
            }
            Ok(_) => observation = "previous instance is still draining".into(),
            Err(error) => observation = error.to_string(),
        }
        if Instant::now() >= deadline {
            return Err(AppError::new(Code::Timeout,format!("daemon restart did not produce a new hello within 30 s: {observation}"))
                .hint("fbittorrent status --json").into());
        }
        std::thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())));
    }
}
