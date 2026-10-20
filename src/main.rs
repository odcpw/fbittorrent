#[macro_use]
mod terminal_macros;
mod agent;
mod cli;
#[path = "cli/sources.rs"]
mod cli_sources;
use cli::{Cli, Cmd, RssCmd, ShowsCmd};
use cli_sources::short;
pub use cli_sources::{magnet_hash, magnet_name};
mod api;
mod backend;
mod browse;
mod config;
mod free_space;
mod daemon;
mod engine;
mod feed_date;
mod native_runtime;
mod durable_sync;
mod network_app;
mod outbound;
mod public_text;
mod ipc;
mod imdb;
mod qbt;
mod release;
mod rss;
mod search;
mod shows;
mod setup;
mod tui;
mod torrent_format;

use agent::{AppError, Code};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use qbt::human_bytes;
use std::process::Command;

fn main() {
    if let Some(code)=search::torznab::run_worker(){std::process::exit(code);}

    // Behave like a Unix tool when stdout is a closed pipe (`fbittorrent list | head`):
    // exit quietly instead of panicking on EPIPE.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }
    let cli = match cli::parse_entry(std::env::args_os()) {
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

#[cold]
#[inline(never)]
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
    if !native_may_start(&cmd) && !matches!(cmd, Cmd::Down) { backend::prevent_start(); }
    // `triage` and the TUI are the two bare-invocation faces; `triage` is always JSON.
    let json = json || matches!(cmd, Cmd::Triage | Cmd::Capabilities);
    if cli::native_handles(&cmd) { return cli::run_native(&cfg, &cmd, json, no_start); }
    if let Cmd::Rss(sub)=&cmd {return cli::catalog_rss::run(&cfg,sub,json,no_start);}
    if let Cmd::Shows(sub)=&cmd {return cli::catalog_shows::run(&cfg,sub,json,no_start);}

    // Commands that never touch the backend.
    match cmd {
        Cmd::Capabilities => { println!("{}", agent::with_warnings(agent::capabilities_json())); return Ok(Code::Ok); }
        Cmd::Setup { port, save, magnet, plugin, quick, dry_run, uninstall, purge, yes } => {
            if purge && !yes { return Err(AppError::new(Code::Safety,"setup --purge requires --yes; saved data was kept").into()); }
            let _ = port;
            let actions=setup::desktop::run(&cfg,setup::desktop::Options {save,magnet,plugin,quick,dry_run,uninstall,purge})?;
            emit(json,serde_json::json!({"ok":true,"action":if uninstall || purge {"setup-uninstall"}else{"setup"},"engine":"native","port":cfg.peer_port,"magnet_handler":magnet,"actions":actions}),||{});
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
        Cmd::Limit{..}=>return Err(AppError::new(Code::Usage,"per-torrent download limits require the daemon API").into()),
        Cmd::Limits { down, up, alt, alt_down, alt_up } => return cli::limits::run(&cfg, json, down, up, alt, alt_down, alt_up),
        Cmd::Log { follow, level, source } => return cli::logs::run(&cfg, json, follow, level, source),
        Cmd::Settings { max_downloads, encryption, vpn, interfaces, idle_secs, handler, clipboard_watch, port, upload } => {
            return cli::settings::run(&cfg, json, max_downloads, encryption, vpn, interfaces, idle_secs, handler, clipboard_watch, port, upload);
        }
        _ => {}
    }

    match cmd {
        Cmd::Up { ui } => {
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            let started = api::client::ensure_daemon(&explicit, no_start)?;
            if ui { backend::dashboard(None, None)?; }
            emit(json, serde_json::json!({"ok": true, "action": "up", "started": started, "engine": "native",
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
            emit(json,serde_json::json!({"ok":true,"action":"down","engine":"native","unit":null}), || println!("fbittorrent closed; its engine is stopped"));
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
        Cmd::Search { query, wait, limit, install, install_url, plugins, install_torznab, key_file } => {
            if let Some(url)=install_torznab {
                let key_file=key_file.map(|p|if p.is_absolute(){Ok(p)}else{std::env::current_dir().map(|dir|dir.join(p))}).transpose()?;
                let name=search::torznab::install(&url,key_file)?;
                emit(json,serde_json::json!({"ok":true,"action":"install-torznab","name":name,"url":url,"kind":"torznab"}),||println!("Torznab indexer installed: {name}"));
                return Ok(Code::Ok);
            }
            cli::catalog_search::run(&cfg,query,wait,limit,install,install_url,plugins,json)
        }
        Cmd::Triage | Cmd::Status {..} | Cmd::List {..} | Cmd::Show {..} | Cmd::Add {..} | Cmd::Wait {..} | Cmd::Start {..} | Cmd::Stop {..} | Cmd::Upload {..} | Cmd::Noupload {..} | Cmd::Delete {..} | Cmd::AddClipboard | Cmd::Move {..} | Cmd::Rename {..} | Cmd::Reannounce {..} | Cmd::Open {..} | Cmd::Restart | Cmd::Events { .. } | Cmd::Api { .. } | Cmd::Log { .. } | Cmd::Serve { .. } | Cmd::Capabilities | Cmd::RobotDocs { .. } | Cmd::Setup { .. } | Cmd::Config | Cmd::Settings { .. } | Cmd::Limits { .. } | Cmd::Limit { .. } | Cmd::Imdb { .. } | Cmd::Rss(..) | Cmd::Shows(..) => unreachable!(),
    }
}

fn native_may_start(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::Limit { .. } | Cmd::Move {..} | Cmd::Reannounce { .. } | Cmd::Add { .. } | Cmd::AddClipboard | Cmd::Start { .. } | Cmd::Upload { .. }
        | Cmd::Noupload { .. } | Cmd::Delete { files: true, .. }
        | Cmd::Up { .. } | Cmd::Tui { .. } | Cmd::Dashboard { .. }
        | Cmd::Rss(RssCmd::Grab { .. }) | Cmd::Shows(ShowsCmd::Grab { .. }))
}

fn no_start_requested(flag: bool, environment: Option<&std::ffi::OsStr>) -> bool {
    flag || environment == Some(std::ffi::OsStr::new("1"))
}

/// Search providers belong to the invoking CLI or app process, including errors.
struct SearchCleanup;
impl Drop for SearchCleanup {
    fn drop(&mut self) { search::shutdown_all(); }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use cli_sources::{local_torrent_source, percent_decode};

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

/// A display/default name: the host only, never userinfo (credentials), port or query.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    Some(parsed.host_str().filter(|host| !host.is_empty())?.trim_start_matches("www.").to_string())
}

fn notify(title: &str, body: &str, error: bool) {
    let _ = Command::new("notify-send")
        .args(["-a", "fbittorrent", "-i", "fbittorrent", "-u", if error { "critical" } else { "normal" }, &daemon::notify::escape_markup(title), &daemon::notify::escape_markup(body)])
        .spawn();
}
