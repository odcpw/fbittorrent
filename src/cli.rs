//! Command declarations, kept separate from execution and owner lifetime.
use super::*;
pub mod shapes;
pub(super) mod catalog_search;
pub(super) mod catalog_imdb;
mod fallback;
pub(super) mod logs;
pub(super) mod stream;
pub(super) mod raw;
pub(super) mod restart;
mod native;
mod handler;
mod offline;
pub(super) mod settings;
pub(super) mod limits;
pub(super) use native::{handles as native_handles, run as run_native};

#[derive(Parser)]
#[command(
    name = "fbittorrent",
    version,
    about = "Torrent app for Omarchy: native engine, CLI, TUI, bar widget.",
    long_about = "Torrent app for Omarchy. Native writes start the headless daemon on demand.\n\nBare `fbittorrent` opens the TUI on a terminal; without a TTY it prints `triage --json`.\nOrdinary CLI commands accept --json; serve logs to stderr. Agents: start with `fbittorrent robot-docs`.",
    after_help = "Exit codes: 0 ok, 2 usage, 3 safety (needs --yes), 4 backend down, 5 upstream, 6 not found, 7 ambiguous, 8 timeout.\nSelect torrents by hash prefix or a unique part of the name; `all` selects every torrent."
)]
pub(super) struct Cli {
    /// Machine-readable JSON on stdout (errors too, with the same exit code)
    #[arg(long, global = true)]
    pub(super) json: bool,
    /// Require an already open app; never start the torrent engine
    #[arg(long, global = true)]
    pub(super) no_start: bool,
    /// Shortcut for `triage --json`
    #[arg(long = "robot-triage", hide = true)]
    pub(super) robot_triage: bool,
    #[command(subcommand)]
    pub(super) cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub(super) enum Cmd {
    /// Replay retained daemon events; --follow streams until EOF or Ctrl+C
    Events {
        #[arg(long)] follow: bool,
        #[arg(long)] since_seq: Option<u64>,
        #[arg(long, value_delimiter=',')] kinds: Vec<String>,
    },
    /// Send a raw API operation or list its compiled catalogue
    Api {
        #[arg(required_unless_present="list")] op: Option<String>,
        #[arg(long, conflicts_with="op")] list: bool,
        #[arg(long="arg", requires="op")] args: Vec<String>,
        #[arg(long, requires="op")] json_args: Option<String>,
        #[arg(long="ref", requires="op")] reference: Option<String>,
    },
    /// Read daemon history without starting it; --follow streams NDJSON
    Log {
        #[arg(long)]
        follow: bool,
        #[arg(long, value_parser=["error","warn","info","debug"])]
        level: Option<String>,
        #[arg(long)]
        source: Option<String>,
    },
    /// Run the torrent engine daemon (other commands start it on demand)
    Serve {
        /// Override the configured idle delay (default 300); 0 disables it
        #[arg(long)]
        idle_exit_secs: Option<u64>,
        /// Disable desktop notifications (systemd readiness is still sent)
        #[arg(long)]
        no_notify: bool,
    },
    /// One-call overview for agents: backend, rates, torrents, warnings, next commands
    #[command(alias = "robot-triage")]
    Triage,
    /// Machine-readable contract: commands, exit codes, conventions
    Capabilities,
    /// Agent handbook (plain text)
    #[command(alias = "docs")]
    RobotDocs {
        /// Section name: `guide` or `keys`
        #[arg(default_value = "guide")]
        section: String,
        /// Keyboard surface to document (keys section only)
        #[arg(long, default_value = "tui", value_parser = ["tui", "dashboard", "popup", "all"])]
        surface: String,
    },
    /// Transfer rates and torrent counts (used by the bar widget). Never starts the backend.
    Status {
        /// Print nothing; exit 0 when running and 4 when stopped.
        #[arg(long)]
        quiet: bool,
    },
    /// Start the daemon; --ui also opens the dashboard or TUI
    Up { #[arg(long)] ui: bool },
    /// Open the dashboard, falling back to the TUI when the panel is absent
    Dashboard { #[arg(long)] view: Option<String>, #[arg(long)] query: Option<String>, #[arg(long)] hash: Option<String> },
    /// Read or persist primary and turtle limits; zero means unlimited
    Limits {
        #[arg(long)] down: Option<String>,
        #[arg(long)] up: Option<String>,
        #[arg(long, value_parser = ["on", "off"])] alt: Option<String>,
        #[arg(long)] alt_down: Option<String>,
        #[arg(long)] alt_up: Option<String>,
    },
    /// Restart the running daemon and wait up to 30 seconds for its replacement
    Restart,
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
        /// Attribution retained by the daemon.
        #[arg(long, default_value="manual", value_parser=["manual","handler","agent"])]
        origin: String,
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
        /// Show a plain notification even when stdout is not a TTY (never with --json)
        #[arg(long)]
        notify: bool,
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
        /// Wait for completion, the verified playback threshold or stall-free playback
        #[arg(long, value_enum, default_value_t = Until::Complete)]
        until: Until,
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
    /// Move a torrent (or all) to another directory, waiting for the move receipt
    Move { selector:String, save_path:String },
    /// Ask live torrents to announce now (at most once per tracker per minute)
    Reannounce { selectors: Vec<String> },
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
        /// Link the checkout's plugin or copy it for a stable installation
        #[arg(long, value_parser=["link", "copy"])]
        plugin: Option<String>,
        /// List exact planned actions without writing files or running commands
        #[arg(long)]
        dry_run: bool,
        /// Remove desktop integration while keeping saved data
        #[arg(long, conflicts_with_all = ["magnet", "save", "plugin"])]
        uninstall: bool,
        /// Remove integration and application data (never downloaded payloads)
        #[arg(long, conflicts_with_all = ["magnet", "save", "plugin"])]
        purge: bool,
        /// Confirm application-data removal with --purge
        #[arg(long)]
        yes: bool,
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
        #[arg(long)]
        idle_secs: Option<u64>,
        #[arg(long)]
        handler: Option<String>,
        #[arg(long, num_args=0..=1, default_missing_value="true", value_parser=clap::builder::BoolishValueParser::new())]
        clipboard_watch: Option<bool>,
        #[arg(long)]
        port: Option<u16>,
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
pub(super) enum RssCmd {
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
pub(super) enum ShowsCmd {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_idle_override_preserves_config_default_and_short_test_delays() {
        let cli = Cli::try_parse_from(["fbittorrent", "serve"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Serve { idle_exit_secs: None, .. })));
        let cli = Cli::try_parse_from(["fbittorrent", "serve", "--idle-exit-secs", "2"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Serve { idle_exit_secs: Some(2), .. })));
    }
    #[test]
    fn robot_docs_keys_selects_each_surface_and_json() {
        for surface in ["tui","dashboard","popup","all"] {
            let cli=Cli::try_parse_from(["fbittorrent","robot-docs","keys","--surface",surface,"--json"]).unwrap();
            assert!(cli.json);
            assert!(matches!(cli.cmd,Some(Cmd::RobotDocs { section,surface: selected }) if section=="keys" && selected==surface));
        }
        let cli=Cli::try_parse_from(["fbittorrent","robot-docs","keys"]).unwrap();
        assert!(matches!(cli.cmd,Some(Cmd::RobotDocs { surface,.. }) if surface=="tui"));
        assert!(Cli::try_parse_from(["fbittorrent","robot-docs","keys","--surface","bogus"]).is_err());
        let caps=agent::capabilities_json();
        assert!(caps["commands"].as_array().unwrap().iter().any(|row|row["name"]=="robot-docs keys" && row["json"]==true));
    }
}

/// Documentation is available even when configuration is invalid or absent.
pub(super) fn robot_docs(section: &str, surface: &str, json: bool) -> Result<Code> {
    match section {
        "api" => {
            if json { println!("{}", serde_json::json!({"ok":true,"operations":crate::api::ops::TABLE})); }
            else { print!("{}",crate::api::ops::markdown()); }
        }
        "keys" => {
            if json { println!("{}", tui::actions::keymap(surface)); }
            else { print!("{}", tui::actions::markdown(surface)); }
        }
        "guide" => if json { println!("{}", agent::with_warnings(serde_json::json!({"ok":true,"section":"guide","text":agent::ROBOT_GUIDE.to_string()}))); } else { print!("{}", agent::ROBOT_GUIDE); },
        _ => return Err(AppError::new(Code::Usage, format!("unknown docs section '{section}'"))
            .hint("fbittorrent robot-docs guide   # or: robot-docs api / robot-docs keys --surface all").into()),
    }
    Ok(Code::Ok)
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Until { Complete, Playable, StallFree }
