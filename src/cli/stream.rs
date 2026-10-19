//! Bounded event replay/follow, with explicit instance/sequence resume identity.
use crate::{
    agent::{AppError, Code},
    api::{self, client::Client},
    config::Config,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
const KINDS: &[&str] = &[
    "engine.changed",
    "torrent.added",
    "torrent.completed",
    "torrent.playable",
    "torrent.error",
    "mutation.result",
    "search.done",
    "shows.checked",
    "clipboard.magnet",
];
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}
struct Interrupt(libc::sighandler_t);
impl Interrupt {
    fn install() -> Self {
        INTERRUPTED.store(false, Ordering::Relaxed);
        Self(unsafe { libc::signal(libc::SIGINT, interrupt as *const () as usize) })
    }
}
impl Drop for Interrupt {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.0);
        }
    }
}
fn output(mut event: Value, instance: &api::Instance) -> Result<()> {
    event["instance"] = json!(instance);
    println!("{event}");
    std::io::stdout().flush()?;
    Ok(())
}
pub(crate) fn events(
    cfg: &Config,
    follow: bool,
    since: Option<u64>,
    kinds: Vec<String>,
) -> Result<Code> {
    if kinds.iter().any(|kind| !KINDS.contains(&kind.as_str())) {
        return Err(AppError::new(Code::Usage, "bad event kind")
            .hint(format!("valid kinds: {}", KINDS.join(",")))
            .into());
    }
    let _interrupt = Interrupt::install();
    let client = Client::connect(cfg, true)?;
    client.limit_reconnects(0);
    let mut args = json!({"topics":["events"]});
    if let Some(seq) = since {
        args["since_seq"] = json!(seq);
    }
    client.call("subscribe", args)?;
    let mut revisions = api::validation::Revisions::default();
    let mut seen = since.unwrap_or(0);
    while !INTERRUPTED.load(Ordering::Relaxed) {
        let Some(frame) = client.event(Duration::from_millis(100))? else {
            continue;
        };
        let kind = frame["ev"].as_str().unwrap_or("");
        if matches!(kind, "bye" | "client.disconnected") {
            return Ok(Code::Ok);
        }
        if let (Some(sub), Some(rev)) = (frame["sub"].as_u64(), frame["rev"].as_u64()) {
            revisions
                .observe(sub, rev)
                .map_err(|error| AppError::new(Code::Backend, error.to_string()))?;
        }
        if kind == "snapshot.begin" && frame["reason"] == "resync" {
            let oldest = frame["oldest_seq"].as_u64().unwrap_or(1);
            seen = oldest.saturating_sub(1);
            output(
                json!({"ev":"resync","reason":"resync","oldest_seq":oldest,"seq":seen}),
                &client.hello.instance,
            )?;
        }
        if matches!(kind, "snapshot.chunk" | "delta") {
            let items = frame
                .get("items")
                .or_else(|| frame.get("added"))
                .and_then(Value::as_array);
            for entry in items.into_iter().flatten() {
                if let Some(seq) = entry["seq"].as_u64().filter(|seq| *seq > seen) {
                    seen = seq;
                    if kinds.is_empty() || kinds.iter().any(|kind| entry["ev"] == kind.as_str()) {
                        output(entry.clone(), &client.hello.instance)?;
                    }
                }
            }
        }
        if kind == "snapshot.end" && !follow {
            return Ok(Code::Ok);
        }
    }
    Ok(Code::Ok)
}
