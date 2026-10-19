//! Torrent CLI execution on API v1; catalogue commands use the shared catalog_* handlers.
use super::*;
use crate::api::{self, client::Client as ApiClient};
use crate::{
    agent::{AppError, Code},
    config::Config,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub(crate) fn handles(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Triage
            | Cmd::Status { .. }
            | Cmd::List { .. }
            | Cmd::Show { .. }
            | Cmd::Add { .. }
            | Cmd::AddClipboard
            | Cmd::Start { .. }
            | Cmd::Stop { .. }
            | Cmd::Reannounce { .. }
            | Cmd::Move { .. }
            | Cmd::Delete { .. }
            | Cmd::Seed { .. }
            | Cmd::Noseed { .. }
            | Cmd::Wait { .. }
            | Cmd::Open { .. }
            | Cmd::Rename {..}
            | Cmd::Limit { .. }
            | Cmd::Restart
    )
}
fn connect(cfg: &Config, write: bool, no_start: bool, json: bool) -> Result<ApiClient> {
    match ApiClient::connect(cfg, json) {
        Ok(client) => {
            Ok(client)
        }
        Err(error) => {
            if !write {
                return Err(error);
            }
            api::client::ensure_daemon(cfg, no_start || !cfg.autostart)?;
            let client = ApiClient::connect(cfg, json)?;
            Ok(client)
        }
    }
}
fn list(client: &ApiClient) -> Result<Vec<api::Torrent>> {
    Ok(serde_json::from_value(
        client.call("torrents.list", json!({}))?["torrents"].clone(),
    )?)
}
fn selected(rows: &[api::Torrent], selectors: &[String]) -> Result<Vec<String>> {
    crate::cli_sources::resolve_in(&shapes::legacy(rows)?, selectors)
}
pub(super) fn emit(json_mode: bool, value: Value) {
    crate::emit(json_mode, value.clone(), || {
        if value["daemon"]["state"]=="stopping" {
            println!("fbittorrent is stopping; draining downloads");
        } else if value["action"] == "add" {
            println!(
                "{}: {}  ({})",
                if value["deferred"] == true {
                    "waiting for VPN"
                } else {
                    "added"
                },
                value["name"].as_str().unwrap_or("torrent"),
                value["hash"].as_str().unwrap_or("pending")
            );
        } else if value["action"] == "wait" {
            println!(
                "{}",
                if value["complete"] == true { "complete" }
                else if value["stall_free"] == true {"ready to watch"}
                else if value["playable"] == true {"playable"}
                else {value["reason"].as_str().unwrap_or("waiting")}
            );
        } else if value["dry_run"] == true {
            let rows = value["torrents"].as_array();
            println!(
                "would remove {} torrent(s){}",
                rows.map_or(0, Vec::len),
                if value["files"] == true {
                    " AND their files"
                } else {
                    " (files kept)"
                }
            );
            for row in rows.into_iter().flatten() {
                println!(
                    "  {}  {}",
                    row["hash"].as_str().unwrap_or(""),
                    row["name"].as_str().unwrap_or("")
                );
            }
        } else if let Some(action) = value["action"].as_str() {
            println!(
                "{action}: {} torrent(s)",
                value["count"].as_u64().unwrap_or(0)
            );
        } else if value.get("backend").is_some() && value.get("total").is_some() {
            if value["running"] == false {
                println!("fbittorrent is closed; {} saved torrent(s)", value["total"]);
            } else {
                println!(
                    "↓ {}  ↑ {}  | {} total",
                    crate::qbt::human_rate(value["dl"].as_i64().unwrap_or(0)),
                    crate::qbt::human_rate(value["up"].as_i64().unwrap_or(0)),
                    value["total"]
                );
            }
            for (key, label) in [("session", "Session"), ("all_time", "All time")] {
                if let (Some(downloaded), Some(uploaded)) =
                    (value[key]["downloaded"].as_u64(), value[key]["uploaded"].as_u64())
                {
                    if let Some(completed) = value[key]["completed"].as_u64() {
                        println!("{label}: downloaded {downloaded} B; uploaded {uploaded} B; completed {completed}");
                    } else {
                        println!("{label}: downloaded {downloaded} B; uploaded {uploaded} B");
                    }
                }
            }
        } else if let Some(rows) = value["torrents"].as_array() {
            for row in rows {
                let hash = row["hash"].as_str().unwrap_or("");
                let progress = if row["progress_known"] == false {
                    "—".into()
                } else {
                    format!("{:.1}%", 100.0 * row["progress"].as_f64().unwrap_or(0.0))
                };
                println!(
                    "{}  {:>6}  {:<12} {}",
                    &hash[..hash.len().min(8)],
                    progress,
                    row["state"].as_str().unwrap_or("unknown"),
                    row["name"].as_str().unwrap_or("")
                );
            }
        } else if value.get("hash").is_some() {
            println!(
                "{}\n  hash  {}\n  state {}\n  open  {}",
                value["name"].as_str().unwrap_or("torrent"),
                value["hash"].as_str().unwrap_or(""),
                value["state"].as_str().unwrap_or("unknown"),
                value["open_target"].as_str().unwrap_or("")
            );
        } else {
            println!("{value}");
        }
    });
}
fn snapshot(cfg: &Config, json_mode: bool) -> Result<(Option<ApiClient>, Vec<api::Torrent>)> {
    match connect(cfg, false, true, json_mode) {
        Ok(c) => {
            if c.hello.state == api::DaemonState::Stopping {
                drop(c);
                return offline::list().map(|rows| (None, rows));
            }
            let rows = list(&c)?;
            Ok((Some(c), rows))
        }
        Err(error) => {
            // Permission/protocol errors must never be disguised as an offline read.
            if let Some(io) = error.downcast_ref::<std::io::Error>() {
                if !matches!(
                    io.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) {
                    return Err(error);
                }
            } else {
                return Err(error);
            }
            Ok((None, offline::list()?))
        }
    }
}
pub(crate) fn run(cfg: &Config, cmd: &Cmd, json_mode: bool, no_start: bool) -> Result<Code> {
    match cmd {
        Cmd::Move {selector,save_path}=>{
            let c=connect(cfg,true,no_start,json_mode)?;
            let hashes=selected(&list(&c)?,&[selector.clone()])?;
            for batch in hashes.chunks(256) {
                let pending=c.call("torrents.move",json!({"hashes":batch,"save_path":save_path}))?;
                await_move(&c,pending)?;
            }
            emit(json_mode,json!({"ok":true,"action":"move","count":hashes.len(),"hashes":hashes}));
        }
        Cmd::Add {
            source,
            stopped,
            seed,
            save,
            category,
            quiet,
            notify: explicit_notify,
            wait,
            timeout,
            origin,
            ..
        } => {
            if let Some(path) = save { crate::free_space::require_writable(path)?; }
            if origin=="handler" && cfg.handler=="confirm" {
                if no_start || !cfg.autostart { let _=ApiClient::connect(cfg,json_mode)?; }
                let value=super::handler::confirm(cfg,source,*stopped,save.clone(),category.clone())?;
                crate::emit(json_mode,value,||println!("Review torrent in the dashboard; download has not started."));return Ok(Code::Ok);
            }
            let client = connect(cfg, true, no_start, json_mode)?;
            let added = add(
                &client,
                cfg,
                source,
                *stopped,
                *seed,
                save.clone(),
                category.clone(),
                if origin == "handler" && (*quiet || json_mode) { "agent" } else { origin },
                false,
            )?;
            if !quiet && cfg.notify && !json_mode && origin != "handler" && origin != "agent"
                && (*explicit_notify || crate::agent::stdout_is_tty()) {
                crate::notify(
                    if added["deferred"] == true {
                        "Waiting for VPN"
                    } else {
                        "Torrent added"
                    },
                    added["name"].as_str().unwrap_or(source),
                    false,
                );
            }
            if *wait {
                return wait_for(
                    cfg,
                    client,
                    vec![added["hash"].as_str().context("add omitted hash")?.into()],
                    *timeout,
                    2,
                    Until::Complete,
                    json_mode,
                );
            }
            if let Some(disk) = added.get("disk") {
                let disk:api::DiskSpace = serde_json::from_value(disk.clone())?;
                crate::agent::warning("disk_low",crate::daemon::supervisor::warning_message(&disk),true);
            }
            let mut value = added;
            value["ok"] = json!(true);
            value["action"] = json!("add");
            value["sequential"] = json!(true);
            value["first_last"] = json!(true);
            value["seed"] = json!(*seed);
            value["stopped"] = json!(stopped);
            if value["source"].is_null() {
                value["source"] = json!(if source.starts_with("magnet:") {
                    "magnet"
                } else if source.starts_with("http") {
                    "url"
                } else {
                    "file"
                });
            }
            emit(json_mode, value);
        }
        Cmd::AddClipboard => {
            let out = std::process::Command::new("wl-paste")
                .arg("--no-newline")
                .output()?;
            let source = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let client = connect(cfg, true, no_start, json_mode)?;
            let mut value = add(&client, cfg, &source, false, false, None, None, "manual", false)?;
            value["ok"] = json!(true);
            value["action"] = json!("add");
            emit(json_mode, value);
        }
        Cmd::Status {quiet} => return super::status::run(cfg,json_mode,*quiet),
        Cmd::List { .. } | Cmd::Show { .. } | Cmd::Triage => {
            let (client, rows) = snapshot(cfg, json_mode)?;
            let running = client.is_some();
            match cmd {
                Cmd::List { active } => {
                    let values: Vec<_> = rows
                        .iter()
                        .filter(|t| {
                            !active
                                || matches!(
                                    t.state,
                                    api::TorrentState::Downloading
                                        | api::TorrentState::Metadata
                                        | api::TorrentState::Checking
                                        | api::TorrentState::Stalled
                                )
                        })
                        .map(shapes::torrent)
                        .collect();
                    emit(
                        json_mode,
                        json!({"ok":true,"running":running,"count":values.len(),"torrents":values}),
                    );
                }
                Cmd::Show { selector } => {
                    let hashes = selected(&rows, &[selector.clone()])?;
                    let t = rows.iter().find(|t| t.hash.as_str() == hashes[0]).unwrap();
                    let mut value = shapes::torrent(t);
                    value["ok"] = json!(true);
                    value["running"] = json!(running);
                    value["open_target"] = json!(t.content_path.as_deref().unwrap_or(&t.save_path));
                    value["seed_allowed"] = json!(t.seed);
                    value["complete"] = json!(t.progress.map(|v| v >= 1.0));
                    value["peers"] = if let Some(c)=&client {if c.hello.caps.iter().any(|cap|cap=="peers"){c.call("torrents.peers",json!({"hash":hashes[0]}))?["peers"].clone()}else{json!([])}}else{json!([])};
                    value["files"] = if let Some(c) = &client {
                        c.call("torrents.files", json!({"hash":hashes[0]}))
                            .map(|v| json!(v["files"].as_array().map(|files|files.iter().map(|f|json!({"index":f["index"],"name":f["path"],"size":f["size"],"progress":f["progress"],"progress_known":t.checked && t.state!=api::TorrentState::WaitingVpn,"priority":if f["included"]==true{1}else{0}})).collect::<Vec<_>>()).unwrap_or_default()))
                            .unwrap_or(json!([]))
                    } else {
                        json!([])
                    };
                    if let Some(c)=&client {
                        let detail=c.call("torrents.get",json!({"hash":hashes[0]}))?;
                        if let Some(count)=detail["torrent"].get("tracker_count") {value["tracker_count"]=count.clone();}
                        value["trackers"]=if c.hello.caps.iter().any(|cap|cap=="trackers") {
                            serde_json::to_value(serde_json::from_value::<api::TrackersResult>(c.call("torrents.trackers",json!({"hash":hashes[0]}))?)?.trackers.into_iter().map(api::trackers::public_tracker).collect::<Vec<_>>())?
                        } else {json!([])};
                    } else { value["trackers"]=json!([]); }
                    emit(json_mode, value);
                }
                Cmd::Triage => {
                    let mut value = json!({"ok":true,"running":running,"contract_version":crate::agent::CONTRACT_VERSION,"backend":{"engine":"native","running":running,"connection":if running{"api"}else{"stopped"}},"counts":counts(&rows),"torrents":shapes::rows(&rows),"next_commands":["fbittorrent events --follow","fbittorrent search \"<title>\" --json"],"docs":"fbittorrent robot-docs"});
                    if let Some(c) = &client {
                        let info = c.call("app.info", json!({}))?;
                        value["backend"]["daemon"] = daemon_status(&info);
                        value["transfer"] = c.call("transfer.info", json!({}))?;
                        value["network"] = c.call("app.network", json!({}))?;
                    }
                    emit(true, value);
                }
                _ => unreachable!(),
            }
        }
        Cmd::Stop { selectors }
        | Cmd::Delete {
            selectors,
            files: false,
            ..
        }
        | Cmd::Delete {
            selectors,
            dry_run: true,
            ..
        } => {
            if let Cmd::Delete { dry_run: true, .. } = cmd {
                let (client, rows) = snapshot(cfg, json_mode)?;
                let hashes = selected(&rows, selectors)?;
                emit(
                    json_mode,
                    json!({"ok":true,"action":"delete","dry_run":true,"files":matches!(cmd,Cmd::Delete{files:true,..}),"running":client.is_some(),"torrents":shapes::rows(&rows.iter().filter(|row|hashes.iter().any(|hash|hash==row.hash.as_str())).cloned().collect::<Vec<_>>())}),
                );
                return Ok(Code::Ok);
            }
            let stop = matches!(cmd, Cmd::Stop { .. });
            let (hashes, running) = offline::mutation(cfg, selectors, stop, json_mode)?;
            emit(
                json_mode,
                json!({"ok":true,"action":if stop{"stop"}else{"delete"},"count":hashes.len(),"hashes":hashes,"running":running}),
            );
        }
        Cmd::Rename{selector,name}=>{
            let c=connect(cfg,true,no_start,json_mode)?;
            let hashes=selected(&list(&c)?,&[selector.clone()])?;
            anyhow::ensure!(hashes.len()==1,AppError::new(Code::Usage,"rename requires one torrent"));
            c.call("torrents.rename",json!({"hash":hashes[0],"name":name}))?;
            emit(json_mode,json!({"ok":true,"action":"rename","count":1,"hashes":hashes}));
        },
        Cmd::Limit{selector,down}=>{
            let bps=super::download_limit::parse(down)?;
            let c=connect(cfg,true,no_start,json_mode)?;
            let hashes=selected(&list(&c)?,&[selector.clone()])?;
            bulk(&c,"torrents.set_download_limit",&hashes,json!({"bps":bps}))?;
            emit(json_mode,json!({"ok":true,"action":"limit","hashes":hashes,"download_limit_bps":bps}));
        },
        Cmd::Start { .. }
        | Cmd::Reannounce { .. }
        | Cmd::Seed { .. }
        | Cmd::Noseed { .. }
        | Cmd::Delete { .. }
        | Cmd::Open { .. }
        | Cmd::Wait { .. } => {
            let write = !matches!(cmd, Cmd::Wait { .. });
            let c = connect(cfg, write, no_start, json_mode)?;
            let rows = list(&c)?;
            let selectors = match cmd {
                Cmd::Open { selector } => vec![selector.clone()],
                Cmd::Start { selectors }
                | Cmd::Reannounce { selectors }
                | Cmd::Seed { selectors }
                | Cmd::Noseed { selectors }
                | Cmd::Delete { selectors, .. }
                | Cmd::Wait { selectors, .. } => selectors.clone(),
                _ => unreachable!(),
            };
            let hashes = selected(&rows, &selectors)?;
            match cmd {
                Cmd::Reannounce {..} => {
                    for batch in hashes.chunks(256) { c.call("torrents.reannounce",json!({"hashes":batch}))?; }
                    emit(json_mode,json!({"ok":true,"action":"reannounce","count":hashes.len(),"hashes":hashes}));
                }
                Cmd::Wait {
                    timeout, interval, until, ..
                } => return wait_for(cfg, c, hashes, *timeout, *interval, *until, json_mode),
                Cmd::Open { .. } => {
                    let target = c.call("torrents.open_target", json!({"hash":hashes[0]}))?;
                    cfg.open_target(target["path"].as_str().context("open target omitted path")?)?;
                    emit(
                        json_mode,
                        json!({"ok":true,"action":"open","hash":hashes[0],"target":target["path"]}),
                    );
                }
                Cmd::Delete {
                    files,
                    yes,
                    dry_run,
                    ..
                } => {
                    if *dry_run {
                        emit(
                            json_mode,
                            json!({"ok":true,"action":"delete","dry_run":true,"files":files,"torrents":shapes::rows(&rows)}),
                        );
                        return Ok(Code::Ok);
                    }
                    anyhow::ensure!(
                        !files || *yes,
                        AppError::new(
                            Code::Safety,
                            "refusing to delete torrent files without --yes"
                        )
                        .hint("fbittorrent delete <hash> --files --yes")
                    );
                    bulk(
                        &c,
                        "torrents.remove",
                        &hashes,
                        json!({"delete_files":files}),
                    )?;
                    emit(
                        json_mode,
                        json!({"ok":true,"action":"delete","files":files,"count":hashes.len(),"hashes":hashes}),
                    );
                }
                Cmd::Start { selectors } => {
                    if selectors != &["all"] {
                        if let Some(row) = rows.iter().find(|r| {
                            hashes.iter().any(|h| h == r.hash.as_str())
                                && (r.completed_at.is_some()
                                    || r.state == api::TorrentState::Done
                                    || r.progress.is_some_and(|p| p >= 1.0))
                                && !r.seed
                        }) {
                            return Err(AppError::new(
                                Code::Failure,
                                "already complete; seeding is off for this torrent",
                            )
                            .hint(format!("fbittorrent seed {}", row.hash))
                            .into());
                        }
                    }
                    let mut started = Vec::new();
                    let mut skipped = Vec::new();
                    for chunk in hashes.chunks(256) {
                        if chunk.is_empty() {
                            continue;
                        }
                        let v = c.call("torrents.start", json!({"hashes":chunk}))?;
                        started.extend(
                            v["started"]
                                .as_array()
                                .context("start omitted started")?
                                .clone(),
                        );
                        skipped.extend(
                            v["skipped"]
                                .as_array()
                                .context("start omitted skipped")?
                                .clone(),
                        );
                    }
                    emit(
                        json_mode,
                        json!({"ok":true,"action":"start","count":started.len(),"hashes":started,"skipped":skipped}),
                    );
                }
                _ => {
                    let on = matches!(cmd, Cmd::Seed { .. });
                    bulk(&c, "torrents.set_seed", &hashes, json!({"on":on}))?;
                    emit(
                        json_mode,
                        json!({"ok":true,"action":if on{"seed"}else{"noseed"},"count":hashes.len(),"hashes":hashes}),
                    );
                }
            }
        }
        Cmd::Restart => {
            emit(json_mode, super::restart::daemon(cfg,json_mode)?);
        }
        _ => unreachable!("non-native CLI command"),
    }
    Ok(Code::Ok)
}
fn bulk(c: &ApiClient, op: &str, hashes: &[String], extra: Value) -> Result<()> {
    for chunk in hashes.chunks(256) {
        let mut args = extra.clone();
        if op == "torrents.stop" {
            args = json!({});
        }
        args["hashes"] = json!(chunk);
        let result = c.call(op, args)?;
        if result["state"] == "pending" {
            await_mutation(c, result)?;
        }
    }
    Ok(())
}
fn await_move(c:&ApiClient,result:Value)->Result<Value> {
    let reference=result["ref"].as_str().context("move acknowledgement omitted ref")?;
    loop {
        let unknown=||AppError::new(Code::Timeout,"move outcome is unknown after the owner connection closed").hint("fbittorrent list --json");
        if let Some(event)=c.event(Duration::from_millis(100)).map_err(|_|unknown())? {
            if event["ev"]=="mutation.result" && event["ref"]==reference {
                if event["outcome"]=="done" {return Ok(event.get("result").cloned().unwrap_or(json!({"state":"done"})));}
                return Err(api::errors::app_error(serde_json::from_value(event["error"].clone())?));
            }
            if event["ev"]=="bye" || event["ev"]=="client.disconnected" {return Err(unknown().into());}
        }
    }
}
pub(super) fn await_mutation(c: &ApiClient, result: Value) -> Result<Value> {
    let reference = result["ref"]
        .as_str()
        .context("pending result omitted ref")?;
    let deadline = Instant::now() + Duration::from_secs(95);
    while Instant::now() < deadline {
        if let Some(event) = c.event(Duration::from_millis(100))? {
            if event["ev"] == "mutation.result" && event["ref"] == reference {
                if event["outcome"] == "done" {
                    return Ok(event
                        .get("result")
                        .cloned()
                        .unwrap_or(json!({"state":"done"})));
                }
                return Err(api::errors::app_error(serde_json::from_value(
                    event["error"].clone(),
                )?));
            }
            if event["ev"] == "bye" || event["ev"] == "client.disconnected" {
                anyhow::bail!(AppError::new(
                    Code::Backend,
                    "owner closed while mutation outcome is unknown"
                ));
            }
        }
    }
    Err(AppError::new(
        Code::Timeout,
        "mutation confirmation timed out; outcome is unknown",
    )
    .into())
}
pub(super) fn add(
    c: &ApiClient,
    cfg: &Config,
    source: &str,
    stopped: bool,
    seed: bool,
    save: Option<String>,
    category: Option<String>,
    origin: &str,
    stage: bool,
) -> Result<Value> {
    if let Some(path) = &save { crate::free_space::require_writable(path)?; }
    let input=super::handler::source(source)?;
    let mut args =
        json!({"source":input,"origin":origin,"stopped":stopped,"seed":seed});
    if let Some(save) = save.or(cfg.save_path.clone()) {
        args["save_path"] = json!(save);
    }
    if let Some(category) = category.or(cfg.category.clone()) {
        args["category"] = json!(category);
    }
    if stage {args["stage"]=json!(true);}
    let result = c.call("torrents.add", args)?;
    if result["deferred"] == true || result["state"] != "pending" || result.get("hash").is_some() {
        return Ok(result);
    }
    await_mutation(c, result)
}
pub(super) fn counts(rows: &[api::Torrent]) -> Value {
    let visible:Vec<_>=rows.iter().filter(|r|!r.staged).collect();
    json!({"total":visible.len(),"active":visible.iter().filter(|r|matches!(r.state,api::TorrentState::Downloading | api::TorrentState::Metadata | api::TorrentState::Checking | api::TorrentState::Stalled)).count(),"queued":visible.iter().filter(|r|r.state==api::TorrentState::Queued).count(),"seeding":visible.iter().filter(|r|r.state==api::TorrentState::Seeding).count(),"stopped":visible.iter().filter(|r|r.state==api::TorrentState::Paused).count(),"ready":visible.iter().filter(|r|r.playable.as_ref().is_some_and(|p|p.ready)).count(),"staged":rows.iter().filter(|r|r.staged).count()})
}
fn wait_for(
    cfg: &Config,
    c: ApiClient,
    hashes: Vec<String>,
    timeout: u64,
    interval: u64,
    until: Until,
    json_mode: bool,
) -> Result<Code> {
    let start = Instant::now();
    anyhow::ensure!(until==Until::Complete || c.hello.caps.iter().any(|c|c=="watch"), AppError::new(Code::Usage,"the daemon does not advertise watch readiness"));
    c.limit_reconnects(1);
    // Pushed changes wake the read immediately; a bounded poll remains the
    // compatibility/reconnect fallback. The connection owns this subscription.
    let subscription=c.hello.caps.iter().any(|cap|cap=="subscribe")
        .then(||c.call("subscribe",json!({"topics":["torrents"],"interval_ms":1000})))
        .and_then(Result::ok).and_then(|v|v["sub"].as_u64());
    struct Release<'a>(&'a ApiClient,Option<u64>);
    impl Drop for Release<'_> {fn drop(&mut self){if let Some(id)=self.1 {let _=self.0.unsubscribe(id);}}}
    let _release=Release(&c,subscription);
    let mut frames=api::client::Subscription::new(&c);
    loop {
        if let Some(event) = c.event(Duration::from_millis(1))? {
            if event.get("sub").is_some(){frames.accept(event.clone())?;}
            match event["reason"].as_str() {
                Some("idle") => {
                    emit(json_mode, timeout_value("every waited torrent is stopped"));
                    return Ok(Code::Timeout);
                }
                Some("slow_client" | "hello_required" | "invalid_message") => {
                    return Ok(Code::Failure)
                }
                _ => {}
            }
        }
        let rows = list(&c)?;
        let mine: Vec<_> = rows
            .iter()
            .filter(|r| hashes.iter().any(|h| h == r.hash.as_str()))
            .collect();
        anyhow::ensure!(
            mine.len() == hashes.len(),
            AppError::new(Code::NotFound, "torrent disappeared while waiting")
        );
        if let Some(bad) = mine.iter().find(|r| r.state == api::TorrentState::Error) {
            return Err(AppError::new(
                Code::Upstream,
                bad.error.as_deref().unwrap_or("torrent error"),
            )
            .into());
        }
        let complete=mine.iter().all(|r|r.progress.is_some_and(|v|v>=1.0));
        let playable=mine.iter().all(|r|r.playable.as_ref().is_some_and(|p|p.ready));
        let stall_free=mine.iter().all(|r|r.playable.as_ref().is_some_and(|p|p.ready && p.stall_free==Some(true)));
        if match until {Until::Complete=>complete,Until::Playable=>playable,Until::StallFree=>stall_free} {
            let elapsed = if std::env::var_os("SOURCE_DATE_EPOCH").is_some() {
                0
            } else {
                start.elapsed().as_secs()
            };
            let mut torrents=Vec::new();
            for row in &mine {
                let mut value=shapes::torrent(row);
                if let Some(play)=row.playable.as_ref().filter(|p|p.ready) {
                    let files=c.call("torrents.files",json!({"hash":row.hash}))?;
                    if let Some(file)=files["files"].as_array().and_then(|fs|fs.iter().find(|f|f["index"]==play.file)) {
                        if let Some(path)=file["path"].as_str() {value["open_target"]=json!(std::path::Path::new(&row.save_path).join(path).to_string_lossy());}
                    }
                }
                torrents.push(value);
            }
            emit(json_mode,json!({"ok":true,"action":"wait","complete":complete,"playable":playable,"stall_free":stall_free,"elapsed_secs":elapsed,"torrents":torrents}));
            return Ok(Code::Ok);
        }
        if (timeout > 0 && start.elapsed() >= Duration::from_secs(timeout))
            || mine.iter().all(|r| r.intent == api::Intent::Pause)
        {
            emit(
                json_mode,
                timeout_value(if mine.iter().all(|r| r.intent == api::Intent::Pause) {
                    "every waited torrent is stopped"
                } else {
                    "timeout"
                }),
            );
            return Ok(Code::Timeout);
        }
        let deadline=Instant::now()+Duration::from_secs(interval.max(1)).min(if timeout==0 {Duration::MAX}else{Duration::from_secs(timeout).saturating_sub(start.elapsed())});
        while Instant::now()<deadline {
            let remaining=deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100));
            if let Some(frame)=c.event(remaining)? {
                if frame["ev"]=="bye" || frame["ev"]=="client.disconnected" {break;}
                let frame=frames.accept(frame)?;
                if subscription.is_some_and(|sub|frame["sub"]==sub) && matches!(frame["ev"].as_str(),Some("delta"|"snapshot.end")) {break;}
            }
        }
    }
}

fn timeout_value(reason: &str) -> Value {
    json!({"ok":false,"action":"wait","complete":false,"reason":reason,"code":8,"code_name":"timeout","error":reason,"hint":"fbittorrent wait <hash> --timeout N"})
}

pub(super) fn daemon_status(info: &Value) -> Value {
    let mut value = json!({"state":info["state"],"upgrade_pending":info["upgrade_pending"]});
    for key in ["busy_reasons", "idle_exit_in_s"] {
        if let Some(field) = info.get(key) {
            value[key] = field.clone();
        }
    }
    value
}

fn summary(c: &ApiClient, cfg: &Config) -> Result<Option<Value>> {
    summary_with(c,cfg,|op,args|c.call(op,args))
}
pub(super) fn summary_with(c:&ApiClient,cfg:&Config,read:impl Fn(&str,Value)->Result<Value>)->Result<Option<Value>> {
    if !c.hello.caps.iter().any(|cap| cap == "subscribe") {
        return Ok(None);
    }
    let reply = read(
        "subscribe",
        json!({"topics":["summary"],"interval_ms":1000}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs.max(1));
    while Instant::now() < deadline {
        if let Some(event) = c.event(Duration::from_millis(100))? {
            if event["ev"] == "state" && event["topic"] == "summary" && event["sub"] == reply["sub"]
            {
                let _: api::Summary = serde_json::from_value(event["data"].clone())?;
                read("unsubscribe",json!({"sub":reply["sub"].as_u64().context("subscribe omitted sub")?}))?;
                return Ok(Some(event["data"].clone()));
            }
        }
    }
    Err(AppError::new(Code::Timeout, "summary snapshot timed out").into())
}
