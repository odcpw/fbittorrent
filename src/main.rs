mod agent;
mod backend;
mod browse;
mod config;
mod imdb;
mod qbt;
#[cfg(test)]
mod qbt_tests;
mod release;
mod setup;
mod tui;

use agent::{AppError, Code};
use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use qbt::{human_bytes, human_eta, human_rate, AddOptions, Client, Torrent};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "fbittorrent",
    version,
    about = "qBittorrent front-end for Omarchy: CLI, TUI, bar widget.",
    long_about = "qBittorrent front-end for Omarchy.\n\nBare `fbittorrent` opens the TUI on a terminal; without a TTY it prints `triage --json`.\nEvery command accepts --json. Agents: start with `fbittorrent robot-docs`.",
    after_help = "Exit codes: 0 ok, 2 usage, 3 safety (needs --yes), 4 backend down, 5 upstream, 6 not found, 7 ambiguous, 8 timeout.\nSelect torrents by hash prefix or a unique part of the name; `all` selects every torrent."
)]
struct Cli {
    /// Machine-readable JSON on stdout (errors too, with the same exit code)
    #[arg(long, global = true)]
    json: bool,
    /// Shortcut for `triage --json`
    #[arg(long = "robot-triage", hide = true)]
    robot_triage: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// One-call overview for agents: backend, rates, torrents, warnings, next commands
    #[command(alias = "robot-triage")]
    Triage,
    /// Machine-readable contract: commands, exit codes, conventions
    Capabilities,
    /// Agent handbook (plain text)
    #[command(alias = "docs")]
    RobotDocs {
        /// Section name; only `guide` exists today
        #[arg(default_value = "guide")]
        section: String,
    },
    /// Transfer rates and torrent counts (used by the bar widget). Never starts the backend.
    Status,
    /// Start the qbittorrent-nox user service
    Up,
    /// Stop the qbittorrent-nox user service
    Down,
    /// List torrents
    #[command(alias = "ls")]
    List {
        /// Only torrents still downloading
        #[arg(long)]
        active: bool,
    },
    /// Show one torrent in full: fields, files, and what `open` would launch
    #[command(alias = "info")]
    Show { selector: String },
    /// Add a magnet link or URL. Sequential + first/last piece on, upload off, by default.
    #[command(alias = "download", alias = "get")]
    Add {
        /// magnet:?…, http(s) URL, or a local .torrent file (file:// accepted)
        source: String,
        /// Do not enable sequential download
        #[arg(long)]
        no_seq: bool,
        /// Do not prioritise first and last pieces
        #[arg(long)]
        no_first_last: bool,
        /// Add in stopped state
        #[arg(long)]
        stopped: bool,
        /// Allow uploading for this torrent (default: no upload, stop when complete)
        #[arg(long)]
        seed: bool,
        /// Save path override
        #[arg(long)]
        save: Option<String>,
        /// Category
        #[arg(long)]
        category: Option<String>,
        /// Suppress the desktop notification
        #[arg(long)]
        quiet: bool,
        /// Block until the download completes (see `wait`)
        #[arg(long)]
        wait: bool,
        /// Give up waiting after N seconds (0 = forever)
        #[arg(long, default_value_t = 0)]
        timeout: u64,
    },
    /// Block until torrents complete. Progress on stderr, result on stdout. Exit 8 on timeout.
    Wait {
        selectors: Vec<String>,
        /// Give up after N seconds (0 = forever)
        #[arg(long, default_value_t = 0)]
        timeout: u64,
        /// Poll interval in seconds
        #[arg(long, default_value_t = 2)]
        interval: u64,
    },
    /// Start (resume) torrents
    #[command(alias = "resume")]
    Start { selectors: Vec<String> },
    /// Stop (pause) torrents
    #[command(alias = "pause")]
    Stop { selectors: Vec<String> },
    /// Remove torrents. Data is kept unless --files (which needs --yes).
    #[command(alias = "rm", alias = "remove")]
    Delete {
        selectors: Vec<String>,
        /// Also delete the downloaded files (irreversible; requires --yes)
        #[arg(long)]
        files: bool,
        /// Confirm an irreversible deletion
        #[arg(long)]
        yes: bool,
        /// Show what would be removed and exit
        #[arg(long)]
        dry_run: bool,
    },
    /// Toggle sequential download
    Seq { selectors: Vec<String> },
    /// Toggle first/last piece priority
    FirstLast { selectors: Vec<String> },
    /// Allow uploading/seeding
    Seed { selectors: Vec<String> },
    /// Forbid uploading: 32 KiB/s cap, stop when complete (default for new torrents)
    Noseed { selectors: Vec<String> },
    /// Enforce default flags on torrents added outside fbittorrent
    Reconcile {
        /// Only touch torrents added within the last N seconds (0 = all)
        #[arg(long, default_value_t = 0)]
        recent: i64,
    },
    /// Add whatever magnet/URL is on the Wayland clipboard
    AddClipboard,
    /// Open the file (single-file torrent) or folder in the desktop
    Open { selector: String },
    /// RSS feeds
    #[command(subcommand)]
    Rss(RssCmd),
    /// Search torrents through the enabled providers
    #[command(alias = "find")]
    Search {
        /// Query words
        query: Vec<String>,
        /// Install/update the official plugins plus a curated set of community providers, then exit
        #[arg(long)]
        install: bool,
        /// Install a search plugin from a URL (repeatable), then exit
        #[arg(long = "install-url")]
        install_url: Vec<String>,
        /// List installed providers, then exit
        #[arg(long)]
        plugins: bool,
        /// Seconds to wait for results
        #[arg(long, default_value_t = 15)]
        wait: u64,
        /// Maximum results, best-seeded first
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Configure local qbittorrent-nox as a systemd user service
    Setup {
        #[arg(long, default_value_t = 8080)]
        port: u16,
        #[arg(long)]
        save: Option<String>,
        /// Also register fbittorrent as the magnet: handler
        #[arg(long)]
        magnet: bool,
    },
    /// Register fbittorrent as the magnet: / .torrent handler
    RegisterMagnet,
    /// Open the terminal UI
    Tui {
        /// Start on the search screen
        #[arg(long)]
        search: bool,
        /// Start on the RSS screen
        #[arg(long)]
        rss: bool,
    },
    /// Show config path and effective values
    Config,
    /// Look a release name up on IMDb (title, rating, blurb, poster)
    Imdb {
        name: Vec<String>,
        /// Refresh the ratings dataset first
        #[arg(long)]
        refresh: bool,
    },
}

#[derive(Subcommand)]
enum RssCmd {
    /// List feeds and unread counts
    #[command(alias = "ls")]
    List,
    /// Add a feed
    Add { url: String, #[arg(long)] name: Option<String> },
    /// Remove a feed by name
    #[command(alias = "rm")]
    Remove { name: String },
    /// Refresh one feed, or all
    Refresh { name: Option<String> },
    /// Show items of a feed
    Items { name: String, #[arg(long)] unread: bool },
    /// Add the torrent behind an RSS item (by article id)
    Grab { name: String, article_id: String },
}

fn main() {
    // Behave like a Unix tool when stdout is a closed pipe (`fbittorrent list | head`):
    // exit quietly instead of panicking on EPIPE.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => std::process::exit(agent::report(&e, json)),
    }
}

/// Print a JSON value or, when not in JSON mode, run the human printer.
fn emit(json: bool, value: serde_json::Value, human: impl FnOnce()) {
    if json { println!("{value}"); } else { human(); }
}

fn run(cli: Cli) -> Result<Code> {
    let cfg = Config::load()?;
    let json = cli.json;
    let cmd = match cli.cmd {
        Some(c) => c,
        None if cli.robot_triage || !agent::stdout_is_tty() => Cmd::Triage,
        None => Cmd::Tui { search: false, rss: false },
    };
    // `triage` and the TUI are the two bare-invocation faces; `triage` is always JSON.
    let json = json || matches!(cmd, Cmd::Triage | Cmd::Capabilities);

    // Commands that never touch the backend.
    match cmd {
        Cmd::Capabilities => { println!("{}", agent::capabilities_json()); return Ok(Code::Ok); }
        Cmd::RobotDocs { section } => {
            if section != "guide" {
                return Err(AppError::new(Code::Usage, format!("unknown docs section '{section}'")).hint("fbittorrent robot-docs guide").into());
            }
            print!("{}", agent::ROBOT_GUIDE);
            return Ok(Code::Ok);
        }
        Cmd::Setup { port, save, magnet } => {
            setup::run(port, save)?;
            if magnet { setup::register_magnet_handler()?; }
            emit(json, serde_json::json!({"ok": true, "action": "setup", "port": port, "magnet_handler": magnet}), || {});
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
                println!("{v}");
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
        _ => {}
    }

    let c = Client::from_config(&cfg);

    match cmd {
        Cmd::Triage => { println!("{}", agent::triage(&cfg, &c)); Ok(Code::Ok) }
        Cmd::Status => {
            if c.version().is_err() {
                let backend = if backend::is_active(&cfg.backend_unit) { "starting" } else { "stopped" };
                emit(json, serde_json::json!({"ok": false, "backend": backend}), || println!("qBittorrent is not running ({})", cfg.backend_unit));
                return Ok(Code::Ok);
            }
            c.login()?;
            let ti = c.transfer_info()?;
            let ts = c.torrents()?;
            let active = ts.iter().filter(|t| t.is_active()).count();
            let seeding = ts.iter().filter(|t| t.is_seeding()).count();
            let stopped = ts.iter().filter(|t| t.is_stopped()).count();
            let mut top: Vec<&Torrent> = ts.iter().filter(|t| t.is_active()).collect();
            top.extend(ts.iter().filter(|t| !t.is_active()).take(12));
            emit(json, serde_json::json!({
                "ok": true, "dl": ti.dl_info_speed, "up": ti.up_info_speed,
                "active": active, "seeding": seeding, "stopped": stopped, "total": ts.len(),
                "connection": ti.connection_status, "torrents": top,
            }), || println!("↓ {}  ↑ {}  | {} active, {} seeding, {} stopped, {} total",
                human_rate(ti.dl_info_speed), human_rate(ti.up_info_speed), active, seeding, stopped, ts.len()));
            Ok(Code::Ok)
        }
        Cmd::Up => {
            let started = backend::ensure_running(&cfg, &c)?;
            emit(json, serde_json::json!({"ok": true, "action": "up", "started": started, "unit": cfg.backend_unit}),
                || println!("{} {}", cfg.backend_unit, if started { "started" } else { "already running" }));
            Ok(Code::Ok)
        }
        Cmd::Down => {
            backend::stop(&cfg)?;
            emit(json, serde_json::json!({"ok": true, "action": "down", "unit": cfg.backend_unit}), || println!("stopped {}", cfg.backend_unit));
            Ok(Code::Ok)
        }
        Cmd::Tui { search, rss } => {
            if !agent::stdout_is_tty() {
                return Err(AppError::new(Code::Usage, "the TUI needs a terminal; stdout is not a TTY").hint("fbittorrent triage --json   # the non-interactive overview").into());
            }
            connect(&cfg, &c)?;
            let screen = if search { tui::Screen::Search } else if rss { tui::Screen::Rss } else { tui::Screen::Torrents };
            tui::run(cfg, screen)?;
            Ok(Code::Ok)
        }
        Cmd::List { active } => {
            connect(&cfg, &c)?;
            let ts: Vec<_> = c.torrents()?.into_iter().filter(|t| !active || t.is_active()).collect();
            emit(json, serde_json::json!({"ok": true, "count": ts.len(), "torrents": ts}), || {
                for t in &ts {
                    let flags = format!("{}{}{}", if t.seq_dl { "S" } else { "-" }, if t.f_l_piece_prio { "F" } else { "-" }, if t.seed_allowed() { "U" } else { "-" });
                    println!("{}  {:>5.1}%  {:>11}  {:>6}  {}  {:<12} {}",
                        &t.hash[..8], t.progress * 100.0, human_rate(t.dlspeed), human_eta(t.eta), flags, t.state, t.name);
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
            v["complete"] = serde_json::json!(t.progress >= 1.0);
            emit(json, v, || {
                println!("{}\n  hash      {}\n  state     {}\n  progress  {:.1}%  {} of {}\n  speed     ↓ {}  ↑ {}\n  eta       {}\n  peers     {} seeds, {} leechers\n  flags     seq={} first/last={} upload={}\n  path      {}\n  open      {}",
                    t.name, t.hash, t.state, t.progress * 100.0, human_bytes(t.completed), human_bytes(t.size),
                    human_rate(t.dlspeed), human_rate(t.upspeed), human_eta(t.eta), t.num_seeds, t.num_leechs,
                    t.seq_dl, t.f_l_piece_prio, if t.seed_allowed() { "allowed" } else { "off" }, t.save_path, qbt::open_target(&t));
                if !files.is_empty() {
                    println!("  files");
                    for f in &files { println!("    [{}] {:>9} {:>5.0}%  {}", if f.priority == 0 { " " } else { "x" }, human_bytes(f.size), f.progress * 100.0, f.name); }
                }
            });
            Ok(Code::Ok)
        }
        Cmd::Add { source, no_seq, no_first_last, stopped, seed, save, category, quiet, wait, timeout } => {
            connect(&cfg, &c)?;
            let opts = AddOptions {
                sequential: cfg.sequential && !no_seq,
                first_last: cfg.first_last && !no_first_last,
                seed: cfg.seed_by_default || seed,
                stopped,
                save_path: save.or(cfg.save_path.clone()),
                category: category.or(cfg.category.clone()),
            };
            // Local .torrent file: the desktop handler passes file:// URLs, users pass paths.
            let local = source.strip_prefix("file://").map(|p| percent_decode(p)).unwrap_or_else(|| source.clone());
            if std::path::Path::new(&local).is_file() {
                let bytes = std::fs::read(&local)?;
                if !bytes.starts_with(b"d") {
                    return Err(AppError::new(Code::Usage, format!("'{}' is not a .torrent file (bencoded data expected)", short(&local, 60)))
                        .hint("fbittorrent add \"<magnet or URL>\"").into());
                }
                let filename = std::path::Path::new(&local).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "file.torrent".into());
                let before: std::collections::HashSet<String> = c.torrents()?.into_iter().map(|t| t.hash).collect();
                if let Err(e) = c.add_file(&filename, &bytes, &opts) {
                    if cfg.notify && !quiet { notify("Torrent add failed", &e.to_string(), true); }
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(400));
                let added = c.torrents()?.into_iter().find(|t| !before.contains(&t.hash));
                let (hash, name) = match &added { Some(t) => (Some(t.hash.clone()), t.name.clone()), None => (None, filename.clone()) };
                if cfg.notify && !quiet { notify("Torrent added", &name, false); }
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
                if cfg.notify && !quiet { notify("Torrent add failed", &e.to_string(), true); }
                return Err(e);
            }
            if cfg.notify && !quiet { notify("Torrent added", &label, false); }
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
        Cmd::Wait { selectors, timeout, interval } => {
            connect(&cfg, &c)?;
            let hs = resolve(&c, &selectors)?;
            wait_for(&c, hs, timeout, interval.max(1), json)
        }
        Cmd::Start { selectors } => act(&cfg, &c, json, "start", &selectors, |c, hs| c.start(hs)),
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
                if cfg.notify { notify("Nothing to add", "Clipboard holds no magnet link or URL", true); }
                return Err(AppError::new(Code::Usage, "clipboard holds no magnet link or URL").hint("fbittorrent add \"<magnet>\"").into());
            }
            let opts = AddOptions {
                sequential: cfg.sequential, first_last: cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            };
            let label = magnet_name(&text).unwrap_or_else(|| text.chars().take(60).collect());
            if let Err(e) = c.add_url(&text, &opts) {
                if cfg.notify { notify("Torrent add failed", &e.to_string(), true); }
                return Err(e);
            }
            if cfg.notify { notify("Torrent added", &label, false); }
            emit(json, serde_json::json!({"ok": true, "action": "add", "hash": magnet_hash(&text), "name": label}), || println!("added: {label}"));
            Ok(Code::Ok)
        }
        Cmd::Reconcile { recent } => {
            connect(&cfg, &c)?;
            let n = c.reconcile(cfg.sequential, cfg.first_last, cfg.seed_by_default, recent)?;
            emit(json, serde_json::json!({"ok": true, "action": "reconcile", "touched": n}), || println!("reconciled {n} torrent(s)"));
            Ok(Code::Ok)
        }
        Cmd::Open { selector } => {
            connect(&cfg, &c)?;
            let ts = c.torrents()?;
            let h = resolve_in(&ts, &[selector])?;
            let t = ts.iter().find(|t| t.hash == h[0]).unwrap();
            let target = qbt::open_target(t);
            Command::new("xdg-open").arg(&target).spawn()?;
            emit(json, serde_json::json!({"ok": true, "action": "open", "hash": t.hash, "target": target}), || println!("opened {target}"));
            Ok(Code::Ok)
        }
        Cmd::Rss(sub) => { connect(&cfg, &c)?; rss(&c, sub, json, &cfg) }
        Cmd::Search { query, wait, limit, install, install_url, plugins } => {
            connect(&cfg, &c)?;
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
        Cmd::Capabilities | Cmd::RobotDocs { .. } | Cmd::Setup { .. } | Cmd::RegisterMagnet | Cmd::Config | Cmd::Imdb { .. } => unreachable!(),
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
        let done = mine.iter().all(|t| t.progress >= 1.0);
        let stopped = mine.iter().filter(|t| t.is_stopped() && t.progress < 1.0).count();
        if !json || !done {
            let line = mine.iter().map(|t| format!("{} {:.1}% {}", &t.hash[..8], t.progress * 100.0, if t.is_active() { human_rate(t.dlspeed) } else { t.state.clone() })).collect::<Vec<_>>().join(" | ");
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
            let progress: Vec<serde_json::Value> = mine.iter().map(|t| serde_json::json!({"hash": t.hash, "name": t.name, "state": t.state, "progress": t.progress, "eta": t.eta})).collect();
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

fn rss(c: &Client, sub: RssCmd, json: bool, cfg: &Config) -> Result<Code> {
    match sub {
        RssCmd::List => {
            let feeds = c.rss_feeds()?;
            emit(json, serde_json::json!({"ok": true, "count": feeds.len(), "feeds": feeds}), || {
                for f in &feeds {
                    let unread = f.articles.iter().filter(|a| !a.is_read).count();
                    println!("{:<30} {:>4} unread / {:>4}  {}{}", f.name, unread, f.articles.len(), f.url, if f.has_error { "  [error]" } else { "" });
                }
            });
        }
        RssCmd::Add { url, name } => {
            let n = name.clone().or_else(|| host_of(&url)).unwrap_or_else(|| url.clone());
            c.rss_add_feed(&url, Some(&n))?;
            emit(json, serde_json::json!({"ok": true, "action": "rss-add", "name": n, "url": url}), || println!("added feed {n}"));
        }
        RssCmd::Remove { name } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            c.rss_remove(&f.name)?;
            emit(json, serde_json::json!({"ok": true, "action": "rss-remove", "name": f.name}), || println!("removed {}", f.name));
        }
        RssCmd::Refresh { name } => {
            let feeds = c.rss_feeds()?;
            let names: Vec<String> = match name {
                Some(n) => vec![find_feed(&feeds, &n)?.name.clone()],
                None => feeds.iter().map(|f| f.name.clone()).collect(),
            };
            for n in &names { c.rss_refresh(n)?; }
            emit(json, serde_json::json!({"ok": true, "action": "rss-refresh", "feeds": names}), || println!("refreshing {} feed(s)", names.len()));
        }
        RssCmd::Items { name, unread } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            let items: Vec<_> = f.articles.iter().filter(|a| !unread || !a.is_read).collect();
            emit(json, serde_json::json!({"ok": true, "feed": f.name, "count": items.len(), "items": items}), || {
                for a in &items { println!("{}  {}  {}  {}", if a.is_read { " " } else { "*" }, qbt::ymd(a.ts), a.id, a.title); }
            });
        }
        RssCmd::Grab { name, article_id } => {
            let feeds = c.rss_feeds()?;
            let f = find_feed(&feeds, &name)?;
            let Some(a) = f.articles.iter().find(|a| a.id == article_id) else {
                return Err(AppError::new(Code::NotFound, format!("no item with id '{article_id}' in feed '{}'", f.name))
                    .hint(format!("fbittorrent rss items \"{}\" --json | jq '.items[].id'", f.name)).into());
            };
            let url = if a.torrent_url.is_empty() { &a.link } else { &a.torrent_url };
            c.add_url(url, &AddOptions {
                sequential: cfg.sequential, first_last: cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            c.rss_mark_read(&f.name, Some(&a.id))?;
            if cfg.notify { notify("Torrent added", &a.title, false); }
            emit(json, serde_json::json!({"ok": true, "action": "rss-grab", "hash": magnet_hash(url), "name": a.title, "feed": f.name}), || println!("added: {}", a.title));
        }
    }
    Ok(Code::Ok)
}

fn find_feed<'a>(feeds: &'a [qbt::RssFeed], name: &str) -> Result<&'a qbt::RssFeed> {
    if let Some(f) = feeds.iter().find(|f| f.name == name) { return Ok(f); }
    let low = name.to_lowercase();
    let m: Vec<&qbt::RssFeed> = feeds.iter().filter(|f| f.name.to_lowercase().contains(&low)).collect();
    match m.len() {
        1 => Ok(m[0]),
        0 => Err(AppError::new(Code::NotFound, format!("no feed named '{name}'"))
            .hint("fbittorrent rss list --json | jq -r '.feeds[].name'").into()),
        n => Err(AppError::new(Code::Ambiguous, format!("'{name}' matches {n} feeds: {}", m.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "))).into()),
    }
}

/// Expand selectors (hash prefix, unique name substring, or `all`) to full hashes.
fn resolve(c: &Client, selectors: &[String]) -> Result<Vec<String>> {
    let ts = c.torrents()?;
    resolve_in(&ts, selectors)
}

fn resolve_in(ts: &[Torrent], selectors: &[String]) -> Result<Vec<String>> {
    if selectors.is_empty() {
        return Err(AppError::new(Code::Usage, "no torrent selector given").hint("fbittorrent list --json | jq -r '.torrents[].hash'   # then pass a hash prefix, a unique part of the name, or `all`").into());
    }
    if selectors.len() == 1 && selectors[0] == "all" {
        return Ok(ts.iter().map(|t| t.hash.clone()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    for sel in selectors {
        let s = sel.trim();
        let low = s.to_lowercase();
        let is_hexish = low.len() >= 4 && low.chars().all(|c| c.is_ascii_hexdigit());
        let by_hash: Vec<&Torrent> = if is_hexish { ts.iter().filter(|t| t.hash.starts_with(&low)).collect() } else { vec![] };
        let matches: Vec<&Torrent> = if !by_hash.is_empty() { by_hash } else {
            ts.iter().filter(|t| t.name.to_lowercase().contains(&low)).collect()
        };
        match matches.len() {
            1 => { let h = matches[0].hash.clone(); if !out.contains(&h) { out.push(h); } }
            0 => return Err(AppError::new(Code::NotFound, format!("no torrent matches '{s}'"))
                .hint("fbittorrent list --json | jq -r '.torrents[] | \"\\(.hash[0:8])  \\(.name)\"'").into()),
            n => return Err(AppError::new(Code::Ambiguous, format!("'{s}' matches {n} torrents: {}",
                    matches.iter().take(6).map(|t| format!("{} ({})", &t.hash[..8], short(&t.name, 40))).collect::<Vec<_>>().join(", ")))
                .hint("use a longer hash prefix, or the exact name").into()),
        }
    }
    Ok(out)
}

/// Collapse duplicate torrents from several providers (same info hash), keeping the best-seeded copy.
fn tui_free_dedupe(rs: Vec<qbt::SearchResult>) -> Vec<qbt::SearchResult> {
    let mut best: std::collections::HashMap<String, qbt::SearchResult> = std::collections::HashMap::new();
    for r in rs {
        let k = magnet_hash(&r.file_url).unwrap_or_else(|| r.file_url.to_lowercase());
        match best.get(&k) { Some(b) if b.seeders >= r.seeders => {}, _ => { best.insert(k, r); } }
    }
    best.into_values().collect()
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// Info hash from a magnet link (lowercase hex), if present.
pub fn magnet_hash(src: &str) -> Option<String> {
    let low = src.to_ascii_lowercase();
    let i = low.find("urn:btih:")?;
    let h: String = low[i + 9..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if h.len() == 40 { Some(h) } else if h.len() == 32 { Some(base32_to_hex(&h)) } else { None }
}

fn base32_to_hex(s: &str) -> String {
    let mut bits = 0u64;
    let mut nbits = 0;
    let mut out = String::new();
    for c in s.chars() {
        let v = match c { 'a'..='z' => c as u64 - 'a' as u64, '2'..='7' => c as u64 - '2' as u64 + 26, _ => continue };
        bits = (bits << 5) | v; nbits += 5;
        while nbits >= 8 { nbits -= 8; out.push_str(&format!("{:02x}", (bits >> nbits) & 0xff)); }
    }
    out
}

pub fn magnet_name(src: &str) -> Option<String> {
    let q = src.split_once('?')?.1;
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("dn=") { return Some(percent_decode(v)); }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { out.push(b' '); i += 1; }
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(v); i += 3; } else { out.push(b'%'); i += 1; }
            }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    Some(rest.split('/').next()?.trim_start_matches("www.").to_string())
}

fn notify(title: &str, body: &str, error: bool) {
    let _ = Command::new("notify-send")
        .args(["-a", "fbittorrent", "-i", "qbittorrent", "-u", if error { "critical" } else { "normal" }, title, body])
        .spawn();
}
