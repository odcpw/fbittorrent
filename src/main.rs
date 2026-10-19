mod agent;
mod cli;
#[path = "cli/local.rs"]
mod cli_local;
#[path = "cli/sources.rs"]
mod cli_sources;
use cli::{Cli, Cmd, RssCmd, ShowsCmd};
use cli_local::*;
use cli_sources::{resolve_in, tui_free_dedupe, short, local_torrent_source};
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
use qbt::{human_bytes, AddOptions, Torrent};
use cli::local_client::Client;
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
    if cli::native_handles(&cmd) { return cli::run_native(&cfg, &cmd, json, no_start); }
    if matches!(&cmd, Cmd::Seq { .. } | Cmd::FirstLast { .. }) {
        return Err(AppError::new(Code::Usage, "unsupported: sequential and first/last scheduling are intrinsic")
            .hint("fbittorrent show <hash> --json   # inspect streaming scheduling").into());
    }

    // Commands that never touch the backend.
    match cmd {
        Cmd::Capabilities => { println!("{}", agent::with_warnings(agent::capabilities_json())); return Ok(Code::Ok); }
        Cmd::Setup { port, save, magnet, plugin, dry_run, uninstall, purge, yes } => {
            if purge && !yes { return Err(AppError::new(Code::Safety,"setup --purge requires --yes; saved data was kept").into()); }
            let _ = port;
            let actions=setup::desktop::run(&cfg,setup::desktop::Options {save,magnet,plugin,dry_run,uninstall,purge})?;
            emit(json,serde_json::json!({"ok":true,"action":if uninstall || purge {"setup-uninstall"}else{"setup"},"engine":cfg.engine,"port":cfg.peer_port,"magnet_handler":magnet,"actions":actions}),||{});
            return Ok(Code::Ok);
        }
        Cmd::RegisterMagnet => {
            setup::register_magnet_handler()?;
            emit(json, serde_json::json!({"ok": true, "action": "register-magnet"}), || {});
            return Ok(Code::Ok);
        }
        Cmd::Imdb { name, refresh } => {
            if refresh || imdb::ratings_stale() {
                eprintln!("fetching IMDb ratings dataset…");
                imdb::refresh_ratings()?;
            }
            let n = name.join(" ");
            let Some(t) = imdb::lookup(&n)? else {
                emit(json, serde_json::json!({"ok": true, "found": false, "query": n}), || println!("no match"));
                return Ok(Code::Ok);
            };
            let poster = if t.poster_url.is_empty() { None } else { imdb::fetch_poster(&t.poster_url, 300).ok().map(|p| p.display().to_string()) };
            if json {
                let mut v = serde_json::to_value(&t)?;
                v["ok"] = serde_json::json!(true);
                v["found"] = serde_json::json!(true);
                v["url"] = serde_json::json!(t.url());
                v["poster_path"] = serde_json::json!(poster);
                println!("{}", agent::with_warnings(v));
                return Ok(Code::Ok);
            }
            println!("{} ({}) [{}]  {}", t.name, t.year.map(|y| y.to_string()).unwrap_or_default(), t.kind, t.url());
            match (t.rating, t.votes) { (Some(r), Some(v)) => println!("★ {r:.1}  ({v} votes)"), _ => println!("★ n/a") }
            if !t.stars.is_empty() { println!("{}", t.stars); }
            if !t.blurb.is_empty() { println!("\n{}\n{}", t.blurb, t.wiki_url); }
            if let Some(p) = poster { println!("poster: {p}"); }
            return Ok(Code::Ok);
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
        Cmd::Up { ui } => {
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            let started = api::client::ensure_daemon(&explicit, no_start)?;
            if ui { backend::dashboard(None, None)?; }
            emit(json, serde_json::json!({"ok": true, "action": "up", "started": started, "engine": cfg.engine,
                "unit": api::client::unit_name()?}),
                || println!("fbittorrent {}", if started { "started" } else { "already running" }));
            Ok(Code::Ok)
        }
        Cmd::Dashboard { view, query, hash } => {
            if let Some(hash)=&hash {api::Hash::new(hash.clone()).map_err(|message|AppError::new(Code::Usage,message))?;}
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            api::client::ensure_daemon(&explicit, no_start)?;
            backend::dashboard_hash(view.as_deref(), query.as_deref(), hash.as_deref())?;
            emit(json, serde_json::json!({"ok":true,"action":"dashboard","count":0,"hashes":[]}), || {});
            Ok(Code::Ok)
        }
        Cmd::Down => {
            backend::stop(&cfg)?;
            emit(json,serde_json::json!({"ok":true,"action":"down","engine":cfg.engine,"unit":null}), || println!("fbittorrent closed; its engine is stopped"));
            Ok(Code::Ok)
        }
        Cmd::Tui { search, rss, shows } => {
            if !agent::stdout_is_tty() {
                return Err(AppError::new(Code::Usage, "the TUI needs a terminal; stdout is not a TTY").hint("fbittorrent triage --json   # the non-interactive overview").into());
            }
            let screen = if search { tui::Screen::Search } else if rss { tui::Screen::Rss } else if shows { tui::Screen::Shows } else { tui::Screen::Torrents };
            tui::run(cfg, screen)?;
            Ok(Code::Ok)
        }
        Cmd::Reconcile { recent: _ } => {
            emit(json,reconcile_result(),|| println!("native scheduling is intrinsic"));
            Ok(Code::Ok)
        }
        Cmd::Rss(sub) => {
            rss(&c, sub, json, &cfg)
        }
        Cmd::Shows(sub) => shows_cmd(&cfg, &c, sub, json),
        Cmd::Search { query, wait, limit, install, install_url, plugins } => {
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
        Cmd::Triage | Cmd::Status {..} | Cmd::List {..} | Cmd::Show {..} | Cmd::Add {..} | Cmd::Wait {..} | Cmd::Start {..} | Cmd::Stop {..} | Cmd::Seq {..} | Cmd::FirstLast {..} | Cmd::Seed {..} | Cmd::Noseed {..} | Cmd::Delete {..} | Cmd::AddClipboard | Cmd::Move {..} | Cmd::Reannounce {..} | Cmd::Open {..} | Cmd::Restart | Cmd::Events { .. } | Cmd::Api { .. } | Cmd::Log { .. } | Cmd::Serve { .. } | Cmd::Capabilities | Cmd::RobotDocs { .. } | Cmd::Setup { .. } | Cmd::RegisterMagnet | Cmd::Config | Cmd::Settings { .. } | Cmd::Limits { .. } | Cmd::Imdb { .. } => unreachable!(),
    }
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

fn reconcile_result() -> serde_json::Value {
    serde_json::json!({"ok":true,"touched":0,"note":"native scheduling is intrinsic"})
}

/// Search providers belong to the invoking CLI or app process, including errors.
struct SearchCleanup;
impl Drop for SearchCleanup {
    fn drop(&mut self) { search::shutdown_all(); }
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
        let result = reconcile_result();
        println!("native Cmd::Reconcile: {result}");
        assert_eq!(result, serde_json::json!({"ok": true, "touched": 0, "note": "native scheduling is intrinsic"}));
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
