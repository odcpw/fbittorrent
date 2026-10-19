//! Read-only log history/follow. A disconnected log reader never starts an owner.
use crate::{
    agent::{AppError, Code},
    api::{client::Client, LogArgs, LogEntry},
    config::Config,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::{io::Write, time::Duration};
pub(crate) fn run(
    cfg: &Config,
    json_mode: bool,
    follow: bool,
    level: Option<String>,
    source: Option<String>,
) -> Result<Code> {
    let client = Client::connect(cfg, json_mode).map_err(|_| {
        AppError::new(
            Code::Backend,
            "daemon is not running; its log ring is unavailable",
        )
        .hint("fbittorrent up   # or: journalctl --user -u fbittorrent")
    })?;
    client.limit_reconnects(0);
    let mut args = json!({});
    if let Some(level) = level {
        args["level"] = json!(level);
    }
    if let Some(source) = source {
        args["source"] = json!(source);
    }
    let filters: LogArgs = serde_json::from_value(args.clone())?;
    let result = client.call("app.log", args)?;
    if !follow {
        if json_mode {
            crate::emit(
                true,
                json!({"ok":true,"entries":result["entries"],"next_seq":result["next_seq"]}),
                || {},
            );
        } else {
            for entry in result["entries"].as_array().unwrap() {
                println!(
                    "{} {} {}",
                    entry["level"].as_str().unwrap(),
                    entry["source"].as_str().unwrap(),
                    entry["message"].as_str().unwrap()
                );
            }
        }
        return Ok(Code::Ok);
    }
    let mut seen = result["next_seq"].as_u64().unwrap_or(0);
    for entry in result["entries"].as_array().unwrap() {
        println!("{entry}");
    }
    std::io::stdout().flush()?;
    client.call("subscribe", json!({"topics":["log"]}))?;
    loop {
        if let Some(event) = client.event(Duration::from_secs(1))? {
            match event["ev"].as_str() {
                Some("bye" | "client.disconnected") => return Ok(Code::Ok),
                Some("snapshot.chunk" | "delta") => {
                    let items = event
                        .get("items")
                        .or_else(|| event.get("added"))
                        .and_then(Value::as_array);
                    for value in items.into_iter().flatten() {
                        let entry: LogEntry = serde_json::from_value(value.clone())?;
                        if entry.seq > seen {
                            seen = entry.seq;
                            if crate::daemon::log::matches(&entry, &filters) {
                                println!("{value}");
                                std::io::stdout().flush()?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
