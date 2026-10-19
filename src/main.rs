mod agent;
mod cli;
#[path = "cli/local.rs"]
mod cli_local;
#[path = "cli/sources.rs"]
mod cli_sources;
use cli::{Cli, Cmd, RssCmd, ShowsCmd};
use cli_local::*;
use cli_sources::{resolve, resolve_in, tui_free_dedupe, short, local_torrent_source};
pub use cli_sources::{magnet_hash, magnet_name};
mod api;
mod backend;
mod browse;
mod config;
mod free_space;
mod daemon;
mod engine;
mod native_runtime;
mod network_app;
mod ipc;
mod imdb;
mod qbt;
#[cfg(test)]
mod qbt_tests;
mod release;
mod rss;
mod search;
mod shows;
mod setup;
mod tui;

use agent::{AppError, Code};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
#[cfg(test)]
use engine::LegacyMigration;
use qbt::{human_bytes, human_eta, human_rate, AddOptions, Client, Torrent};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    // Behave like a Unix tool when stdout is a closed pipe (`fbittorrent list | head`):
    // exit quietly instead of panicking on EPIPE.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }
    let cli = match Cli::try_parse() {
        Ok(cli)=>cli,
        Err(error)=>{
            if std::env::args().any(|arg|arg=="--json") && !matches!(error.kind(),clap::error::ErrorKind::DisplayHelp|clap::error::ErrorKind::DisplayVersion) {
                std::process::exit(agent::report(&AppError::new(Code::Usage,error.to_string()).into(),true));
            }
            error.exit();
        }
    };
    let json = cli.json;
    if let Some(Cmd::Serve { idle_exit_secs, no_notify }) = &cli.cmd {
        if no_start_requested(cli.no_start, std::env::var_os("FBITTORRENT_NO_START").as_deref()) {
            let error = AppError::new(Code::Backend, "fbittorrent is closed (daemon startup is disabled)").hint("fbittorrent up");
            std::process::exit(agent::report(&error.into(), json));
        }
        // No search session, TUI signal owner or background thread may start
        // before serve has blocked its shutdown signals.
        let result = std::panic::catch_unwind(|| Config::load()
            .and_then(|cfg| daemon::run_serve(cfg, *idle_exit_secs, *no_notify)));
        std::process::exit(match result {
            Ok(result) => result.unwrap_or_else(|error| {
                let (_, message, hint) = agent::classify(&error);
                eprintln!("ERROR daemon startup failed message={message:?}");
                if json {
                    let mut error = AppError::new(Code::Failure, message);
                    error.hint = hint;
                    agent::report(&error.into(), true)
                } else {
                    if let Some(hint) = hint {
                        eprintln!("ERROR daemon startup hint={hint:?}");
                    }
                    1
                }
            }),
            Err(_) => 70,
        });
    }
    match run(cli) {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => std::process::exit(if let Some(signal) = e.downcast_ref::<tui::SignalTermination>() {
            128 + signal.0
        } else { agent::report(&e, json) }),
    }
}

/// Print a JSON value or, when not in JSON mode, run the human printer.
fn emit(json: bool, value: serde_json::Value, human: impl FnOnce()) {
    if json { println!("{}",agent::with_warnings(value)); } else { human(); }
}

fn run(cli: Cli) -> Result<Code> {
    // Static key documentation must not read or create app state.
    if let Some(Cmd::RobotDocs { section, surface }) = &cli.cmd {
        return cli::robot_docs(section, surface, cli.json);
    }
    if matches!(&cli.cmd,Some(Cmd::Api { list:true,.. })) { return cli::raw::catalog(cli.json); }
    let _search_cleanup = SearchCleanup;
    search::begin_session()?;
    let cfg = Config::load()?;
    let no_start = no_start_requested(cli.no_start, std::env::var_os("FBITTORRENT_NO_START").as_deref());
    if no_start { backend::prevent_start(); }
    let json = cli.json;
    let cmd = match cli.cmd {
        Some(c) => c,
        None if cli.robot_triage || !agent::stdout_is_tty() => Cmd::Triage,
        None => Cmd::Tui { search: false, rss: false, shows: false },
    };
    // Read/offline-intent commands never cause an owner to be launched. Offline
    // ledger mutations themselves are supplied by P1-11.
    if cfg.native() && !native_may_start(&cmd) && !matches!(cmd, Cmd::Down) { backend::prevent_start(); }
    // `triage` and the TUI are the two bare-invocation faces; `triage` is always JSON.
    let json = json || matches!(cmd, Cmd::Triage | Cmd::Capabilities);
    if !cfg.native() { agent::warning("deprecated_engine", "engine = \"qbittorrent\" is deprecated and is removed in v0.4.0; see the README".into(), true); }
    if cfg.native() && cli::native_handles(&cmd) && !(matches!(&cmd,Cmd::Status{..}|Cmd::List{..}|Cmd::Show{..}|Cmd::Triage|Cmd::Wait{..}) && !ipc::socket_path(&cfg).with_file_name("api.sock").exists() && ipc::is_running(&cfg)) { return cli::run_native(&cfg, &cmd, json, no_start); }
    if cfg.native() && matches!(&cmd, Cmd::Seq { .. } | Cmd::FirstLast { .. }) {
        return Err(AppError::new(Code::Usage, "native sequential and first/last scheduling are intrinsic; toggle commands require legacy mode")
            .hint("explicitly configure engine = \"qbittorrent\" to use legacy scheduling controls").into());
    }

    // Commands that never touch the backend.
    match cmd {
        Cmd::Capabilities => { println!("{}", agent::with_warnings(agent::capabilities_json())); return Ok(Code::Ok); }
        Cmd::Setup { port, save, magnet, plugin, dry_run, uninstall, purge, yes } => {
            if purge && !yes { return Err(AppError::new(Code::Safety,"setup --purge requires --yes; saved data was kept").into()); }
            if cfg.native() {
                let actions=setup::desktop::run(&cfg,setup::desktop::Options {save,magnet,plugin,dry_run,uninstall,purge})?;
                emit(json,serde_json::json!({"ok":true,"action":if uninstall || purge {"setup-uninstall"}else{"setup"},"engine":cfg.engine,"port":cfg.peer_port,"magnet_handler":magnet,"actions":actions}),||{});
                return Ok(Code::Ok);
            }
            if dry_run || plugin.is_some() || purge { return Err(AppError::new(Code::Usage,"extended desktop setup requires the native engine").into()); }
            if uninstall {
                setup::uninstall()?;
                emit(json, serde_json::json!({"ok":true,"action":"setup-uninstall"}), || {});
                return Ok(Code::Ok);
            }
            if no_start && !cfg.native() {
                return Err(AppError::new(Code::Backend, "legacy setup cannot start qBittorrent while --no-start or FBITTORRENT_NO_START=1 is active")
                    .hint("fbittorrent up").into());
            }
            setup::run(port, save)?;
            if magnet { setup::register_magnet_handler()?; }
            emit(json, serde_json::json!({"ok": true, "action": "setup", "engine": cfg.engine, "port": if cfg.native() { cfg.peer_port } else { port }, "magnet_handler": magnet}), || {});
            return Ok(Code::Ok);
        }
        Cmd::RegisterMagnet => {
            setup::register_magnet_handler()?;
            emit(json, serde_json::json!({"ok": true, "action": "register-magnet"}), || {});
            return Ok(Code::Ok);
        }
        Cmd::Imdb { name, refresh } => {
            return cli::catalog_imdb::run(&cfg,name.join(" "),refresh,json);
        }
        Cmd::Config => {
            emit(json, serde_json::json!({"ok": true, "path": config::config_path().display().to_string(), "config": serde_json::to_value(&cfg)?}), || {
                println!("# {}", config::config_path().display());
                print!("{}", toml::to_string_pretty(&cfg).unwrap_or_default());
            });
            return Ok(Code::Ok);
        }
        Cmd::Events { follow, since_seq, kinds } => return cli::stream::events(&cfg, follow, since_seq, kinds),
        Cmd::Api { op, list, args, json_args, reference } => return cli::raw::run(&cfg, json, no_start, op, list, args, json_args, reference),
        Cmd::Limits { down, up, alt, alt_down, alt_up } => return cli::limits::run(&cfg, json, down, up, alt, alt_down, alt_up),
        Cmd::Log { follow, level, source } => return cli::logs::run(&cfg, json, follow, level, source),
        Cmd::Settings { max_downloads, encryption, vpn, interfaces, idle_secs, handler, clipboard_watch, port } => {
            return cli::settings::run(&cfg, json, max_downloads, encryption, vpn, interfaces, idle_secs, handler, clipboard_watch, port);
        }
        _ => {}
    }

    let c = Client::from_config(&cfg);

    match cmd {
        Cmd::Triage => { println!("{}", agent::with_warnings(agent::triage(&cfg, &c))); Ok(Code::Ok) }
        Cmd::Status { quiet: _ } => {
            if c.version().is_err() {
                let backend = if !cfg.native() && backend::is_active(&cfg.backend_unit) { "starting" } else { "stopped" };
                emit(json, serde_json::json!({"ok": false, "backend": backend, "engine": cfg.engine, "app_open": false}), || {
                    if cfg.native() { println!("fbittorrent is closed; the native engine is stopped"); }
                    else { println!("qBittorrent is not running ({})", cfg.backend_unit); }
                });
                return Ok(Code::Ok);
            }
            c.login()?;
            let ti = c.transfer_info()?;
            let ts = c.torrents()?;
            let active = ts.iter().filter(|t| t.is_active()).count();
            let queued = ts.iter().filter(|t| t.is_queued()).count();
            let seeding = ts.iter().filter(|t| t.is_seeding()).count();
            let stopped = ts.iter().filter(|t| t.is_stopped()).count();
            let mut top: Vec<&Torrent> = ts.iter().filter(|t| t.is_active()).collect();
            top.extend(ts.iter().filter(|t| !t.is_active()).take(12));
            emit(json, serde_json::json!({
                "ok": true, "engine": cfg.engine, "app_open": cfg.native(), "dl": ti.dl_info_speed, "up": ti.up_info_speed,
                "active": active, "queued": queued, "seeding": seeding, "stopped": stopped, "total": ts.len(),
                "connection": ti.connection_status, "torrents": top,
            }), || println!("↓ {}  ↑ {}  | {} active, {} seeding, {} stopped, {} total",
                human_rate(ti.dl_info_speed), human_rate(ti.up_info_speed), active, seeding, stopped, ts.len()));
            Ok(Code::Ok)
        }
        Cmd::Up { ui } => {
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            let started = backend::ensure_running(&explicit, &c)?;
            if ui { backend::dashboard(None, None)?; }
            emit(json, serde_json::json!({"ok": true, "action": "up", "started": started, "engine": cfg.engine,
                "unit": if cfg.native() { api::client::unit_name()? } else { cfg.backend_unit.clone() }}),
                || println!("fbittorrent {}", if started { "started" } else { "already running" }));
            Ok(Code::Ok)
        }
        Cmd::Dashboard { view, query, hash } => {
            if let Some(hash)=&hash {api::Hash::new(hash.clone()).map_err(|message|AppError::new(Code::Usage,message))?;}
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            backend::ensure_running(&explicit, &c)?;
            backend::dashboard_hash(view.as_deref(), query.as_deref(), hash.as_deref())?;
            emit(json, serde_json::json!({"ok":true,"action":"dashboard","count":0,"hashes":[]}), || {});
            Ok(Code::Ok)
        }
        Cmd::Restart => {
            if cfg.native() {
                let result = cli::restart::daemon(&cfg,json)?;
                emit(json,result,|| println!("daemon restarted"));
                return Ok(Code::Ok);
            } else {
                backend::stop(&cfg)?;
                let mut explicit = cfg.clone(); explicit.autostart = !no_start;
                backend::ensure_running(&explicit, &c)?;
            }
            emit(json, serde_json::json!({"ok":true,"action":"restart","count":0,"hashes":[],"scope":"engine"}), || {});
            Ok(Code::Ok)
        }
        Cmd::Down => {
            backend::stop(&cfg)?;
            emit(json, serde_json::json!({"ok": true, "action": "down", "engine": cfg.engine, "unit": if cfg.native() { None } else { Some(&cfg.backend_unit) }}), || {
                if cfg.native() { println!("fbittorrent closed; its engine is stopped"); }
                else { println!("stopped {}", cfg.backend_unit); }
            });
            Ok(Code::Ok)
        }
        Cmd::Tui { search, rss, shows } => {
            if !agent::stdout_is_tty() {
                return Err(AppError::new(Code::Usage, "the TUI needs a terminal; stdout is not a TTY").hint("fbittorrent triage --json   # the non-interactive overview").into());
            }
            let screen = if search { tui::Screen::Search } else if rss { tui::Screen::Rss } else if shows { tui::Screen::Shows } else { tui::Screen::Torrents };
            if !cfg.native() { connect(&cfg, &c)?; }
            tui::run(cfg, screen)?;
            Ok(Code::Ok)
        }
        Cmd::List { active } => {
            connect(&cfg, &c)?;
            let ts: Vec<_> = c.torrents()?.into_iter().filter(|t| !active || t.is_active()).collect();
            emit(json, serde_json::json!({"ok": true, "count": ts.len(), "torrents": ts}), || {
                for t in &ts {
                    let flags = format!("{}{}{}", if t.seq_dl { "S" } else { "-" }, if t.f_l_piece_prio { "F" } else { "-" }, if t.seed_allowed() { "U" } else { "-" });
                    let progress = if t.progress_known { format!("{:.1}%", t.progress * 100.0) } else { "—".into() };
                    println!("{}  {:>6}  {:>11}  {:>6}  {}  {:<12} {}",
                        &t.hash[..8], progress, human_rate(t.dlspeed), human_eta(t.eta), flags, t.state, t.name);
                }
            });
            Ok(Code::Ok)
        }
        Cmd::Show { selector } => {
            connect(&cfg, &c)?;
            let ts = c.torrents()?;
            let h = resolve_in(&ts, &[selector])?;
            let t = ts.iter().find(|t| t.hash == h[0]).cloned().unwrap();
            let files = c.files(&t.hash).unwrap_or_default();
            let mut v = serde_json::to_value(&t)?;
            v["ok"] = serde_json::json!(true);
            v["files"] = serde_json::to_value(&files)?;
            v["open_target"] = serde_json::json!(qbt::open_target(&t));
            v["seed_allowed"] = serde_json::json!(t.seed_allowed());
            v["complete"] = if t.progress_known { serde_json::json!(t.progress >= 1.0) } else { serde_json::Value::Null };
            emit(json, v, || {
                let progress = if t.progress_known { format!("{:.1}%  {} of {}", t.progress * 100.0, human_bytes(t.completed), human_bytes(t.size)) } else if t.state == "waitingVPN" { "checked when the VPN connects".into() } else { "waiting for metadata or verification".into() };
                println!("{}\n  hash      {}\n  state     {}\n  progress  {}\n  speed     ↓ {}  ↑ {}\n  eta       {}\n  peers     {} seeds, {} leechers\n  flags     seq={} first/last={} upload={}\n  path      {}\n  open      {}",
                    t.name, t.hash, t.state, progress,
                    human_rate(t.dlspeed), human_rate(t.upspeed), human_eta(t.eta), t.num_seeds, t.num_leechs,
                    t.seq_dl, t.f_l_piece_prio, if t.seed_allowed() { "allowed" } else { "off" }, t.save_path, qbt::open_target(&t));
                if let Some(message) = &t.error_message { println!("  error     {message}"); }
                if !files.is_empty() {
                    println!("  files");
                    for f in &files {
                        let progress = if f.progress_known { format!("{:>5.0}%", f.progress * 100.0) } else { "     —".into() };
                        println!("    [{}] {:>9} {}  {}", if f.priority == 0 { " " } else { "x" }, human_bytes(f.size), progress, f.name);
                    }
                }
            });
            Ok(Code::Ok)
        }
        Cmd::Add { source, no_seq, no_first_last, stopped, seed, save, category, quiet, notify: explicit_notify, origin, wait, timeout } => {
            if cfg.native() && (no_seq || no_first_last) {
                return Err(AppError::new(Code::Usage, "the native engine downloads sequentially with first/last pieces prioritised; those settings cannot be disabled")
                    .hint("drop --no-seq and --no-first-last, or explicitly configure engine = \"qbittorrent\" for legacy scheduling").into());
            }
            connect(&cfg, &c)?;
            let plain_notify = cfg.notify && !quiet && !json && origin != "handler" && origin != "agent"
                && (explicit_notify || agent::stdout_is_tty());
            let opts = AddOptions {
                origin: Some(if origin == "handler" && (quiet || json) { "agent".into() } else { origin.clone() }),
                sequential: cfg.native() || (cfg.sequential && !no_seq),
                first_last: cfg.native() || (cfg.first_last && !no_first_last),
                seed: cfg.seed_by_default || seed,
                stopped,
                save_path: save.or(cfg.save_path.clone()),
                category: category.or(cfg.category.clone()),
            };
            // Local .torrent file: the desktop handler passes file:// URLs, users pass paths.
            let local = local_torrent_source(&source)?;
            if std::path::Path::new(&local).is_file() {
                let bytes = std::fs::read(&local)?;
                if !bytes.starts_with(b"d") {
                    return Err(AppError::new(Code::Usage, format!("'{}' is not a .torrent file (bencoded data expected)", short(&local, 60)))
                        .hint("fbittorrent add \"<magnet or URL>\"").into());
                }
                let filename = std::path::Path::new(&local).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "file.torrent".into());
                let before: std::collections::HashSet<String> = c.torrents()?.into_iter().map(|t| t.hash).collect();
                if let Err(e) = c.add_file(&filename, &bytes, &opts) {
                    if plain_notify { notify("Torrent add failed", "Could not add this torrent. Inspect the log for details.", true); }
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(400));
                let added = c.torrents()?.into_iter().find(|t| !before.contains(&t.hash));
                let (hash, name) = match &added { Some(t) => (Some(t.hash.clone()), t.name.clone()), None => (None, filename.clone()) };
                if plain_notify { notify("Torrent added", &name, false); }
                if wait {
                    let Some(h) = hash.clone() else {
                        return Err(AppError::new(Code::Failure, "added, but could not determine the torrent hash to wait for")
                            .hint("fbittorrent list --json   # find it by name, then `fbittorrent wait <hash>`").into());
                    };
                    return wait_for(&c, vec![h], timeout, 2, json);
                }
                emit(json, serde_json::json!({"ok": true, "action": "add", "hash": hash, "name": name, "source": "file", "sequential": opts.sequential, "first_last": opts.first_last, "seed": opts.seed, "stopped": stopped}),
                    || println!("added: {name}{}", hash.as_deref().map(|h| format!("  ({})", &h[..8.min(h.len())])).unwrap_or_default()));
                return Ok(Code::Ok);
            }
            if !(source.starts_with("magnet:") || source.starts_with("http://") || source.starts_with("https://")) {
                return Err(AppError::new(Code::Usage, format!("'{}' is not a magnet link or URL", short(&source, 60)))
                    .hint("fbittorrent search \"<title>\" --json | jq -r '.results[0].fileUrl'").into());
            }
            let label = magnet_name(&source).unwrap_or_else(|| source.chars().take(60).collect());
            let before: std::collections::HashSet<String> = c.torrents()?.into_iter().map(|t| t.hash).collect();
            if let Err(e) = c.add_url(&source, &opts) {
                if plain_notify { notify("Torrent add failed", "Could not add this torrent. Inspect the log for details.", true); }
                return Err(e);
            }
            if plain_notify { notify("Torrent added", &label, false); }
            // The hash is in the magnet; for URLs we look for the torrent that appeared.
            let hash = magnet_hash(&source).or_else(|| {
                std::thread::sleep(Duration::from_millis(400));
                c.torrents().ok()?.into_iter().find(|t| !before.contains(&t.hash)).map(|t| t.hash)
            });
            if wait {
                let Some(h) = hash.clone() else {
                    return Err(AppError::new(Code::Failure, "added, but could not determine the torrent hash to wait for")
                        .hint("fbittorrent list --json   # find it by name, then `fbittorrent wait <hash>`").into());
                };
                return wait_for(&c, vec![h], timeout, 2, json);
            }
            emit(json, serde_json::json!({"ok": true, "action": "add", "hash": hash, "name": label, "sequential": opts.sequential, "first_last": opts.first_last, "seed": opts.seed, "stopped": stopped}),
                || println!("added: {label}{}", hash.as_deref().map(|h| format!("  ({})", &h[..8.min(h.len())])).unwrap_or_default()));
            Ok(Code::Ok)
        }
        Cmd::Wait { selectors, timeout, interval, until } => {
            if until!=cli::Until::Complete {return Err(AppError::new(Code::Usage,"watch readiness requires the native API").into());}
            connect(&cfg, &c)?;
            let hs = resolve(&c, &selectors)?;
            wait_for(&c, hs, timeout, interval.max(1), json)
        }
        Cmd::Start { selectors } => {
            connect(&cfg, &c)?;
            let ts = c.torrents()?;
            let hs = resolve_in(&ts, &selectors)?;
            let result = c.start_report(&hs, selectors == ["all"])?;
            emit(json, serde_json::json!({"ok": true, "action": "start", "count": result.started.len(),
                "hashes": result.started, "skipped": result.skipped}), || {
                println!("start: {}", result.started.join(", "));
                for skip in &result.skipped { println!("skipped {}: complete; seeding is off", skip.hash); }
            });
            Ok(Code::Ok)
        },
        Cmd::Stop { selectors } => act(&cfg, &c, json, "stop", &selectors, |c, hs| c.stop(hs)),
        Cmd::Seq { selectors } => act(&cfg, &c, json, "toggle-sequential", &selectors, |c, hs| c.toggle_sequential(hs)),
        Cmd::FirstLast { selectors } => act(&cfg, &c, json, "toggle-first-last", &selectors, |c, hs| c.toggle_first_last(hs)),
        Cmd::Seed { selectors } => act(&cfg, &c, json, "seed", &selectors, |c, hs| c.allow_seeding(hs)),
        Cmd::Noseed { selectors } => act(&cfg, &c, json, "noseed", &selectors, |c, hs| c.deny_seeding(hs)),
        Cmd::Delete { selectors, files, yes, dry_run } => {
            connect(&cfg, &c)?;
            let ts = c.torrents()?;
            let hs = resolve_in(&ts, &selectors)?;
            let picked: Vec<&Torrent> = ts.iter().filter(|t| hs.contains(&t.hash)).collect();
            let summary: Vec<serde_json::Value> = picked.iter().map(|t| serde_json::json!({"hash": t.hash, "name": t.name, "size": t.size, "path": qbt::open_target(t)})).collect();
            if dry_run {
                emit(json, serde_json::json!({"ok": true, "action": "delete", "dry_run": true, "files": files, "torrents": summary}), || {
                    println!("would remove {} torrent(s){}:", picked.len(), if files { " AND their files" } else { " (files kept)" });
                    for t in &picked { println!("  {}  {}  {}", &t.hash[..8], human_bytes(t.size), t.name); }
                });
                return Ok(Code::Ok);
            }
            if files && !yes {
                let total: i64 = picked.iter().map(|t| t.size).sum();
                return Err(AppError::new(Code::Safety, format!("refusing to delete the files of {} torrent(s) ({}) without --yes", picked.len(), human_bytes(total)))
                    .hint(format!("fbittorrent delete {} --files --yes   # or drop --files to keep the data, or add --dry-run to preview", selectors.join(" "))).into());
            }
            c.delete(&hs, files)?;
            emit(json, serde_json::json!({"ok": true, "action": "delete", "files": files, "count": hs.len(), "hashes": hs, "torrents": summary}),
                || println!("removed {} torrent(s){}", picked.len(), if files { " with files" } else { ", files kept" }));
            Ok(Code::Ok)
        }
        Cmd::AddClipboard => {
            connect(&cfg, &c)?;
            let out = Command::new("wl-paste").arg("--no-newline").output()?;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !(text.starts_with("magnet:") || text.starts_with("http://") || text.starts_with("https://")) {
                if cfg.notify && !json && agent::stdout_is_tty() { notify("Nothing to add", "Clipboard holds no magnet link or URL", true); }
                return Err(AppError::new(Code::Usage, "clipboard holds no magnet link or URL").hint("fbittorrent add \"<magnet>\"").into());
            }
            let opts = AddOptions {
                origin: None,
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            };
            let label = magnet_name(&text).unwrap_or_else(|| text.chars().take(60).collect());
            if let Err(e) = c.add_url(&text, &opts) {
                if cfg.notify && !json && agent::stdout_is_tty() { notify("Torrent add failed", "Could not add this torrent. Inspect the log for details.", true); }
                return Err(e);
            }
            if cfg.notify && !json && agent::stdout_is_tty() { notify("Torrent added", &label, false); }
            emit(json, serde_json::json!({"ok": true, "action": "add", "hash": magnet_hash(&text), "name": label}), || println!("added: {label}"));
            Ok(Code::Ok)
        }
        Cmd::Move {..} => Err(AppError::new(Code::Usage,"move requires the native API daemon").into()),
        Cmd::Reannounce {..} => Err(AppError::new(Code::Usage,"reannounce requires the native API daemon").into()),
        Cmd::Reconcile { recent } => {
            let result = reconcile_result(&cfg, || {
                connect(&cfg, &c)?;
                c.reconcile(cfg.sequential, cfg.first_last, cfg.seed_by_default, recent)
            })?;
            emit(json, result.clone(), || {
                if cfg.native() { println!("native scheduling is intrinsic"); }
                else { println!("reconciled {} torrent(s)", result["touched"]); }
            });
            Ok(Code::Ok)
        }
        Cmd::Open { selector } => {
            connect(&cfg, &c)?;
            let ts = c.torrents()?;
            let h = resolve_in(&ts, &[selector])?;
            let t = ts.iter().find(|t| t.hash == h[0]).unwrap();
            let target = qbt::open_target(t);
            cfg.open_target(&target)?;
            emit(json, serde_json::json!({"ok": true, "action": "open", "hash": t.hash, "target": target}), || println!("opened {target}"));
            Ok(Code::Ok)
        }
        Cmd::Rss(sub) => {
            if !cfg.native() || matches!(&sub, RssCmd::Grab { .. }) { connect(&cfg, &c)?; }
            rss(&c, sub, json, &cfg)
        }
        Cmd::Shows(sub) => shows_cmd(&cfg, &c, sub, json),
        Cmd::Search { query, wait, limit, install, install_url, plugins } => {
            if cfg.native() {return cli::catalog_search::run(&cfg,query,wait,limit,install,install_url,plugins,json);}
            if !cfg.native() { connect(&cfg, &c)?; }
            if plugins {
                let ps = c.search_plugins()?;
                emit(json, serde_json::json!({"ok": true, "count": ps.len(), "enabled": ps.iter().filter(|p| p.enabled).count(), "plugins": ps}), || {
                    for p in &ps { println!("{:<18} {:<3} {:<8} {}", p.name, if p.enabled { "on" } else { "off" }, p.version, p.url); }
                });
                return Ok(Code::Ok);
            }
            if install || !install_url.is_empty() {
                let mut srcs: Vec<String> = install_url.clone();
                if install {
                    let base = "https://raw.githubusercontent.com/qbittorrent/search-plugins/master/nova3/engines/";
                    for n in ["eztv", "limetorrents", "piratebay", "solidtorrents", "torlock", "torrentproject", "torrentscsv"] {
                        srcs.push(format!("{base}{n}.py"));
                    }
                    srcs.extend(qbt::COMMUNITY_SEARCH_PLUGINS.iter().map(|s| s.to_string()));
                }
                let before = c.search_plugins()?.len();
                c.search_install_plugins(&srcs)?;
                for _ in 0..30 {
                    std::thread::sleep(Duration::from_millis(500));
                    if c.search_plugins()?.len() >= before.max(srcs.len()) { break; }
                }
                c.search_enable_all()?;
                let ps = c.search_plugins()?;
                emit(json, serde_json::json!({"ok": true, "action": "install-plugins", "count": ps.len(), "plugins": ps.iter().map(|p| p.name.clone()).collect::<Vec<_>>()}),
                    || println!("{} search plugins installed: {}", ps.len(), ps.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")));
                return Ok(Code::Ok);
            }
            let q = query.join(" ");
            if q.trim().is_empty() {
                return Err(AppError::new(Code::Usage, "no search query given").hint("fbittorrent search \"<title>\" --json").into());
            }
            let plugins_now = c.search_plugins()?;
            if plugins_now.is_empty() {
                return Err(AppError::new(Code::Backend, "no search providers installed").hint("fbittorrent search --install").into());
            }
            if !plugins_now.iter().any(|p| p.enabled) {
                return Err(AppError::new(Code::Usage, "every search provider is disabled").hint("fbittorrent search --plugins   # then enable some in the TUI with `s`, or run `search --install`").into());
            }
            let id = c.search_start(&q)?;
            let deadline = Instant::now() + Duration::from_secs(wait);
            let mut last;
            loop {
                last = match c.search_results(id, 1000, 0) {
                    Ok(r) => r,
                    Err(e) if e.to_string().contains("404") && Instant::now() < deadline => { std::thread::sleep(Duration::from_millis(500)); continue; }
                    Err(e) => return Err(e),
                };
                if last.status != "Running" || Instant::now() >= deadline { break; }
                std::thread::sleep(Duration::from_millis(700));
            }
            let _ = c.search_stop(id);
            let _ = c.search_delete(id);
            let mut rs = tui_free_dedupe(last.results);
            rs.sort_by(|a, b| b.seeders.cmp(&a.seeders));
            rs.truncate(limit);
            emit(json, serde_json::json!({"ok": true, "query": q, "count": rs.len(), "complete": last.status != "Running", "results": rs}), || {
                for r in &rs {
                    println!("{:>5}S {:>5}L  {:>10}  {:<12}  {}", r.seeders, r.leechers, human_bytes(r.file_size), r.engine, r.file_name);
                }
            });
            Ok(Code::Ok)
        }
        Cmd::Events { .. } | Cmd::Api { .. } | Cmd::Log { .. } | Cmd::Serve { .. } | Cmd::Capabilities | Cmd::RobotDocs { .. } | Cmd::Setup { .. } | Cmd::RegisterMagnet | Cmd::Config | Cmd::Settings { .. } | Cmd::Limits { .. } | Cmd::Imdb { .. } => unreachable!(),
    }
}

/// Resolve selectors, run a bulk action, report the affected hashes.
fn act<F>(cfg: &Config, c: &Client, json: bool, action: &str, selectors: &[String], f: F) -> Result<Code>
where F: FnOnce(&Client, &[String]) -> Result<()> {
    connect(cfg, c)?;
    let ts = c.torrents()?;
    let hs = resolve_in(&ts, selectors)?;
    f(c, &hs)?;
    let names: Vec<&str> = ts.iter().filter(|t| hs.contains(&t.hash)).map(|t| t.name.as_str()).collect();
    emit(json, serde_json::json!({"ok": true, "action": action, "count": hs.len(), "hashes": hs}), || {
        println!("{action}: {}", names.join(", "));
    });
    Ok(Code::Ok)
}

/// Block until every hash is complete. Exit 0 when done, 8 on timeout, 5 when a torrent errors.
fn wait_for(c: &Client, hashes: Vec<String>, timeout: u64, interval: u64, json: bool) -> Result<Code> {
    let start = Instant::now();
    let mut last_line = String::new();
    loop {
        let ts = c.torrents()?;
        let mine: Vec<&Torrent> = ts.iter().filter(|t| hashes.contains(&t.hash)).collect();
        if mine.len() < hashes.len() {
            let missing: Vec<&String> = hashes.iter().filter(|h| !mine.iter().any(|t| &t.hash == *h)).collect();
            return Err(AppError::new(Code::NotFound, format!("torrent {} disappeared while waiting", missing[0])).into());
        }
        if let Some(bad) = mine.iter().find(|t| t.state == "error" || t.state == "missingFiles") {
            return Err(AppError::new(Code::Upstream, format!("torrent '{}' is in state {}", bad.name, bad.state))
                .hint(format!("fbittorrent show {} --json", &bad.hash[..8])).into());
        }
        let done = mine.iter().all(|t| t.progress_known && t.progress >= 1.0);
        let stopped = mine.iter().filter(|t| t.is_stopped() && t.progress < 1.0).count();
        if !json || !done {
            let line = mine.iter().map(|t| {
                let progress = if t.progress_known { format!("{:.1}%", t.progress * 100.0) } else { "—".into() };
                format!("{} {} {}", &t.hash[..8], progress, if t.is_active() { human_rate(t.dlspeed) } else { t.state.clone() })
            }).collect::<Vec<_>>().join(" | ");
            if line != last_line { eprintln!("{line}"); last_line = line; }
        }
        if done {
            let result: Vec<serde_json::Value> = mine.iter().map(|t| serde_json::json!({"hash": t.hash, "name": t.name, "state": t.state, "open_target": qbt::open_target(t)})).collect();
            emit(json, serde_json::json!({"ok": true, "action": "wait", "complete": true, "elapsed_secs": start.elapsed().as_secs(), "torrents": result}),
                || for t in &mine { println!("complete: {}  {}", t.name, qbt::open_target(t)); });
            return Ok(Code::Ok);
        }
        if stopped == mine.len() {
            return Err(AppError::new(Code::Timeout, "every waited torrent is stopped; it will never complete like this")
                .hint(format!("fbittorrent start {} --json", hashes.iter().map(|h| &h[..8]).collect::<Vec<_>>().join(" "))).into());
        }
        if timeout > 0 && start.elapsed() >= Duration::from_secs(timeout) {
            let progress: Vec<serde_json::Value> = mine.iter().map(|t| serde_json::json!({"hash": t.hash, "name": t.name, "state": t.state, "progress": if t.progress_known { Some(t.progress) } else { None }, "progress_known": t.progress_known, "eta": t.eta})).collect();
            if json { println!("{}", serde_json::json!({"ok": false, "code": Code::Timeout as i32, "code_name": "timeout", "error": "timed out before completion", "torrents": progress, "hint": "fbittorrent wait <hash> --timeout N   # keep waiting"})); }
            else { eprintln!("fbittorrent: error[timeout]: timed out before completion\n  try: fbittorrent wait {} --timeout {}", hashes.iter().map(|h| &h[..8]).collect::<Vec<_>>().join(" "), timeout); }
            return Ok(Code::Timeout);
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn connect(cfg: &Config, c: &Client) -> Result<()> {
    backend::ensure_running(cfg, c)?;
    c.login()
}

fn native_may_start(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::Move {..} | Cmd::Reannounce { .. } | Cmd::Add { .. } | Cmd::AddClipboard | Cmd::Start { .. } | Cmd::Seed { .. }
        | Cmd::Noseed { .. } | Cmd::Delete { files: true, .. }
        | Cmd::Up { .. } | Cmd::Tui { .. } | Cmd::Dashboard { .. }
        | Cmd::Rss(RssCmd::Grab { .. }) | Cmd::Shows(ShowsCmd::Grab { .. }))
}

fn no_start_requested(flag: bool, environment: Option<&std::ffi::OsStr>) -> bool {
    flag || environment == Some(std::ffi::OsStr::new("1"))
}

fn reconcile_result(cfg: &Config, legacy: impl FnOnce() -> Result<usize>) -> Result<serde_json::Value> {
    if cfg.native() {
        return Ok(serde_json::json!({"ok": true, "touched": 0, "note": "native scheduling is intrinsic"}));
    }
    Ok(serde_json::json!({"ok": true, "action": "reconcile", "touched": legacy()?}))
}

/// Search providers belong to the invoking CLI or app process, including errors.
struct SearchCleanup;
impl Drop for SearchCleanup {
    fn drop(&mut self) { search::shutdown_all(); }
}

#[cfg(test)]
mod migration_store_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn migration_save_is_private_for_new_and_reused_temporaries() {
        let directory = std::env::temp_dir().join(format!("fbittorrent-migration-private-{}", std::process::id()));
        let path = directory.join("legacy-migration.json");
        let migration = LegacyMigration {
            phase: "captured".into(), source_url: "http://127.0.0.1:9".into(),
            source_unit: "private-fixture.service".into(), backup: directory.join("backup"),
            torrents: Vec::new(), files: Default::default(), feeds: Vec::new(), plugins: Vec::new(),
        };
        for replacement in [false, true] {
            if replacement {
                for file in [&path, &path.with_extension("json.tmp")] {
                    std::fs::write(file, b"old file").unwrap();
                    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            migration.save(&path).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let saved: LegacyMigration = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.phase, "captured");
            assert!(!path.with_extension("json.tmp").exists());
            println!("{}: {mode:o} after migration save", path.display());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use cli_sources::percent_decode;

    #[test]
    fn native_start_policy_matches_command_roles() {
        for args in [vec!["list"], vec!["show", "abcd"], vec!["stop", "all"], vec!["delete", "all"], vec!["status"], vec!["config"]] {
            let cli = Cli::try_parse_from(std::iter::once("fbittorrent").chain(args)).unwrap();
            assert!(!native_may_start(&cli.cmd.unwrap()));
        }
        for args in [vec!["up"], vec!["up", "--ui"], vec!["dashboard", "--view", "rss"], vec!["start", "all"], vec!["delete", "--files", "all"]] {
            let cli = Cli::try_parse_from(std::iter::once("fbittorrent").chain(args)).unwrap();
            assert!(native_may_start(&cli.cmd.unwrap()));
        }
        println!("read/offline roles never start; writes and explicit UI/up roles may start");
    }

    #[test]
    fn no_start_is_global_and_environment_requires_one() {
        for args in [["fbittorrent", "--no-start", "list"], ["fbittorrent", "list", "--no-start"]] {
            assert!(Cli::try_parse_from(args).unwrap().no_start);
        }
        for value in [None, Some(""), Some("0"), Some("true")] {
            assert!(!no_start_requested(false, value.map(std::ffi::OsStr::new)));
            assert!(no_start_requested(true, value.map(std::ffi::OsStr::new)));
        }
        assert!(no_start_requested(false, Some(std::ffi::OsStr::new("1"))));
    }

    #[test]
    fn native_reconcile_returns_stub_without_connecting() {
        let result = reconcile_result(&Config::default(), || panic!("native reconcile must not connect")).unwrap();
        println!("native Cmd::Reconcile: {result}");
        assert_eq!(result, serde_json::json!({"ok": true, "touched": 0, "note": "native scheduling is intrinsic"}));
        let legacy = Config { engine: "qbittorrent".into(), ..Config::default() };
        assert_eq!(reconcile_result(&legacy, || Ok(3)).unwrap(),
                   serde_json::json!({"ok": true, "action": "reconcile", "touched": 3}));
    }

    #[test]
    fn browser_file_urls_preserve_plus_spaces_and_unicode() {
        assert_eq!(local_torrent_source("file:///tmp/A+B%20%C3%BC.torrent").unwrap(), "/tmp/A+B ü.torrent");
        assert_eq!(local_torrent_source("file://localhost/tmp/a.torrent").unwrap(), "/tmp/a.torrent");
        assert!(local_torrent_source("file://remote/tmp/a.torrent").is_err());
    }

    #[test]
    fn malformed_unicode_query_escapes_do_not_panic() {
        assert_eq!(percent_decode("%中+%C3%BC%"), "%中 ü%");
        assert_eq!(magnet_name("magnet:?dn=A+B%2BC"), Some("A B+C".into()));
    }
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    Some(rest.split('/').next()?.trim_start_matches("www.").to_string())
}

fn notify(title: &str, body: &str, error: bool) {
    let _ = Command::new("notify-send")
        .args(["-a", "fbittorrent", "-i", "fbittorrent", "-u", if error { "critical" } else { "normal" }, &daemon::notify::escape_markup(title), &daemon::notify::escape_markup(body)])
        .spawn();
}
