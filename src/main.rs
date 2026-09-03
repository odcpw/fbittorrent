mod backend;
mod browse;
mod config;
mod imdb;
mod qbt;
mod release;
mod setup;
mod tui;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use config::Config;
use qbt::{human_bytes, human_eta, human_rate, AddOptions, Client};
use std::process::Command;

#[derive(Parser)]
#[command(name = "fbittorrent", version, about = "Fast qBittorrent front-end for Omarchy. No command = open the TUI.")]
struct Cli {
    /// Machine-readable JSON output where supported
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Transfer rates and torrent counts (used by the bar widget). Never starts the backend.
    Status,
    /// Start the qbittorrent-nox user service
    Up,
    /// Stop the qbittorrent-nox user service
    Down,
    /// List torrents
    List {
        /// Only torrents still downloading
        #[arg(long)]
        active: bool,
    },
    /// Add a magnet link, URL or .torrent file. Sequential + first/last piece are on by default.
    Add {
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
    },
    /// Start (resume) torrents by hash prefix
    Start { hashes: Vec<String> },
    /// Stop (pause) torrents by hash prefix
    Stop { hashes: Vec<String> },
    /// Delete torrents by hash prefix
    Delete {
        hashes: Vec<String>,
        /// Also delete downloaded files
        #[arg(long)]
        files: bool,
    },
    /// Toggle sequential download for torrents by hash prefix
    Seq { hashes: Vec<String> },
    /// Toggle first/last piece priority for torrents by hash prefix
    FirstLast { hashes: Vec<String> },
    /// Allow uploading/seeding for torrents by hash prefix
    Seed { hashes: Vec<String> },
    /// Forbid uploading: 1 B/s cap, stop when complete (default for new torrents)
    Noseed { hashes: Vec<String> },
    /// Enforce sequential + first/last on downloading torrents
    Reconcile {
        /// Only touch torrents added within the last N seconds (0 = all)
        #[arg(long, default_value_t = 0)]
        recent: i64,
    },
    /// Add whatever magnet/URL is on the Wayland clipboard
    AddClipboard,
    /// Open the save folder of a torrent in the file manager
    Open { hash: String },
    /// RSS feeds
    #[command(subcommand)]
    Rss(RssCmd),
    /// Search torrents through qBittorrent's search plugins
    Search {
        /// Query. Use `--install` once to fetch the official search plugins.
        query: Vec<String>,
        /// Install/update the official plugins plus a curated set of community providers, then exit
        #[arg(long)]
        install: bool,
        /// Install a search plugin from a URL (repeatable), then exit
        #[arg(long = "install-url")]
        install_url: Vec<String>,
        /// List installed search plugins, then exit
        #[arg(long)]
        plugins: bool,
        /// Seconds to wait for results
        #[arg(long, default_value_t = 15)]
        wait: u64,
        #[arg(long, default_value_t = 30)]
        limit: i64,
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
    List,
    /// Add a feed
    Add { url: String, #[arg(long)] name: Option<String> },
    /// Remove a feed by name
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
    if let Err(e) = run() {
        eprintln!("fbittorrent: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load()?;
    let cmd = cli.cmd.unwrap_or(Cmd::Tui { search: false, rss: false });

    match cmd {
        Cmd::Setup { port, save, magnet } => {
            setup::run(port, save)?;
            if magnet { setup::register_magnet_handler()?; }
            return Ok(());
        }
        Cmd::RegisterMagnet => return setup::register_magnet_handler(),
        Cmd::Imdb { name, refresh } => {
            if refresh || imdb::ratings_stale() {
                eprintln!("fetching IMDb ratings dataset…");
                imdb::refresh_ratings()?;
            }
            let n = name.join(" ");
            let Some(t) = imdb::lookup(&n)? else { println!("no match"); return Ok(()); };
            if cli.json { println!("{}", serde_json::to_string_pretty(&t)?); return Ok(()); }
            println!("{} ({}) [{}]  {}", t.name, t.year.map(|y| y.to_string()).unwrap_or_default(), t.kind, t.url());
            match (t.rating, t.votes) { (Some(r), Some(v)) => println!("★ {r:.1}  ({v} votes)"), _ => println!("★ n/a") }
            if !t.stars.is_empty() { println!("{}", t.stars); }
            if !t.blurb.is_empty() { println!("
{}
{}", t.blurb, t.wiki_url); }
            if !t.poster_url.is_empty() {
                match imdb::fetch_poster(&t.poster_url, 300) { Ok(p) => println!("poster: {}", p.display()), Err(e) => println!("poster: {e}") }
            }
            return Ok(());
        }
        Cmd::Config => {
            println!("# {}", config::config_path().display());
            print!("{}", toml::to_string_pretty(&cfg)?);
            return Ok(());
        }
        _ => {}
    }

    let c = Client::from_config(&cfg);

    match cmd {
        Cmd::Status => {
            if c.version().is_err() {
                if cli.json { println!("{}", serde_json::json!({"ok": false, "backend": if backend::is_active(&cfg.backend_unit) { "starting" } else { "stopped" }})); }
                else { println!("qBittorrent is not running ({})", cfg.backend_unit); }
                return Ok(());
            }
            c.login()?;
            let ti = c.transfer_info()?;
            let ts = c.torrents()?;
            let active = ts.iter().filter(|t| t.is_active()).count();
            let seeding = ts.iter().filter(|t| t.is_seeding()).count();
            let stopped = ts.iter().filter(|t| t.is_stopped()).count();
            if cli.json {
                let mut top: Vec<&qbt::Torrent> = ts.iter().filter(|t| t.is_active()).collect();
                top.extend(ts.iter().filter(|t| !t.is_active()).take(12));
                println!("{}", serde_json::json!({
                    "ok": true,
                    "dl": ti.dl_info_speed, "up": ti.up_info_speed,
                    "active": active, "seeding": seeding, "stopped": stopped, "total": ts.len(),
                    "connection": ti.connection_status,
                    "torrents": top,
                }));
            } else {
                println!("↓ {}  ↑ {}  | {} active, {} seeding, {} stopped, {} total",
                    human_rate(ti.dl_info_speed), human_rate(ti.up_info_speed), active, seeding, stopped, ts.len());
            }
        }
        Cmd::Up => {
            if backend::ensure_running(&cfg, &c)? { println!("started {}", cfg.backend_unit); }
            else { println!("{} already running", cfg.backend_unit); }
        }
        Cmd::Down => {
            backend::stop(&cfg)?;
            println!("stopped {}", cfg.backend_unit);
        }
        Cmd::Tui { search, rss } => {
            connect(&cfg, &c)?;
            let screen = if search { tui::Screen::Search } else if rss { tui::Screen::Rss } else { tui::Screen::Torrents };
            tui::run(cfg, screen)?;
        }
        Cmd::List { active } => {
            connect(&cfg, &c)?;
            let ts: Vec<_> = c.torrents()?.into_iter().filter(|t| !active || t.is_active()).collect();
            if cli.json { println!("{}", serde_json::to_string(&ts)?); return Ok(()); }
            for t in &ts {
                let flags = format!("{}{}{}", if t.seq_dl { "S" } else { "-" }, if t.f_l_piece_prio { "F" } else { "-" }, if t.seed_allowed() { "U" } else { "-" });
                println!("{}  {:>5.1}%  {:>11}  {:>6}  {}  {:<12} {}",
                    &t.hash[..8], t.progress * 100.0, human_rate(t.dlspeed), human_eta(t.eta), flags, t.state, t.name);
            }
        }
        Cmd::Add { source, no_seq, no_first_last, stopped, seed, save, category, quiet } => {
            connect(&cfg, &c)?;
            let opts = AddOptions {
                sequential: cfg.sequential && !no_seq,
                first_last: cfg.first_last && !no_first_last,
                seed: cfg.seed_by_default || seed,
                stopped,
                save_path: save.or(cfg.save_path.clone()),
                category: category.or(cfg.category.clone()),
            };
            let src = if std::path::Path::new(&source).is_file() {
                bail!(".torrent file adds are not implemented yet; pass a magnet or http(s) URL");
            } else { source.clone() };
            let label = magnet_name(&src).unwrap_or_else(|| src.chars().take(60).collect());
            match c.add_url(&src, &opts) {
                Ok(()) => {
                    if cli.json { println!("{}", serde_json::json!({"ok": true, "name": label})); }
                    else { println!("added: {label}"); }
                    if cfg.notify && !quiet { notify("Torrent added", &label, false); }
                }
                Err(e) => {
                    if cfg.notify && !quiet { notify("Torrent add failed", &e.to_string(), true); }
                    return Err(e);
                }
            }
        }
        Cmd::Start { hashes } => { connect(&cfg, &c)?; c.start(&resolve(&c, &hashes)?)? }
        Cmd::Stop { hashes } => { connect(&cfg, &c)?; c.stop(&resolve(&c, &hashes)?)? }
        Cmd::Delete { hashes, files } => { connect(&cfg, &c)?; c.delete(&resolve(&c, &hashes)?, files)? }
        Cmd::Seq { hashes } => { connect(&cfg, &c)?; c.toggle_sequential(&resolve(&c, &hashes)?)? }
        Cmd::FirstLast { hashes } => { connect(&cfg, &c)?; c.toggle_first_last(&resolve(&c, &hashes)?)? }
        Cmd::Seed { hashes } => { connect(&cfg, &c)?; c.allow_seeding(&resolve(&c, &hashes)?)?; println!("seeding allowed"); }
        Cmd::Noseed { hashes } => { connect(&cfg, &c)?; c.deny_seeding(&resolve(&c, &hashes)?)?; println!("upload off"); }
        Cmd::AddClipboard => {
            connect(&cfg, &c)?;
            let out = Command::new("wl-paste").arg("--no-newline").output()?;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !(text.starts_with("magnet:") || text.starts_with("http://") || text.starts_with("https://")) {
                if cfg.notify { notify("Nothing to add", "Clipboard holds no magnet link or URL", true); }
                bail!("clipboard holds no magnet link or URL");
            }
            let opts = AddOptions {
                sequential: cfg.sequential, first_last: cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            };
            let label = magnet_name(&text).unwrap_or_else(|| text.chars().take(60).collect());
            match c.add_url(&text, &opts) {
                Ok(()) => { println!("added: {label}"); if cfg.notify { notify("Torrent added", &label, false); } }
                Err(e) => { if cfg.notify { notify("Torrent add failed", &e.to_string(), true); } return Err(e); }
            }
        }
        Cmd::Reconcile { recent } => {
            connect(&cfg, &c)?;
            let n = c.reconcile(cfg.sequential, cfg.first_last, cfg.seed_by_default, recent)?;
            if cli.json { println!("{}", serde_json::json!({"ok": true, "touched": n})); }
            else { println!("reconciled {n} torrent(s)"); }
        }
        Cmd::Open { hash } => {
            connect(&cfg, &c)?;
            let h = resolve(&c, &[hash])?;
            let ts = c.torrents()?;
            let t = ts.iter().find(|t| t.hash == h[0]).unwrap();
            Command::new("xdg-open").arg(qbt::open_target(t)).spawn()?;
        }
        Cmd::Rss(sub) => { connect(&cfg, &c)?; rss(&c, sub, cli.json, &cfg)? }
        Cmd::Search { query, wait, limit, install, install_url, plugins } => {
            connect(&cfg, &c)?;
            if plugins {
                for p in c.search_plugins()? {
                    println!("{:<18} {:<3} {:<8} {}", p.name, if p.enabled { "on" } else { "off" }, p.version, p.url);
                }
                return Ok(());
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
                // Installation is asynchronous; give qBittorrent a moment, then enable everything.
                for _ in 0..30 {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if c.search_plugins()?.len() >= before.max(srcs.len()) { break; }
                }
                c.search_enable_all()?;
                let ps = c.search_plugins()?;
                println!("{} search plugins installed: {}", ps.len(), ps.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
                return Ok(());
            }
            let q = query.join(" ");
            if q.trim().is_empty() { bail!("empty query"); }
            c.search_enable_all().ok();
            let id = c.search_start(&q)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait);
            let mut last;
            loop {
                last = match c.search_results(id, limit, 0) {
                    Ok(r) => r,
                    // The job can take a moment to become queryable right after start.
                    Err(e) if e.to_string().contains("404") && std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                if last.status != "Running" || std::time::Instant::now() >= deadline { break; }
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
            let _ = c.search_stop(id);
            let _ = c.search_delete(id);
            last.results.sort_by(|a, b| b.seeders.cmp(&a.seeders));
            if cli.json { println!("{}", serde_json::to_string(&last.results)?); return Ok(()); }
            for r in &last.results {
                println!("{:>5}S {:>5}L  {:>10}  {:<12}  {}", r.seeders, r.leechers, human_bytes(r.file_size), r.engine, r.file_name);
            }
        }
        Cmd::Setup { .. } | Cmd::RegisterMagnet | Cmd::Config | Cmd::Imdb { .. } => unreachable!(),
    }
    Ok(())
}

fn connect(cfg: &Config, c: &Client) -> Result<()> {
    backend::ensure_running(cfg, c)?;
    c.login()
}

fn rss(c: &Client, sub: RssCmd, json: bool, cfg: &Config) -> Result<()> {
    match sub {
        RssCmd::List => {
            let feeds = c.rss_feeds()?;
            if json { println!("{}", serde_json::to_string(&feeds)?); return Ok(()); }
            for f in &feeds {
                let unread = f.articles.iter().filter(|a| !a.is_read).count();
                println!("{:<30} {:>4} unread / {:>4}  {}{}", f.name, unread, f.articles.len(), f.url,
                    if f.has_error { "  [error]" } else { "" });
            }
        }
        RssCmd::Add { url, name } => {
            let n = name.clone().or_else(|| host_of(&url));
            c.rss_add_feed(&url, n.as_deref())?;
            println!("added feed {}", n.unwrap_or(url));
        }
        RssCmd::Remove { name } => { c.rss_remove(&name)?; println!("removed {name}"); }
        RssCmd::Refresh { name } => {
            match name {
                Some(n) => c.rss_refresh(&n)?,
                None => for f in c.rss_feeds()? { c.rss_refresh(&f.name)?; },
            }
            println!("refreshing");
        }
        RssCmd::Items { name, unread } => {
            let feeds = c.rss_feeds()?;
            let Some(f) = feeds.iter().find(|f| f.name == name) else { bail!("no feed named {name}") };
            let items: Vec<_> = f.articles.iter().filter(|a| !unread || !a.is_read).collect();
            if json { println!("{}", serde_json::to_string(&items)?); return Ok(()); }
            for a in items {
                println!("{}  {}  {}  {}", if a.is_read { " " } else { "*" }, qbt::ymd(a.ts), a.id, a.title);
            }
        }
        RssCmd::Grab { name, article_id } => {
            let feeds = c.rss_feeds()?;
            let Some(f) = feeds.iter().find(|f| f.name == name) else { bail!("no feed named {name}") };
            let Some(a) = f.articles.iter().find(|a| a.id == article_id) else { bail!("no article {article_id}") };
            let url = if a.torrent_url.is_empty() { &a.link } else { &a.torrent_url };
            c.add_url(url, &AddOptions {
                sequential: cfg.sequential, first_last: cfg.first_last, seed: cfg.seed_by_default, stopped: false,
                save_path: cfg.save_path.clone(), category: cfg.category.clone(),
            })?;
            c.rss_mark_read(&f.name, Some(&a.id))?;
            println!("added: {}", a.title);
            if cfg.notify { notify("Torrent added", &a.title, false); }
        }
    }
    Ok(())
}

/// Expand hash prefixes to full hashes.
fn resolve(c: &Client, prefixes: &[String]) -> Result<Vec<String>> {
    if prefixes.is_empty() { bail!("no torrent hash given"); }
    if prefixes.len() == 1 && prefixes[0] == "all" {
        return Ok(c.torrents()?.into_iter().map(|t| t.hash).collect());
    }
    let ts = c.torrents()?;
    let mut out = Vec::new();
    for p in prefixes {
        let m: Vec<&str> = ts.iter().filter(|t| t.hash.starts_with(&p.to_lowercase())).map(|t| t.hash.as_str()).collect();
        match m.len() {
            0 => bail!("no torrent matches {p}"),
            1 => out.push(m[0].to_string()),
            _ => bail!("{p} is ambiguous ({} matches)", m.len()),
        }
    }
    Ok(out)
}

pub fn magnet_name(src: &str) -> Option<String> {
    let q = src.split_once('?')?.1;
    for kv in q.split('&') {
        if let Some(v) = kv.strip_prefix("dn=") {
            return Some(percent_decode(v));
        }
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
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(v); i += 3; }
                else { out.push(b'%'); i += 1; }
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
