//! Agent-facing surface: exit-code dictionary, typed errors, JSON envelopes,
//! `capabilities`, `robot-docs`, and the `triage` mega-command.

use serde::{Deserialize, Serialize};
use std::fmt;

thread_local! { static WARNINGS: std::cell::RefCell<Vec<serde_json::Value>> = const { std::cell::RefCell::new(Vec::new()) }; }
pub fn warning(code: &str, message: String, stderr: bool) {
    if stderr { eprintln!("{message}"); }
    WARNINGS.with(|list| { let mut list=list.borrow_mut(); let item=serde_json::json!({"code":code,"message":message}); if !list.contains(&item){list.push(item);} });
}
pub fn with_warnings(mut value:serde_json::Value)->serde_json::Value {
    WARNINGS.with(|list| {let list=list.borrow();if !list.is_empty(){value["warnings"]=serde_json::json!(*list);}});value
}
pub const CONTRACT_VERSION: &str = "1.2.32";

const NATIVE_LIFETIME: &str = "The daemon (fbittorrent serve) owns the engine independently of windows and its replaceable engine slot; clients connect through api.sock. Reads never start it; writes and a TUI start it on demand. The TUI attaches in the current terminal; q or Ctrl+C closes only that UI, while Q (confirmed when active) or down stops the daemon.";
const SERVE_LIMITATIONS: &str = "serve stops on owner shutdown or idle_exit_secs (default 300; 0 disables idle exit). Transfers, interactive clients, searches and pending mutations keep it alive; a bar-only client never counts as interactive, a visible dashboard on its shared socket does, and an open glance pauses the countdown. systemd starts it on demand for writes or a TUI. API v1 uses api.sock for the CLI, TUI, subscriptions and settings. Settings use the private config while stopped; replaced binaries restart the daemon when idle; restart switches immediately.";

const JSON_CONVENTION: &str = "Ordinary CLI commands with --json print one JSON object on stdout; errors use {ok:false,code,code_name,error,hint} and the same exit code. serve is a long-running owner with stderr diagnostics, not a JSON response command; --json only formats its startup errors. events and log --follow stream NDJSON; api preserves the server response/error envelope and maps its API exit code.";
const UNIT_EXITS: &str = "serve unit-only exits: 70 panic; 75 upgrade. These are outside the CLI 0..8 dictionary.";

/// Exit codes. Stable; documented by `capabilities --json` and `robot-docs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum Code {
    Ok = 0,
    /// Something unexpected (bug, I/O). Retrying rarely helps.
    Failure = 1,
    /// Bad arguments or input. Same code clap uses for usage errors.
    Usage = 2,
    /// Refused for safety; the hint names the flag that overrides.
    Safety = 3,
    /// The torrent engine or app is unavailable or would not start.
    Backend = 4,
    /// The torrent engine or a network provider answered with an error.
    Upstream = 5,
    /// No torrent / feed / plugin matched the given selector.
    NotFound = 6,
    /// A selector matched more than one item.
    Ambiguous = 7,
    /// A wait ran out of time; the state was reported, nothing is broken.
    Timeout = 8,
}

impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Code::Ok => "ok",
            Code::Failure => "failure",
            Code::Usage => "usage",
            Code::Safety => "safety",
            Code::Backend => "backend",
            Code::Upstream => "upstream",
            Code::NotFound => "not_found",
            Code::Ambiguous => "ambiguous",
            Code::Timeout => "timeout",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Code::Ok => "success",
            Code::Failure => "unexpected failure (bug or I/O); retrying rarely helps",
            Code::Usage => "invalid arguments or input; the message names the exact fix",
            Code::Safety => "refused for safety; re-run with the flag named in the hint",
            Code::Backend => "torrent app unavailable; run `fbittorrent up` or `fbittorrent setup`",
            Code::Upstream => "torrent engine or a provider returned an error; inspect the message before retrying",
            Code::NotFound => "no torrent, feed or plugin matched; `fbittorrent list --json` shows what exists",
            Code::Ambiguous => "selector matched several items; use a longer hash prefix",
            Code::Timeout => "`wait` timed out; the torrent is still there, poll `show <hash> --json`",
        }
    }
    pub const ALL: [Code; 9] = [Code::Ok, Code::Failure, Code::Usage, Code::Safety, Code::Backend, Code::Upstream, Code::NotFound, Code::Ambiguous, Code::Timeout];
}

/// An error with an exit code and a copy-pasteable next step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: Code,
    pub message: String,
    pub hint: Option<String>,
}

impl AppError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), hint: None }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.message) }
}
impl std::error::Error for AppError {}

/// Turn any error into (code, message, hint). Typed errors keep their code;
/// client errors are classified by what they say.
pub fn classify(e: &anyhow::Error) -> (Code, String, Option<String>) {
    if let Some(invalid)=e.downcast_ref::<crate::search::torznab::Invalid>() {return (Code::Usage,invalid.to_string(),None);}
    if let Some(ae) = e.downcast_ref::<AppError>() {
        return (ae.code, ae.message.clone(), ae.hint.clone());
    }
    if let Some(io)=e.downcast_ref::<std::io::Error>() { if matches!(io.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::ConnectionRefused) { return (Code::Backend,format!("fbittorrent is closed: {io}"),Some("fbittorrent up".into())); } }
    let msg = format!("{e:#}");
    let low = msg.to_ascii_lowercase();
    if low.contains("fbittorrent is closed") || low.contains("opening the fbittorrent app") || low.contains("not running") || low.contains("not installed") || low.contains("did not come up") {
        return (Code::Backend, msg, Some("fbittorrent up   # opens the app; `fbittorrent setup` configures its download folder".into()));
    }
    if low.contains("returned http") || low.contains("http ") || low.contains("timed out") || low.contains("dns") || low.contains("connection") {
        return (Code::Upstream, msg, Some("fbittorrent status --json   # confirm the backend answers, then retry".into()));
    }
    (Code::Failure, msg, None)
}

#[derive(Serialize)]
pub struct ErrorEnvelope<'a> {
    pub ok: bool,
    pub code: i32,
    pub code_name: &'static str,
    pub error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'a str>,
}

/// Print an error the way the caller asked for it and return the exit code.
pub fn report(e: &anyhow::Error, json: bool) -> i32 {
    let (code, msg, hint) = classify(e);
    if json {
        let env = ErrorEnvelope { ok: false, code: code as i32, code_name: code.name(), error: &msg, hint: hint.as_deref() };
        println!("{}", with_warnings(serde_json::to_value(&env).unwrap()));
    } else {
        eprintln!("fbittorrent: error[{}]: {}", code.name(), msg);
        if let Some(h) = hint { eprintln!("  try: {h}"); }
    }
    code as i32
}

pub fn stdout_is_tty() -> bool {
    unsafe { libc::isatty(1) == 1 }
}

// ---------------------------------------------------------------- capabilities

#[derive(Serialize)]
struct CommandDoc { name: &'static str, aliases: &'static [&'static str], summary: &'static str, json: bool, mutating: bool, needs_backend: bool }

fn upload_cap_text() -> String {
    "off by default: zero piece payload while downloading or complete; enable upload in settings or explicitly for one torrent (non-zero caps must be at least 16 KiB/s)".into()
}

pub fn capabilities_json() -> serde_json::Value {
    let changes = vec![
        serde_json::json!({"since":"1.2.32","version":"1.2.32","breaking":false,"field":"status.session","cap":"history","note":"P6-04 status includes current daemon-session downloaded/uploaded network bytes and session start; plain status shows session and all-time totals"}),
        serde_json::json!({"since":"1.2.32","version":"1.2.32","breaking":false,"field":"torrents.rename","cap":"rename","note":"display-name overrides preserve metadata names, payload paths and torrent identity"}),
        serde_json::json!({"since":"1.2.32","version":"1.2.32","breaking":false,"field":"status.all_time","cap":"history","note":"P6-04 real session network counters and atomic all-time statistics surviving daemon restarts"}),
        serde_json::json!({"since":"1.2.31","version":"1.2.31","breaking":false,"field":"status.history","cap":"history","note":"P6-03 one bounded 600-sample global speed ring, 60-point summary/status and five-second stats topic"}),
        serde_json::json!({"since":"1.2.31","version":"1.2.31","breaking":false,"field":"pieces","cap":"pieces","note":"P6-02 verified MSB-first base64 bitfield and sampled detail topic"}),
        serde_json::json!({"since":"1.2.31","version":"1.2.31","breaking":false,"field":"cli.catalogue","note":"RSS/Shows CLI uses shared native handlers and normalized views on both sources"}),
        serde_json::json!({"since":"1.2.30","version":"1.2.30","breaking":false,"field":"shows.quality","cap":"shows","note":"P3-18 durable per-show quality override, inbox filters and CLI shows quality without autostart"}),
        serde_json::json!({"since":"1.2.30","version":"1.2.30","breaking":false,"field":"trackers.edit","cap":"tracker_edit","note":"P7-04 live idempotent tracker add/remove with durable edits and bounded URL validation"}),
        serde_json::json!({"since":"1.2.29","version":"1.2.29","breaking":false,"field":"search.torznab","cap":"torznab","note":"native Torznab providers; strict private indexers/key files, HTTPS for keyed remote endpoints, bounded same-origin requests and secret-free results"}),
        serde_json::json!({"since":"1.2.28","version":"1.2.28","breaking":false,"field":"search.install_torznab","note":"CLI search --install-torznab/--key-file and Torznab provider kind; credentials stay in private files"}),
        serde_json::json!({"since":"1.2.27","version":"1.2.27","breaking":false,"field":"download_limit_bps","cap":"download_limit","note":"P7-06 durable live per-torrent download caps, effective cap field and limit --down CLI without rechecking"}),
        serde_json::json!({"since":"1.2.26","version":"1.2.26","breaking":false,"field":"list[].num_seeds","cap":"peers","note":"P5-01 fills connected seed counts for live torrents; show adds peers[] with one-second rates and optional seeds"}),
        serde_json::json!({"since":"1.2.25","version":"1.2.25","breaking":false,"field":"playable","cap":"watch","note":"verified video prefix readiness, once-per-lifetime playback notification and wait --until playable|stall-free"}),
        serde_json::json!({"since":"1.2.24","version":"1.2.24","breaking":false,"field":"search.results[].providers","note":"CLI search uses daemon or local normalized items with identical fields and deterministic provider provenance"}),
        serde_json::json!({"since":"1.2.23","version":"1.2.23","breaking":false,"field":"app.client.dashboard_visible","cap":"dashboard_presence","note":"M1a visible dashboard keeps its shared bar connection interactive; hidden dashboard and bar-only connections do not"}),
        serde_json::json!({"since":"1.2.22","version":"1.2.22","breaking":false,"field":"api.staging","note":"stage metainfo without payload access; commit choices or discard without deleting data","cap":"staging"}),
        serde_json::json!({"since":"1.2.21","version":"1.2.21","breaking":false,"field":"api.move","note":"pending/ref payload moves preserve verification, save path and CLI act bookkeeping","cap":"move"}),
        serde_json::json!({"since":"1.2.20","version":"1.2.20","breaking":false,"field":"shows","cap":"shows","note":"P3-03 daemon Shows inbox, stale interactive checks, read/undo/grab intents and pushed topics"}),
        serde_json::json!({"since":"1.2.19","version":"1.2.19","breaking":false,"field":"rss","cap":"rss","note":"P3-02 daemon RSS operations, feed/item topics, interactive stale refresh and bounded feeds"}),
        serde_json::json!({"since":"1.2.18","version":"1.2.18","breaking":false,"field":"clipboard","cap":"clipboard","note":"P6-13 opt-in bounded NUL-framed clipboard magnet offers, dismiss operation, notification actions and settings on/off"}),
        serde_json::json!({"since":"1.2.17","version":"1.2.17","breaking":false,"field":"add.disk","note":"P5-07 known disk capacity/warnings; summary disk monitoring and disk_full recovery on a 16 MiB/piece free-space rise"}),
        serde_json::json!({"since":"1.2.16","version":"1.2.16","breaking":false,"field":"app.free_space","cap":"free_space","note":"P6-11: read-only destination space and writability; native add --save refuses an unwritable folder with exit 2"}),
        serde_json::json!({"since":"1.2.15","version":"1.2.15","breaking":false,"field":"caps.imdb","cap":"imdb","note":"P3-04 implemented IMDb capability recorded in exact hello/error fixtures"}),
        serde_json::json!({"since":"1.2.14","version":"1.2.14","breaking":false,"field":"limits","cap":"limits","note":"live global/turtle limits, persisted alt switch, structural summary fields and offline CLI edits"}),
        serde_json::json!({"since":"1.2.13","version":"1.2.13","breaking":false,"field":"add.confirmation_required","note":"handler confirm opens the dashboard review before downloading; dashboard accepts --hash for staged records"}),
        serde_json::json!({"since":"1.2.12","version":"1.2.12","breaking":false,"field":"trackers","cap":"trackers","note":"tracker diagnostics and rate-limited reannounce; show adds trackers and tracker_count"}),
        serde_json::json!({"since":"1.2.11","version":"1.2.11","breaking":false,"field":"setup.actions","note":"planned native desktop integration; dry-run, plugin link/copy, uninstall and confirmed purge"}),
        serde_json::json!({"since":"1.2.10","version":"1.2.10","breaking":false,"field":"api.search","cap":"search","note":"daemon search sessions, keyed deduplicated results, providers, browse and grab"}),
        serde_json::json!({"since":"1.2.9","version":"1.2.9","breaking":false,"field":"api","note":"unchanged raw response envelope; compiled operation catalogue and per-operation startup/retry policy"}),
        serde_json::json!({"since":"1.2.8","version":"1.2.8","breaking":false,"field":"events","note":"NDJSON history/follow with instance and seq, replay cursors and resync metadata"}),
        serde_json::json!({"since":"1.2.7","version":"1.2.7","breaking":false,"field":"restart.server","note":"restart drains the daemon and waits for its replacement instance; binary replacement is deferred while busy"}),
        serde_json::json!({"since":"1.2.6","version":"1.2.6","breaking":false,"field":"log","cap":"log","note":"app.log, log topic and CLI history/follow; app.stats distinguishes active clients and cumulative connections"}),
        serde_json::json!({"since":"1.2.5","version":"1.2.5","breaking":false,"field":"api.origin","cap":"origin","note":"durable open origin, manual-bucket filters, keep-files batch undo_until and restore source"}),
        serde_json::json!({"since":"1.2.4","version":"1.2.4","breaking":false,"field":"api.queue","cap":"queue","note":"queue capability, force-start, queue movement and positioned torrent rows"}),
        serde_json::json!({"since":"1.2.3","version":"1.2.3","breaking":false,"field":"api.torrents.recheck","cap":"recheck","note":"recheck capability, pending ref acknowledgement and measured verification result"}),
        serde_json::json!({"since":"1.2.2","version":"1.2.2","breaking":false,"field":"settings","cap":"settings","note":"durable app.settings.get/set; CLI adds applied, restart, running and canonical settings keys"}),
        serde_json::json!({"since":"1.2.1","version":"1.2.1","breaking":false,"field":"add.deferred","cap":"deferred_add","note":"parked magnet/local metainfo adds return deferred:true and retain intent until VPN returns"}),
        serde_json::json!({"since":"1.2.0","breaking":true,"field":"status.ok","note":"stopped status is successful with running:false"}),
        serde_json::json!({"since":"1.2.0","breaking":false,"field":"list[].eta","note":"legacy sentinel remains 8640000; read eta_s for known values"}),
        serde_json::json!({"since":"1.2.0","breaking":false,"field":"list[].native_state","note":"normalized state, checked, reason and origin are additive"}),
        serde_json::json!({"since":"1.2.0","breaking":false,"field":"add.duplicate","note":"atomic API result; add --origin manual|handler|agent"}),
        serde_json::json!({"since":"1.2.0","version":"1.2.0","field":"list[].num_seeds","breaking":false,"note":"always 0 in native mode: unknown; list[].num_leechs is the connected peer count"}),
    ];
    let cmds = [
        CommandDoc { name: "serve", aliases: &[], summary: "Run the torrent engine daemon (other commands start it on demand)", json: false, mutating: true, needs_backend: false },
        CommandDoc { name: "triage", aliases: &["robot-triage"], summary: "One-call overview: backend, rates, torrents, warnings, next commands", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "status", aliases: &[], summary: "Rates and counts; never starts the backend", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "list", aliases: &["ls"], summary: "All torrents with hash, progress, speed, flags", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "show", aliases: &["info"], summary: "One torrent in full, including its files and the path to open", json: true, mutating: false, needs_backend: true },
        CommandDoc { name:"move", aliases:&[], summary:"Move a torrent or all to another directory and wait for its ref result", json:true, mutating:true, needs_backend:true },
        CommandDoc { name:"reannounce", aliases:&[], summary:"Force a live torrent tracker announce, rate limited to once per minute", json:true, mutating:true, needs_backend:true },
        CommandDoc { name: "search", aliases: &["find"], summary: "Search local providers; results carry magnet links; native mode does not open the app", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "events", aliases: &[], summary: "Replay/follow NDJSON events with instance and seq; never starts", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "api", aliases: &[], summary: "Raw API envelope; --list catalogue; mutating and startup policy depend on the operation", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "limits", aliases: &[], summary: "Persist global and alternative limits; never starts a stopped daemon", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "log", aliases: &[], summary: "Read daemon history; --follow streams NDJSON without starting the daemon", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "add", aliases: &["download", "get"], summary: "Add a magnet or URL; --wait blocks until complete; --notify requests a plain toast; --origin handler delegates actions to the daemon", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "wait", aliases: &[], summary: "Block until torrents complete (or --timeout); progress on stderr", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "start", aliases: &["resume"], summary: "Start torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "stop", aliases: &["pause"], summary: "Stop torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "delete", aliases: &["rm", "remove"], summary: "Remove torrents; --files deletes data and needs --yes; --dry-run previews", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "seed", aliases: &[], summary: "Allow upload for torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "noseed", aliases: &[], summary: "Cap upload and stop when complete (default for new torrents)", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "open", aliases: &[], summary: "Open the file or folder of a torrent in the desktop", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "rss", aliases: &[], summary: "Local list | add | remove | refresh | items; only grab needs the native engine", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "shows", aliases: &[], summary: "Followed TV shows (local list, checked against EZTV): list | follow | unfollow | check | episodes | grab | seen", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "imdb", aliases: &[], summary: "Title, rating, blurb and poster for a release name", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "up", aliases: &[], summary: "Start the daemon; --ui also opens the dashboard or TUI", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "dashboard", aliases: &[], summary: "Start the daemon and summon the dashboard, falling back to the TUI", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "restart", aliases: &[], summary: "Restart the daemon now; wait up to 30 seconds for its replacement hello", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "down", aliases: &[], summary: "Stop the daemon and wait for engine cleanup", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "config", aliases: &[], summary: "Effective configuration", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "settings", aliases: &[], summary: "Set simultaneous downloads, peer encryption and required VPN connection; --interfaces lists connections", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "capabilities", aliases: &[], summary: "This document", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs keys", aliases: &[], summary: "Keyboard catalogue; --surface tui|dashboard|popup|all, Markdown or --json", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs", aliases: &["docs"], summary: "Agent handbook, plain text or JSON", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "shows quality", aliases: &[], summary: "Set preferred show quality or default; never starts the daemon", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "setup", aliases: &[], summary: "Configure storage and desktop integration; --dry-run lists actions; --plugin link/copy; --uninstall keeps data", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "tui", aliases: &[], summary: "Interactive terminal UI (only useful on a TTY)", json: false, mutating: false, needs_backend: true },
    ];
    let mut cmds = command_tree(&cmds);
    if let Some(search)=cmds.iter_mut().find(|row|row["name"]=="search") {
        search["flags"]=serde_json::json!(["--install-torznab <URL>","--key-file <PATH>"]);
    }
    let mut exit_codes: Vec<serde_json::Value> = Code::ALL.iter().map(|c| serde_json::json!({
        "code": *c as i32, "name": c.name(), "meaning": c.describe()
    })).collect();
    exit_codes.extend([
        serde_json::json!({"code": 70, "name": "panic", "scope": "serve unit", "meaning": "daemon panic; outside the CLI 0..8 dictionary"}),
        serde_json::json!({"code": 75, "name": "upgrade", "scope": "serve unit", "meaning": "binary replaced while idle; outside the CLI 0..8 dictionary"}),
    ]);
    serde_json::json!({
        "ok": true,
        "tool": "fbittorrent",
        "version": env!("CARGO_PKG_VERSION"),
        "contract_version": CONTRACT_VERSION,
        "field_notes": {
            "list[].num_seeds": "connected seeds for live native torrents; read optional seeds to distinguish unknown",
            "list[].num_leechs": "total connected peers in native mode; not a count of leechers",
        },
        "caps": crate::api::ops::app::CAPS,
        "api_operations": crate::api::ops::TABLE,
        "changes": changes,
        "global_flags": ["--json", "--no-start"],
        "description": "Torrent app for Omarchy with an embedded Rust engine, CLI, TUI, bar widget and headless serve command.",
        "engines": { "default": "native", "native_lifetime": NATIVE_LIFETIME, "serve_limitations": SERVE_LIMITATIONS },
        "native_features": { "file_priorities": ["skip", "normal"], "sequential": "intrinsic", "first_last_piece": "intrinsic", "scheduling_toggles": false, "protocol_encryption": true, "peer_encryption_policies": ["off", "prefer", "require"], "download_queue": true, "interface_binding": "required exact Linux interface; proton selects proton0" },
        "backend_subcommands": ["rss grab", "shows grab"],
        "conventions": {
            "json": JSON_CONVENTION,
            "unit_exits": UNIT_EXITS,
            "stdout": "data only; diagnostics and progress go to stderr",
            "selectors": "torrents are selected by hash prefix (>= 4 hex chars) or a unique case-insensitive substring of the name; `all` selects every torrent",
            "bare_invocation": "on a TTY opens the TUI; when stdout is not a TTY prints `triage --json`",
            "destructive": "`delete --files` requires --yes; `delete --dry-run` previews any deletion",
            "native_connect": "write commands start the static user unit on demand unless --no-start, FBITTORRENT_NO_START=1 or autostart=false; up/dashboard override autostart=false. Reads, stop and delete-keeping-files never start it; stop and delete-keeping-files edit the ledger under engine.lock while the daemon is down. Without a user manager run fbittorrent serve.",
            "local_commands": "native search, RSS except grab, and Shows except grab operate without opening the torrent engine",
            "native_upload": upload_cap_text(),
        },
        "defaults": {
            "sequential": true, "first_last_piece": true, "upload": upload_cap_text(),
            "save_path": "the configured folder, or the desktop Downloads folder",
        },
        "commands": cmds,
        "exit_codes": exit_codes,
        "env": {
            "FBITTORRENT_UNIT": "systemd user unit name; default fbittorrent.service (setup and startup share it)",
            "FBITTORRENT_NO_START": "set to 1 to require an already open app, like --no-start; unavailable engine exits 4 with a hint",
            "XDG_CONFIG_HOME": "config at $XDG_CONFIG_HOME/fbittorrent/config.toml",
            "XDG_CACHE_HOME": "IMDb caches at $XDG_CACHE_HOME/fbittorrent",
            "NO_COLOR": "accepted; the CLI never emits colour anyway",
        },
        "files": {
            "config": crate::config::config_path().display().to_string(),
            "cache": crate::imdb::cache_dir().display().to_string(),
            "data": crate::config::data_dir().display().to_string(),
            "socket": crate::config::runtime_dir().join("api.sock").display().to_string(),
        },
        "agent_workflow": [
            "fbittorrent triage --json",
            "fbittorrent search \"<title>\" --json | jq '.results[0]'",
            "fbittorrent add \"<magnet>\" --json",
            "fbittorrent wait <hash> --timeout 3600 --json",
            "fbittorrent show <hash> --json | jq -r .open_target",
        ],
    })
}

// ---------------------------------------------------------------- robot docs

// Keep the existing printable guide interface while deriving the cap at display time.
pub struct RobotGuide;
pub const ROBOT_GUIDE: RobotGuide = RobotGuide;

impl fmt::Display for RobotGuide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&ROBOT_GUIDE_TEXT.replace("{upload_policy}", &upload_cap_text()).replace("{contract_version}", CONTRACT_VERSION)
            .replace("{native_lifetime}", NATIVE_LIFETIME)
            .replace("{serve_limitations}", SERVE_LIMITATIONS)
            .replace("{json_convention}", JSON_CONVENTION)
            .replace("{unit_exits}", UNIT_EXITS))
    }
}

const ROBOT_GUIDE_TEXT: &str = r#"fbittorrent — agent handbook (contract {contract_version})

WHAT IT IS
  A native Rust torrent app. {native_lifetime}
  {serve_limitations}
  Write commands start the systemd user unit without a terminal. Reads, stop and
  delete-keeping-files never start it (stop and delete-keeping-files edit the ledger under engine.lock while the daemon is down).
  Status and triage never open it. Search, RSS browsing, and Shows tracking work
  locally without starting downloads.
  Upload is {upload_policy}.
  Files land in the configured download folder or the desktop Downloads folder.
  The native daemon is the only backend.

RULES
  - {json_convention}
  - stdout is data. Progress and diagnostics are on stderr.
  - Select torrents by hash prefix (>= 4 hex chars) or a unique substring of
    the name. `all` means every torrent. Ambiguous -> exit 7 with the matches.
  - Destructive: `delete --files` needs --yes. `delete --dry-run` previews.
  - Bare `fbittorrent` opens the TUI on a TTY. Without a TTY it prints
    `triage --json`, so agents can always call it safely.
  - `up` starts only the daemon; `up --ui` also opens the dashboard or TUI.
    `dashboard --view <view> --query <query>` starts then summons the panel, with
    a TUI fallback. `down` shuts down the owner and waits for engine cleanup.
    `restart` requests daemon replacement and waits up to 30 s for a new hello.
    Without a systemd user manager, run `fbittorrent serve` in a terminal/tmux.
    `setup --magnet` writes the static unit and handlers; nothing starts at login.
    `setup --uninstall` removes the unit and keeps saved data.
  - `--no-start` (global) or FBITTORRENT_NO_START=1 prevents engine startup,
    including explicit `up` and `tui`. A closed engine returns exit 4 with a hint.
    autostart=false is the persistent form for ordinary torrent commands.
  - Streaming priority is intrinsic. File priority supports skip/normal. Peer encryption defaults to prefer; require refuses
    plaintext peers. F6 or `settings` changes encryption, queue limit and VPN.
  - Three downloads run at once by default. Extra downloads wait in FIFO order.
    Manual pause remains paused. Completed seeders do not consume queue slots.
  - `settings --vpn proton` requires ProtonVPN's proton0 tunnel. Missing/down
    tunnel blocks engine traffic and the app waits. Feed/search/title websites
    use the usual connection; external torrent download hooks are refused.
  - RSS read state is local. Shows read state is independent of download state;
    `episodes --new` lists unread releases including S01 season bundles.
  - Existing native ledger, RSS, Shows and settings files load directly.


EXIT CODES
  0 ok            1 failure       2 usage         3 safety (needs --yes)
  4 backend down  5 upstream err  6 not found     7 ambiguous     8 timeout
  {unit_exits}

DOWNLOAD SOMETHING (the canonical flow)
  fbittorrent triage --json                       # backend state, torrents, warnings
  fbittorrent search "Sintel 2010" --json         # {"results":[{fileName,fileUrl,seeders,imdb,...}]}
  fbittorrent add "<fileUrl or magnet>" --json    # {"ok":true,"hash":"...","name":"..."}
  fbittorrent wait <hash> --timeout 3600 --json   # blocks; exit 0 complete, 8 timeout
  fbittorrent show <hash> --json                  # files[], open_target (file or folder)
  fbittorrent open <hash>                         # hands it to the desktop

  Shortcut: `fbittorrent add "<magnet>" --wait --json` does add + wait.

OTHER USEFUL CALLS
  fbittorrent events --follow --kinds torrent.completed --json # NDJSON; store instance/seq
  fbittorrent api torrents.list --json            # unchanged API response envelope
  fbittorrent api --list --json / robot-docs api   # operation args/results and retry policy
  fbittorrent serve                              # foreground daemon; stop with SIGTERM
  fbittorrent list --json                         # every torrent
  fbittorrent stop|start|delete <sel> --json      # {"ok":true,"action":"stop","hashes":[...]}
  fbittorrent seed <sel> --json                   # allow upload for one torrent
  fbittorrent search --plugins --json             # providers and whether enabled
  fbittorrent rss list --json / rss items <feed> --json / rss grab <feed> <id> --json
  fbittorrent limit <sel> --down 64K --json           # live download cap; none clears
  fbittorrent shows follow "<title|tt id>" --json      # follow a TV show (local list, no service)
  fbittorrent shows check --json                       # fetch episodes from EZTV, list what is new
  fbittorrent shows grab <show> S01E02 --json          # add the best torrent for an episode
  fbittorrent imdb "<release name>" --json        # rating, blurb, poster path
  fbittorrent capabilities --json                 # this contract, machine readable
  fbittorrent down                                # close the app and stop downloads

JSON SHAPES (stable within contract 1.x)
  torrent: {hash,name,state,progress(0..1),progress_known,size,completed,dlspeed,upspeed,eta,
            num_seeds,num_leechs,ratio,seq_dl,f_l_piece_prio,save_path,
            content_path,category,tags,added_on,completion_on,up_limit,ratio_limit}
  search result: {fileName,fileUrl,fileSize,nbSeeders,nbLeechers,siteUrl,
                  descrLink,engineName,imdb,pubDate}
  states: downloading, metaDL, stalledDL, queuedDL, stoppedDL, uploading,
          stalledUP, stoppedUP (= done), checkingDL, waitingVPN, error, missingFiles
  When progress_known=false, do not interpret progress/completed as measured.
  `show.complete` is null while progress is unknown.
  Read native_state, reason, checked and origin for the normalized model.
  eta_s and seeds are absent when unknown; eta/num_seeds keep legacy sentinels.
  API contract: docs/IPC.md. Add agents should pass --origin agent.
  Native `num_seeds` counts connected seeds when `seeds` is present;
  `show` adds `peers[]` with connection direction, seed flags and sampled rates.
  `num_leechs` counts connected peers,
  not leechers. Seed counts describe connected seeds, never a tracker scrape.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_caps_are_supported_and_live_caps_are_catalogued() {
        let value = capabilities_json();
        let live = value["caps"].as_array().unwrap();
        let docs = include_str!("../docs/IPC.md");
        let section = docs.split("## Capability catalogue").nth(1).unwrap()
            .split("\n## ").next().unwrap();
        let mut catalogue: std::collections::BTreeSet<_> = section.lines()
            .filter(|line| line.starts_with('|'))
            .flat_map(|line| line.split('|').nth(1).unwrap().split('`').skip(1).step_by(2))
            .collect();
        // Operation addenda also declare capabilities (for example free_space).
        catalogue.extend(docs.lines().filter_map(|line| {
            line.strip_prefix("Capability `")?.split_once("` adds").map(|(cap, _)| cap)
        }));
        for cap in live {
            assert!(catalogue.contains(cap.as_str().unwrap()), "uncatalogued live capability: {cap}");
        }
        for fixture in [
            include_str!("../contracts/ipc/v1/hello.ndjson"),
            include_str!("../contracts/ipc/v1/errors.ndjson"),
            include_str!("../contracts/ipc/v1/torrents.ndjson"),
        ] {
            for line in fixture.lines() {
                let row: serde_json::Value = serde_json::from_str(line).unwrap();
                if let Some(caps) = row["frame"]["result"].get("caps") {
                    for cap in caps.as_array().unwrap() {
                        assert!(live.contains(cap), "unsupported fixture capability: {cap}");
                    }
                }
            }
        }
    }

    #[test]
    fn contract_version_is_present_and_well_formed() {
        let value = capabilities_json();
        let version = value["contract_version"].as_str().unwrap();
        let components: Vec<_> = version.split('.').collect();
        assert_eq!(components.len(), 3, "contract_version must be major.minor.patch");
        assert!(components.iter().all(|part| !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (*part == "0" || !part.starts_with('0'))));
        // changes[] is descriptive bookkeeping: entries may share a version.
        assert!(value["changes"].is_array());
    }

    #[test]
    fn native_seed_counts_preserve_unknown_and_measured_zero() {
        let capabilities = capabilities_json();
        for field in ["list[].num_seeds", "list[].num_leechs"] {
            assert!(!capabilities["field_notes"][field].as_str().unwrap().trim().is_empty());
        }
        let fixture = include_str!("../contracts/ipc/v1/torrents.ndjson");
        let mut torrent = fixture.lines().find_map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            serde_json::from_value::<crate::api::Torrent>(row["frame"]["result"]["torrent"].clone()).ok()
        }).expect("torrent fixture");
        for seeds in [None, Some(0), Some(2)] {
            torrent.seeds = seeds;
            torrent.peers = Some(3);
            let row = crate::cli::shapes::torrent(&torrent);
            assert_eq!(row.get("seeds"), seeds.map(serde_json::Value::from).as_ref());
            assert_eq!(row["num_seeds"], seeds.unwrap_or(0));
            assert_eq!(row["num_leechs"], 3);
        }
    }

    #[test]
    fn no_start_contract_is_advertised() {
        let capabilities = capabilities_json();
        assert_eq!(capabilities["contract_version"], CONTRACT_VERSION);
        assert!(capabilities["global_flags"].as_array().unwrap().contains(&serde_json::json!("--no-start")));
        assert!(capabilities["env"]["FBITTORRENT_NO_START"].is_string());
        assert!(ROBOT_GUIDE.to_string().contains(&format!("contract {CONTRACT_VERSION}")));
    }

    #[test]
    fn serve_documentation_matches_capabilities() {
        let capabilities = capabilities_json();
        let guide = ROBOT_GUIDE.to_string();
        let serve = capabilities["commands"].as_array().unwrap().iter()
            .find(|command| command["name"] == "serve").unwrap();
        assert_eq!(serve["json"], false);
        for text in [capabilities["engines"]["native_lifetime"].as_str().unwrap(),
                     capabilities["engines"]["serve_limitations"].as_str().unwrap(),
                     capabilities["conventions"]["json"].as_str().unwrap(),
                     capabilities["conventions"]["unit_exits"].as_str().unwrap()] {
            assert!(guide.contains(text), "robot-docs disagrees with capabilities: {text}");
        }
        assert!(guide.contains("fbittorrent serve"));
        for limitation in ["idle_exit_secs", "on demand", "API v1", "api.sock", "glance pauses"] {
            assert!(guide.contains(limitation));
        }
        for (code, name) in [(70, "panic"), (75, "upgrade")] {
            let entry = capabilities["exit_codes"].as_array().unwrap().iter()
                .find(|entry| entry["code"] == code).unwrap();
            assert_eq!(entry["scope"], "serve unit");
            assert_eq!(entry["name"], name);
            assert!(guide.contains(&format!("{code} {name}")));
        }
        assert!(guide.contains("outside the CLI 0..8 dictionary"));
        assert_eq!(Code::ALL.map(|code| code as i32), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(capabilities["contract_version"], CONTRACT_VERSION);
        println!("serve docs/capabilities agree on lifetime, JSON exception, limitations and unit exits 70/75; CLI dictionary and contract unchanged");
    }

    #[test]
    fn upload_cap_text_from_constant() {
        let capabilities = capabilities_json();
        for text in [capabilities["defaults"]["upload"].as_str().unwrap(), capabilities["conventions"]["native_upload"].as_str().unwrap()] {
            assert!(text.contains("zero piece payload"));
            assert!(text.contains("off by default"));
        }
        let guide = ROBOT_GUIDE.to_string();
        assert!(guide.contains("zero piece payload"));
        assert!(!guide.contains("{upload_policy}"));

    }
}

// ---------------------------------------------------------------- triage

/// The mega-command payload. Everything an agent wants before deciding what to do.
fn command_tree(metadata:&[CommandDoc])->Vec<serde_json::Value>{
    use clap::CommandFactory;
    fn walk(command:&clap::Command,prefix:&str,metadata:&[CommandDoc],out:&mut Vec<serde_json::Value>){
        for child in command.get_subcommands().filter(|c|c.get_name()!="help"){
            let name=if prefix.is_empty(){child.get_name().to_string()}else{format!("{prefix} {}",child.get_name())};
            let old=metadata.iter().find(|m|m.name==name);
            let starts=matches!(name.as_str(),"rename"|"limit"|"add"|"add-clipboard"|"start"|"seed"|"noseed"|"open"|"up"|"dashboard"|"tui"|"rss grab"|"shows grab");
            out.push(serde_json::json!({"name":name,"aliases":child.get_all_aliases().collect::<Vec<_>>(),"summary":child.get_about().map(ToString::to_string).unwrap_or_default(),"json":old.map(|m|m.json).unwrap_or(true),"mutating":old.map(|m|m.mutating).unwrap_or(starts),"needs_backend":old.map(|m|m.needs_backend).unwrap_or(starts),"starts":starts,"since":if name=="limit" {"1.2.27"} else if name=="limits" {"1.2.14"} else {"1.2.0"}}));
            walk(child,&name,metadata,out);
        }
    }
    let mut out=Vec::new();walk(&crate::Cli::command(),"",metadata,&mut out);
    // robot-docs uses positional sections; keys is a documented command row.
    out.push(serde_json::json!({"name":"robot-docs keys","aliases":[],"summary":"Keyboard catalogue","json":true,"mutating":false,"needs_backend":false,"starts":false,"since":"P4-02"}));
    out
}

#[cfg(test)] mod clap_contract_tests {
 use super::*;
 use clap::CommandFactory;
 #[test] fn commands_match_clap(){
  fn names(command:&clap::Command,prefix:&str,out:&mut Vec<String>){for c in command.get_subcommands().filter(|c|c.get_name()!="help"){
   let name=if prefix.is_empty(){c.get_name().to_owned()}else{format!("{prefix} {}",c.get_name())};out.push(name.clone());names(c,&name,out);
  }}
  let mut expected=Vec::new();names(&crate::Cli::command(),"",&mut expected);expected.push("robot-docs keys".into());expected.sort();
  let caps=capabilities_json();let mut actual=caps["commands"].as_array().unwrap().iter().map(|v|v["name"].as_str().unwrap().to_owned()).collect::<Vec<_>>();actual.sort();assert_eq!(actual,expected);
  for row in caps["commands"].as_array().unwrap(){assert!(row["starts"].is_boolean());assert!(row["since"].is_string());}
 }
}
