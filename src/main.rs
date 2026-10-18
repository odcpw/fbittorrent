mod agent;
mod backend;
mod browse;
mod config;
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
use qbt::{human_bytes, human_eta, human_rate, AddOptions, Client, Torrent};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "fbittorrent",
    version,
    about = "Torrent app for Omarchy: native engine, CLI, TUI, bar widget.",
    long_about = "Torrent app for Omarchy. The native engine runs while the app is open.\n\nBare `fbittorrent` opens the TUI on a terminal; without a TTY it prints `triage --json`.\nEvery command accepts --json. Agents: start with `fbittorrent robot-docs`.",
    after_help = "Exit codes: 0 ok, 2 usage, 3 safety (needs --yes), 4 backend down, 5 upstream, 6 not found, 7 ambiguous, 8 timeout.\nSelect torrents by hash prefix or a unique part of the name; `all` selects every torrent."
)]
struct Cli {
    /// Machine-readable JSON on stdout (errors too, with the same exit code)
    #[arg(long, global = true)]
    json: bool,
    /// Require an already open app; never start the torrent engine
    #[arg(long, global = true)]
    no_start: bool,
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
    /// Open or focus the app (starts the service only in explicit legacy mode)
    Up,
    /// Close the app and stop its engine (legacy mode stops the service)
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
    /// Toggle sequential download (legacy mode; native scheduling is intrinsic)
    Seq { selectors: Vec<String> },
    /// Toggle first/last piece priority (legacy mode; native scheduling is intrinsic)
    FirstLast { selectors: Vec<String> },
    /// Allow uploading/seeding
    Seed { selectors: Vec<String> },
    /// Limit uploading and stop when complete (default for new torrents)
    Noseed { selectors: Vec<String> },
    /// Native scheduling stub; legacy mode applies defaults to outside additions
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
    /// Followed TV shows: a local list checked against EZTV (no service, no timer)
    #[command(subcommand, alias = "show-list")]
    Shows(ShowsCmd),
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
    /// Configure the app download folder (explicit legacy mode also sets up its service)
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
        /// Start on the Shows screen
        #[arg(long)]
        shows: bool,
    },
    /// Show config path and effective values
    Config,
    /// Download queue, peer encryption and VPN connection settings
    Settings {
        #[arg(long)]
        max_downloads: Option<usize>,
        /// off, prefer, or require
        #[arg(long)]
        encryption: Option<String>,
        /// proton, an interface name, or off
        #[arg(long)]
        vpn: Option<String>,
        /// List available network interface names
        #[arg(long)]
        interfaces: bool,
    },
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

#[derive(Subcommand)]
enum ShowsCmd {
    /// Followed shows with new / seen / grabbed counts
    #[command(alias = "ls")]
    List,
    /// Follow a show by IMDb id (tt…) or title
    #[command(alias = "add")]
    Follow {
        query: Vec<String>,
        /// Preferred quality for this show (default: config show_quality)
        #[arg(long)]
        quality: Option<String>,
    },
    /// Stop following a show (IMDb id or unique part of the title)
    #[command(alias = "rm", alias = "remove")]
    Unfollow { show: String },
    /// Fetch the episode lists from EZTV; one show, or all. Prints what is new.
    #[command(alias = "refresh")]
    Check { show: Option<String> },
    /// Episodes of a show from the last check
    #[command(alias = "items")]
    Episodes {
        show: String,
        /// Only releases not yet marked read
        #[arg(long)]
        new: bool,
    },
    /// Add the best torrent for an episode (S01E02) or season pack (S01)
    Grab { show: String, episode: String },
    /// Mark episodes seen (toggle); `all` marks every episode of the show
    Seen { show: String, episodes: Vec<String> },
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
    // `triage` and the TUI are the two bare-invocation faces; `triage` is always JSON.
    let json = json || matches!(cmd, Cmd::Triage | Cmd::Capabilities);
    if cfg.native() && matches!(&cmd, Cmd::Seq { .. } | Cmd::FirstLast { .. }) {
        return Err(AppError::new(Code::Usage, "native sequential and first/last scheduling are intrinsic; toggle commands require legacy mode")
            .hint("explicitly configure engine = \"qbittorrent\" to use legacy scheduling controls").into());
    }

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
        Cmd::Settings { max_downloads, encryption, vpn, interfaces } => {
            anyhow::ensure!(cfg.native(), "these settings require the native engine");
            let mut changed = cfg.clone();
            if let Some(limit) = max_downloads { changed.max_active_downloads = limit; }
            if let Some(policy) = encryption { changed.peer_encryption = policy; }
            if let Some(selection) = vpn { changed.network_interface = if selection == "off" { None } else { Some(selection) }; }
            changed.validate()?;
            let altered = changed.max_active_downloads != cfg.max_active_downloads || changed.peer_encryption != cfg.peer_encryption || changed.network_interface != cfg.network_interface;
            if altered {
                let client = Client::from_config(&cfg);
                if ipc::is_running(&cfg) && changed.peer_encryption == cfg.peer_encryption && changed.network_interface == cfg.network_interface {
                    client.set_queue_limit(changed.max_active_downloads)?;
                } else {
                    changed.save()?;
                    if ipc::is_running(&cfg) { client.restart_app()?; }
                }
            }
            let available = if interfaces { Some(network_app::interfaces()?) } else { None };
            emit(json, serde_json::json!({"ok": true, "max_active_downloads": changed.max_active_downloads, "peer_encryption": changed.peer_encryption, "network_interface": changed.network_interface, "network": network_app::state(&changed), "interfaces": available}), || {
                println!("Simultaneous downloads: {}", changed.max_active_downloads);
                println!("Peer encryption: {}", changed.peer_encryption);
                println!("VPN: {}", network_app::state(&changed).message);
                if let Some(names) = &available { println!("Connections: {}", names.join(", ")); }
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
        Cmd::Up => {
            let mut explicit = cfg.clone();
            explicit.autostart = !no_start;
            let started = backend::ensure_running(&explicit, &c)?;
            if cfg.native() && !started && !no_start { backend::open_app()?; }
            emit(json, serde_json::json!({"ok": true, "action": "up", "started": started, "engine": cfg.engine, "unit": if cfg.native() { None } else { Some(&cfg.backend_unit) }}), || {
                if cfg.native() { println!("fbittorrent {}", if started { "opened" } else if no_start { "already open" } else { "focused" }); }
                else { println!("{} {}", cfg.backend_unit, if started { "started" } else { "already running" }); }
            });
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
            if cfg.native() && !no_start { run_native_app(cfg, screen)?; }
            else { connect(&cfg, &c)?; tui::run(cfg, screen)?; }
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
                let progress = if t.progress_known { format!("{:.1}%  {} of {}", t.progress * 100.0, human_bytes(t.completed), human_bytes(t.size)) } else { "checked when the VPN connects".into() };
                println!("{}\n  hash      {}\n  state     {}\n  progress  {}\n  speed     ↓ {}  ↑ {}\n  eta       {}\n  peers     {} seeds, {} leechers\n  flags     seq={} first/last={} upload={}\n  path      {}\n  open      {}",
                    t.name, t.hash, t.state, progress,
                    human_rate(t.dlspeed), human_rate(t.upspeed), human_eta(t.eta), t.num_seeds, t.num_leechs,
                    t.seq_dl, t.f_l_piece_prio, if t.seed_allowed() { "allowed" } else { "off" }, t.save_path, qbt::open_target(&t));
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
        Cmd::Add { source, no_seq, no_first_last, stopped, seed, save, category, quiet, wait, timeout } => {
            if cfg.native() && (no_seq || no_first_last) {
                return Err(AppError::new(Code::Usage, "the native engine downloads sequentially with first/last pieces prioritised; those settings cannot be disabled")
                    .hint("drop --no-seq and --no-first-last, or explicitly configure engine = \"qbittorrent\" for legacy scheduling").into());
            }
            connect(&cfg, &c)?;
            let opts = AddOptions {
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
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
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
            Command::new("xdg-open").arg(&target).spawn()?;
            emit(json, serde_json::json!({"ok": true, "action": "open", "hash": t.hash, "target": target}), || println!("opened {target}"));
            Ok(Code::Ok)
        }
        Cmd::Rss(sub) => {
            if !cfg.native() || matches!(&sub, RssCmd::Grab { .. }) { connect(&cfg, &c)?; }
            rss(&c, sub, json, &cfg)
        }
        Cmd::Shows(sub) => shows_cmd(&cfg, &c, sub, json),
        Cmd::Search { query, wait, limit, install, install_url, plugins } => {
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
        Cmd::Capabilities | Cmd::RobotDocs { .. } | Cmd::Setup { .. } | Cmd::RegisterMagnet | Cmd::Config | Cmd::Settings { .. } | Cmd::Imdb { .. } => unreachable!(),
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

fn run_native_app(mut cfg: Config, screen: tui::Screen) -> Result<()> {
    if ipc::is_running(&cfg) { return backend::open_app(); }
    loop {
    let snapshot = network_app::state(&cfg);
    network_app::initialize(snapshot.clone());
    if !snapshot.ready {
        let parked = network_app::ParkedEngine::start(&cfg, snapshot)?;
        let result = tui::run(cfg.clone(), screen);
        drop(parked);
        result?;
        if network_app::restart_requested() && !network_app::close_requested() {
            cfg = Config::load()?;
            continue;
        }
        return Ok(());
    }
    let mut engine_cfg = cfg.clone();
    if cfg.network_interface.is_some() { engine_cfg.network_interface = snapshot.interface.clone(); }
    let mut app = match engine::start(&engine_cfg) {
        Ok(app) => app,
        Err(error) if error.to_string().contains("already open") => return backend::open_app(),
        Err(_) if cfg.network_interface.is_some() && network_app::state(&cfg) != snapshot => continue,
        Err(error) => return Err(error),
    };
    let dispatcher = app.dispatcher();
    migrate_legacy(&cfg, &dispatcher)?;
    app.serve(&cfg, move |request| {
        network_app::controls(request, &snapshot).or_else(|| dispatcher.dispatch(request))
            .unwrap_or_else(|| Err(anyhow::anyhow!("unsupported native engine request {}", request.path)))
    })?;
    // The guard drops before run() returns, including a TUI initialization error.
    // main() only calls process::exit after network activity and searches stop.
    let result = tui::run(cfg.clone(), screen);
    drop(app);
    result?;
    if network_app::restart_requested() && !network_app::close_requested() {
        cfg = Config::load()?;
        continue;
    }
    return Ok(());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LegacyMigration {
    phase: String,
    source_url: String,
    source_unit: String,
    backup: std::path::PathBuf,
    torrents: Vec<Torrent>,
    files: std::collections::HashMap<String, Vec<qbt::TorrentFile>>,
    feeds: Vec<qbt::RssFeed>,
    plugins: Vec<qbt::SearchPlugin>,
}

impl LegacyMigration {
    fn save(&self, path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = path.parent().context("migration path has no parent")?;
        std::fs::create_dir_all(directory)?;
        let staged = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true)
            .mode(0o600).open(&staged)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        std::fs::rename(&staged, path)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
}

fn migrate_legacy(cfg: &Config, dispatcher: &engine::EngineDispatcher) -> Result<()> {
    let path = config::data_dir().join("legacy-migration.json");
    let mut source_cfg = cfg.clone();
    source_cfg.engine = "qbittorrent".into();
    let mut migration: LegacyMigration = if path.exists() {
        let saved: LegacyMigration = serde_json::from_slice(&std::fs::read(&path)?)
            .context("reading legacy migration progress; the qBittorrent source is preserved")?;
        if saved.phase == "complete" { return Ok(()); }
        anyhow::ensure!(matches!(saved.phase.as_str(), "captured" | "prepared"), "unknown legacy migration phase; source preserved");
        source_cfg.url = saved.source_url.clone();
        source_cfg.backend_unit = saved.source_unit.clone();
        saved
    } else {
        let source = Client::from_config(&source_cfg);
        if source.login().is_err() || source.version().is_err() {
            anyhow::ensure!(!backend::is_active(&source_cfg.backend_unit),
                "the legacy qBittorrent service is active but unavailable; migration is deferred and its records are preserved");
            if !backend::start_for_legacy_migration(&source_cfg, &source)? { return Ok(()); }
        }
        anyhow::ensure!(source_cfg.url.starts_with("http://127.0.0.1:") || source_cfg.url.starts_with("http://localhost:"),
            "automatic legacy migration requires a local qBittorrent WebUI; the source is preserved");
        source.login().context("logging into legacy qBittorrent before migration")?;
        let torrents = source.torrents().context("capturing all legacy torrents")?;
        let mut files = std::collections::HashMap::new();
        for torrent in &torrents {
            let selected = source.files(&torrent.hash)
                .with_context(|| format!("capturing legacy file selection for {}", torrent.name))?;
            anyhow::ensure!(!selected.is_empty(), "legacy torrent {} has no resolved file metadata; source preserved", torrent.name);
            files.insert(torrent.hash.clone(), selected);
        }
        let saved = LegacyMigration {
            phase: "captured".into(),
            source_url: source_cfg.url.clone(),
            source_unit: source_cfg.backend_unit.clone(),
            backup: dirs::data_local_dir().context("no local data directory for qBittorrent backup")?
                .join("qBittorrent/BT_backup"),
            torrents,
            files,
            feeds: source.rss_feeds().context("capturing legacy RSS feeds and read history")?,
            plugins: source.search_plugins().context("capturing legacy provider preferences")?,
        };
        saved.save(&path).context("saving legacy migration snapshot; source preserved")?;
        saved
    };

    if migration.phase == "captured" {
        if migration.files.values().flatten().any(|file| !matches!(file.priority, 0 | 1)) {
            eprintln!("legacy file priorities above normal become normal; native file selection supports skip and normal");
        }
        let count = dispatcher.import_legacy(migration.torrents.clone(), migration.files.clone(), migration.backup.clone())
            .context("importing legacy torrents paused; qBittorrent remains available")?;
        rss::import(&migration.feeds).context("saving legacy RSS history; qBittorrent remains available")?;
        search::import_plugins(&migration.plugins).context("saving legacy search preferences; qBittorrent remains available")?;
        migration.phase = "prepared".into();
        migration.save(&path).context("saving completed import before retiring the source")?;
        eprintln!("saved {count} legacy torrent(s), {} RSS feed(s), and provider preferences", migration.feeds.len());
    }

    let source = Client::from_config(&source_cfg);
    if legacy_endpoint_reachable(&source_cfg.url)? {
        source.login()?;
        let current = source.torrents()?;
        let same_hashes = current.len() == migration.torrents.len()
            && current.iter().all(|torrent| migration.torrents.iter().any(|old| old.hash == torrent.hash));
        anyhow::ensure!(same_hashes, "legacy torrents changed during migration; source kept running and imported torrents remain paused");
        for torrent in &current {
            let old = migration.torrents.iter().find(|old| old.hash == torrent.hash).unwrap();
            anyhow::ensure!(migration_torrent_settings(old) == migration_torrent_settings(torrent),
                "legacy settings changed for {}; source kept running and imported torrents remain paused", torrent.name);
            let files = source.files(&torrent.hash)?;
            let selection = |items: &[qbt::TorrentFile]| items.iter().map(|file| (file.index, file.name.clone(), file.priority)).collect::<Vec<_>>();
            anyhow::ensure!(selection(&files) == selection(&migration.files[&torrent.hash]),
                "legacy file selection changed for {}; source kept running and imported torrents remain paused", torrent.name);
        }
    }
    backend::stop_legacy(&source_cfg).context("retiring the legacy service after saved import; native torrents remain paused")?;
    anyhow::ensure!(!legacy_endpoint_reachable(&source_cfg.url)?,
        "legacy qBittorrent still answers after stopping its service; close that instance before native downloads can resume");
    anyhow::ensure!(dispatcher.legacy_migration_completed()?, "native import has not been durably completed; source records preserved");
    backend::disable_legacy(&source_cfg).context("retiring legacy login startup after verified import")?;
    dispatcher.resume_imported().context("resuming only the legacy torrents that were running")?;
    migration.phase = "complete".into();
    migration.save(&path).context("saving completed legacy migration")?;
    eprintln!("legacy migration complete; downloads now run while fbittorrent is open");
    Ok(())
}

fn migration_torrent_settings(torrent: &Torrent) -> serde_json::Value {
    serde_json::json!({
        "hash": torrent.hash, "name": torrent.name, "save_path": torrent.save_path,
        "content_path": torrent.content_path, "category": torrent.category, "tags": torrent.tags,
        "up_limit": torrent.up_limit, "ratio_limit": torrent.ratio_limit,
        "stopped": torrent.is_stopped(),
    })
}

fn legacy_endpoint_reachable(url: &str) -> Result<bool> {
    let authority = url.strip_prefix("http://").context("legacy migration requires a local HTTP WebUI")?
        .split('/').next().unwrap_or_default();
    let (host, port) = authority.rsplit_once(':').context("legacy WebUI URL has no port")?;
    anyhow::ensure!(matches!(host, "127.0.0.1" | "localhost"), "legacy migration requires a loopback WebUI");
    let port: u16 = port.parse().context("invalid legacy WebUI port")?;
    Ok(std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(250)).is_ok())
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
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            c.rss_mark_read(&f.name, Some(&a.id))?;
            if cfg.notify { notify("Torrent added", &a.title, false); }
            emit(json, serde_json::json!({"ok": true, "action": "rss-grab", "hash": magnet_hash(url), "name": a.title, "feed": f.name}), || println!("added: {}", a.title));
        }
    }
    Ok(Code::Ok)
}

fn shows_cmd(cfg: &Config, c: &Client, sub: ShowsCmd, json: bool) -> Result<Code> {
    use shows::{EpState, Shows};
    let mut all = Shows::load()?;
    let print_eps = |s: &shows::Show, eps: &[&shows::Episode]| {
        for e in eps {
            let mark = match s.state(&e.key) { EpState::New => "*", EpState::Grabbed => "↓", EpState::Seen => " " };
            println!("{mark} {}  {:>5}  {:>9}  {:>4} seeds  {}", e.key, e.quality, human_bytes(e.size), e.seeds, e.title);
        }
    };
    match sub {
        ShowsCmd::List => {
            emit(json, serde_json::json!({"ok": true, "count": all.shows.len(), "new": all.new_total(), "path": shows::path(),
                "shows": all.shows.iter().map(|s| s.to_json()).collect::<Vec<_>>()}), || {
                if all.shows.is_empty() { println!("no shows followed yet: fbittorrent shows follow \"<title or tt id>\""); }
                for s in &all.shows {
                    let checked = if s.last_checked > 0 { qbt::ymd(s.last_checked) } else { "never".into() };
                    println!("{:<10}  {:<40} {:>3} new / {:>3} eps  checked {}{}", s.imdb, short(&s.label(), 40), s.new_count(), s.episodes.len(), checked,
                        s.error.as_deref().map(|e| format!("  [error: {e}]")).unwrap_or_default());
                }
            });
        }
        ShowsCmd::Follow { query, quality } => {
            let q = query.join(" ");
            if q.trim().is_empty() { return Err(AppError::new(Code::Usage, "give a title or an IMDb id").hint("fbittorrent shows follow \"The Bear\"").into()); }
            let t = shows::resolve_title(&q)?;
            let added = all.follow(&t);
            let s = all.get_mut(&t.id).unwrap();
            if quality.is_some() { s.quality = quality; }
            let fresh = shows::refresh(s, &cfg.show_quality).ok();
            let (label, n_new, n_eps) = (s.label(), s.new_count(), s.episodes.len());
            let warning = s.error.clone();
            let catalogue_complete = s.catalogue_complete;
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-follow", "added": added, "imdb": t.id, "title": t.name, "year": t.year,
                "kind": t.kind, "episodes": n_eps, "new": n_new, "fetched": fresh.is_some(), "warning": warning, "catalogue_complete": catalogue_complete}), || {
                    println!("{} {label} [{}]: {n_eps} releases on EZTV, {n_new} new", if added { "following" } else { "already following" }, t.id);
                    if let Some(warning) = &warning { println!("warning: {warning}"); }
                });
        }
        ShowsCmd::Unfollow { show } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.unfollow(&id).unwrap();
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-unfollow", "imdb": s.imdb, "title": s.title}), || println!("unfollowed {}", s.label()));
        }
        ShowsCmd::Check { show } => {
            let ids: Vec<String> = match show {
                Some(sel) => vec![all.find(&sel)?.imdb.clone()],
                None => all.shows.iter().map(|s| s.imdb.clone()).collect(),
            };
            let mut report = Vec::new();
            let mut failed = 0;
            for id in &ids {
                let s = all.get_mut(id).unwrap();
                let fresh = match shows::refresh(s, &cfg.show_quality) { Ok(n) => n, Err(_) => { failed += 1; 0 } };
                report.push((s.imdb.clone(), s.label(), fresh, s.new_count(), s.error.clone()));
            }
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-check", "checked": ids.len(), "failed": failed, "new": all.new_total(),
                "shows": report.iter().map(|(id, l, f, n, e)| serde_json::json!({"imdb": id, "title": l, "fresh": f, "new": n, "error": e})).collect::<Vec<_>>()}), || {
                for (_, l, f, n, e) in &report {
                    match e { Some(e) => println!("{l}: {e}"), None => println!("{l}: {f} fresh, {n} new") }
                }
                for s in all.shows.iter().filter(|s| ids.contains(&s.imdb)) {
                    let eps: Vec<&shows::Episode> = s.episodes.iter().filter(|e| s.is_new(&e.key)).collect();
                    if eps.is_empty() { continue; }
                    println!("\n{}", s.label());
                    print_eps(s, &eps);
                }
            });
            if failed > 0 && failed == ids.len() { return Ok(Code::Upstream); }
        }
        ShowsCmd::Episodes { show, new } => {
            let s = all.find(&show)?;
            let eps: Vec<&shows::Episode> = s.episodes.iter().filter(|e| !new || s.is_new(&e.key)).collect();
            emit(json, serde_json::json!({"ok": true, "imdb": s.imdb, "title": s.title, "last_checked": s.last_checked, "count": eps.len(),
                "warning": s.error, "catalogue_complete": s.catalogue_complete,
                "episodes": eps.iter().map(|e| s.episode_json(e)).collect::<Vec<_>>()}), || {
                if eps.is_empty() { println!("{}: nothing listed{}", s.label(), if s.last_checked == 0 { " (never checked: fbittorrent shows check)" } else { "" }); }
                if let Some(warning) = &s.error { println!("warning: {warning}"); }
                print_eps(s, &eps);
            });
        }
        ShowsCmd::Grab { show, episode } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.get_mut(&id).unwrap();
            let Some(e) = s.episode(&episode).cloned() else {
                return Err(AppError::new(Code::NotFound, format!("{} has no episode {}", s.title, episode.to_uppercase()))
                    .hint(format!("fbittorrent shows episodes \"{}\" --json", s.imdb)).into());
            };
            connect(cfg, c)?;
            c.add_url(&e.magnet, &AddOptions {
                sequential: cfg.native() || cfg.sequential, first_last: cfg.native() || cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            s.mark_grabbed(&e.key, &e.magnet);
            let title = s.title.clone();
            all.save()?;
            if cfg.notify { notify("Torrent added", &e.title, false); }
            emit(json, serde_json::json!({"ok": true, "action": "shows-grab", "imdb": id, "show": title, "episode": e.key, "hash": magnet_hash(&e.magnet), "name": e.title}),
                || println!("added: {}", e.title));
        }
        ShowsCmd::Seen { show, episodes } => {
            let id = all.find(&show)?.imdb.clone();
            let s = all.get_mut(&id).unwrap();
            let mut changed = Vec::new();
            if episodes.iter().any(|e| e == "all") {
                let n = s.mark_all_seen();
                changed.push(serde_json::json!({"all": true, "marked": n}));
            } else {
                if episodes.is_empty() { return Err(AppError::new(Code::Usage, "give release keys like S01E02 or S01, or `all`").into()); }
                for k in &episodes {
                    let key = k.to_ascii_uppercase();
                    if s.episode(&key).is_none() {
                        return Err(AppError::new(Code::NotFound, format!("{} has no episode {key}", s.title))
                            .hint(format!("fbittorrent shows episodes \"{}\" --json", s.imdb)).into());
                    }
                    let seen = s.toggle_seen(&key);
                    changed.push(serde_json::json!({"episode": key, "seen": seen}));
                }
            }
            let (title, n_new) = (s.title.clone(), s.new_count());
            all.save()?;
            emit(json, serde_json::json!({"ok": true, "action": "shows-seen", "imdb": id, "show": title, "changed": changed, "new": n_new}),
                || println!("{title}: {n_new} new left"));
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
    fn hex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { out.push(b' '); i += 1; }
            b'%' if i + 2 < b.len() => {
                if let Some(v) = hex(b[i + 1]).zip(hex(b[i + 2])).map(|(a, b)| a * 16 + b) {
                    out.push(v); i += 3;
                } else { out.push(b'%'); i += 1; }
            }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn local_torrent_source(source: &str) -> Result<String> {
    if !source.starts_with("file:") { return Ok(source.to_owned()); }
    let uri = url::Url::parse(source).context("invalid local torrent file URL")?;
    let path = uri.to_file_path().map_err(|_| anyhow::anyhow!("torrent file URL must name a local file"))?;
    path.into_os_string().into_string().map_err(|_| anyhow::anyhow!("torrent file path must be valid UTF-8"))
}

#[cfg(test)]
mod source_tests {
    use super::*;

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
        .args(["-a", "fbittorrent", "-i", "qbittorrent", "-u", if error { "critical" } else { "normal" }, title, body])
        .spawn();
}
