//! Search uses a running capable owner, with the same A.9 source and shapes locally.
use super::{fallback, shapes};
use crate::{
    agent::{AppError, Code},
    api::{self, client::Client},
    config::Config,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(crate) fn running(cfg: &Config, cap: &str) -> Option<Client> {
    if !cfg.native() {
        return None;
    }
    let client = Client::connect(cfg, false).ok()?;
    if client.hello.state != api::DaemonState::Running
        || !client.hello.caps.iter().any(|v| v == cap)
    {
        return None;
    }
    client.limit_reconnects(0);
    Some(client)
}
fn providers(client: Option<&Client>) -> Result<Vec<api::Provider>> {
    match client {
        Some(c) => Ok(serde_json::from_value::<api::ProvidersResult>(
            c.call("search.providers", json!({}))?,
        )?
        .providers),
        None => fallback::providers(),
    }
}
struct Remote<'a> {
    client: &'a Client,
    search: Option<u64>,
    subscriptions: Vec<u64>,
}
impl Drop for Remote<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.search {
            let _ = self.client.call("search.stop", json!({"search_id":id}));
        }
        for sub in &self.subscriptions {
            let _ = self.client.call("unsubscribe", json!({"sub":sub}));
        }
    }
}
fn subscribe(remote: &mut Remote<'_>, topic: &str) -> Result<u64> {
    let result = remote.client.call("subscribe", json!({"topics":[topic]}))?;
    let sub = result["sub"].as_u64().context("subscription omitted sub")?;
    remote.subscriptions.push(sub);
    Ok(sub)
}
fn receive(frames: &mut api::client::Subscription<'_>, timeout: Duration) -> Result<Option<Value>> {
    let Some(frame) = frames.client.event(timeout)? else {
        return Ok(None);
    };
    if matches!(frame["ev"].as_str(), Some("bye" | "client.disconnected")) {
        return Err(AppError::new(Code::Backend, "search owner disconnected").into());
    }
    Ok(Some(frames.accept(frame)?))
}
fn entries(frame: &Value) -> impl Iterator<Item = &Value> {
    frame
        .get("items")
        .or_else(|| frame.get("added"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}
fn remote_search(
    client: &Client,
    query: &str,
    wait: u64,
    timeout: u64,
) -> Result<fallback::Search> {
    let mut remote = Remote {
        client,
        search: None,
        subscriptions: vec![],
    };
    // Listen first: a fast provider may finish before search.start answers.
    subscribe(&mut remote, "events")?;
    let id = client.call("search.start", json!({"query":query}))?["search_id"]
        .as_u64()
        .context("search omitted id")?;
    remote.search = Some(id);
    let mut frames = api::client::Subscription::new(client);
    let deadline = Instant::now() + Duration::from_secs(wait);
    let mut complete = false;
    let mut warnings = Vec::new();
    while Instant::now() < deadline {
        let Some(frame) = receive(
            &mut frames,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        )?
        else {
            continue;
        };
        if let Some(done) = std::iter::once(&frame)
            .chain(entries(&frame))
            .find(|v| v["ev"] == "search.done" && v["search_id"] == id)
        {
            warnings = serde_json::from_value(done["warnings"].clone())?;
            complete = true;
            break;
        };
    }
    // A final atomic snapshot avoids depending on the order of the last search
    // delta and search.done on different subscriptions. The session stays owned.
    let topic = format!("search:{id}");
    let sub = subscribe(&mut remote, &topic)?;
    let deadline = Instant::now() + Duration::from_secs(timeout.max(1));
    let mut items = Vec::new();
    let mut expected = None;
    while Instant::now() < deadline {
        let Some(frame) = receive(
            &mut frames,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        )?
        else {
            continue;
        };
        if frame["sub"] != sub || frame["topic"] != topic {
            continue;
        }
        match frame["ev"].as_str() {
            Some("snapshot.begin") => {
                items.clear();
                expected = frame["count"].as_u64();
            }
            Some("snapshot.chunk") => {
                let chunk: Vec<api::SearchItem> = serde_json::from_value(frame["items"].clone())?;
                anyhow::ensure!(
                    items.len() + chunk.len() <= 5000,
                    "search snapshot exceeds 5000 results"
                );
                items.extend(chunk);
            }
            Some("snapshot.end") => {
                anyhow::ensure!(
                    expected == Some(items.len() as u64),
                    "search snapshot count differs"
                );
                return Ok(fallback::Search {
                    items,
                    complete,
                    warnings,
                });
            }
            _ => {}
        }
    }
    Err(AppError::new(Code::Timeout, "search snapshot timed out").into())
}
fn install(
    client: Option<&Client>,
    cfg: &Config,
    sources: &[String],
) -> Result<api::InstalledResult> {
    let Some(client) = client else {
        return Ok(fallback::install(cfg, sources));
    };
    let result = client.call("search.install", json!({"sources":sources}))?;
    if result["state"] != "pending" {
        return Ok(serde_json::from_value(result)?);
    }
    let reference = result["ref"].clone();
    let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs.max(95));
    while Instant::now() < deadline {
        if let Some(frame) = client.event(Duration::from_millis(100))? {
            if frame["ev"] == "mutation.result" && frame["ref"] == reference {
                if frame["outcome"] != "done" {
                    return Err(api::errors::app_error(serde_json::from_value(
                        frame["error"].clone(),
                    )?));
                }
                return Ok(serde_json::from_value(frame["result"].clone())?);
            }
        }
    }
    Err(AppError::new(
        Code::Timeout,
        "provider installation timed out; outcome unknown",
    )
    .into())
}
pub(crate) fn run(
    cfg: &Config,
    query: Vec<String>,
    wait: u64,
    limit: usize,
    curated: bool,
    sources: Vec<String>,
    plugins: bool,
    json_mode: bool,
) -> Result<Code> {
    let client = if sources
        .iter()
        .any(|source| url::Url::parse(source).is_ok_and(|url| url.scheme() != "https"))
    {
        None
    } else {
        running(cfg, "search")
    };
    if plugins {
        let ps = shapes::providers(&providers(client.as_ref())?);
        crate::emit(
            json_mode,
            json!({"ok":true,"count":ps.len(),"enabled":ps.iter().filter(|p|p.enabled).count(),"plugins":ps}),
            || {
                for p in &ps {
                    println!(
                        "{:<18} {:<3} {:<8} {}",
                        p.name,
                        if p.enabled { "on" } else { "off" },
                        p.version,
                        p.url
                    );
                }
            },
        );
        return Ok(Code::Ok);
    }
    if curated || !sources.is_empty() {
        let mut sources = sources;
        if curated {
            let base="https://raw.githubusercontent.com/qbittorrent/search-plugins/master/nova3/engines/";
            sources.extend(
                [
                    "eztv",
                    "limetorrents",
                    "piratebay",
                    "solidtorrents",
                    "torlock",
                    "torrentproject",
                    "torrentscsv",
                ]
                .map(|n| format!("{base}{n}.py")),
            );
            sources.extend(
                crate::qbt::COMMUNITY_SEARCH_PLUGINS
                    .iter()
                    .map(|s| s.to_string()),
            );
        }
        let result = install(client.as_ref(), cfg, &sources)?;
        if result.installed.is_empty() && !result.failed.is_empty() {
            return Err(AppError::new(Code::Upstream, "all provider installations failed").into());
        }
        let ps = providers(client.as_ref())?;
        for p in &ps {
            if let Some(c) = client.as_ref() {
                c.call("search.set_provider", json!({"name":p.name,"enabled":true}))?;
            } else {
                crate::search::set_enabled(&p.name, true)?;
            }
        }
        crate::emit(
            json_mode,
            json!({"ok":true,"action":"install-plugins","count":ps.len(),"plugins":ps.iter().map(|p|&p.name).collect::<Vec<_>>()}),
            || {
                println!(
                    "{} search plugins installed: {}",
                    ps.len(),
                    ps.iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        );
        return Ok(Code::Ok);
    }
    let query = query.join(" ");
    if query.trim().is_empty() {
        return Err(AppError::new(Code::Usage, "no search query given")
            .hint("fbittorrent search \"<title>\" --json")
            .into());
    }
    let ps = providers(client.as_ref())?;
    if ps.is_empty() {
        return Err(
            AppError::new(Code::Backend, "no search providers installed")
                .hint("fbittorrent search --install")
                .into(),
        );
    }
    if !ps.iter().any(|p| p.enabled) {
        return Err(
            AppError::new(Code::Usage, "every search provider is disabled")
                .hint("fbittorrent search --plugins")
                .into(),
        );
    }
    let mut result = match client.as_ref() {
        Some(c) => remote_search(c, &query, wait, cfg.timeout_secs)?,
        None => fallback::search(&query, wait)?,
    };
    if result.complete && result.items.is_empty() && !result.warnings.is_empty() {
        for warning in &result.warnings {
            eprintln!("WARN search {}: {}",warning.provider,warning.message);
        }
        return Err(AppError::new(Code::Upstream, "all search providers failed")
            .hint("fbittorrent search --plugins")
            .into());
    }
    result.items.sort_by(|a, b| {
        b.seeds
            .cmp(&a.seeds)
            .then(a.name.cmp(&b.name))
            .then(a.providers.cmp(&b.providers))
    });
    result.items.truncate(limit);
    let rows: Vec<_> = result
        .items
        .iter()
        .map(|r| shapes::search(r, &ps))
        .collect();
    let mut value = json!({"ok":true,"query":query,"complete":result.complete,"count":rows.len(),"results":rows});
    if client.is_some() && !result.warnings.is_empty() {
        value["warnings"] = json!(result.warnings);
    }
    crate::emit(json_mode, value, || {
        for item in &result.items {
            let row = shapes::search_legacy(item, &ps);
            println!(
                "{:>5}S {:>5}L  {:>10}  {:<12}  {}",
                row.seeders,
                row.leechers,
                crate::human_bytes(row.file_size),
                row.engine,
                row.file_name
            );
        }
    });
    Ok(Code::Ok)
}
