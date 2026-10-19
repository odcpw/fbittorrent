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
pub const CONTRACT_VERSION: &str = "1.2.30";

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
    if low.contains("connecting to qbittorrent") || low.contains("fbittorrent is closed") || low.contains("opening the fbittorrent app") || low.contains("not running") || low.contains("not installed") || low.contains("did not come up") {
        return (Code::Backend, msg, Some("fbittorrent up   # opens the app; `fbittorrent setup` configures its download folder".into()));
    }
    if low.contains("returned http") || low.contains("qbittorrent ") || low.contains("http ") || low.contains("timed out") || low.contains("dns") || low.contains("connection") {
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
    format!(
        "{} KiB/s while downloading (non-zero caps must be at least 16 KiB/s); stops when complete unless seeding is enabled",
        crate::qbt::NO_UPLOAD_LIMIT / 1024,
    )
}

pub fn capabilities_json() -> serde_json::Value {
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
        CommandDoc { name: "seq", aliases: &[], summary: "Toggle sequential download in legacy mode; native scheduling is intrinsic", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "first-last", aliases: &[], summary: "Toggle first/last priority in legacy mode; native scheduling is intrinsic", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "open", aliases: &[], summary: "Open the file or folder of a torrent in the desktop", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "reconcile", aliases: &[], summary: "Native scheduling stub (no connection); legacy mode applies default flags", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "rss", aliases: &[], summary: "Local list | add | remove | refresh | items; only grab needs the native engine", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "shows", aliases: &[], summary: "Followed TV shows (local list, checked against EZTV): list | follow | unfollow | check | episodes | grab | seen", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "imdb", aliases: &[], summary: "Title, rating, blurb and poster for a release name", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "up", aliases: &[], summary: "Start the daemon; --ui also opens the dashboard or TUI", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "dashboard", aliases: &[], summary: "Start the daemon and summon the dashboard, falling back to the TUI", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "restart", aliases: &[], summary: "Restart the daemon now; wait up to 30 seconds for its replacement hello", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "down", aliases: &[], summary: "Close the app and wait for its engine to stop; legacy mode stops its service", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "config", aliases: &[], summary: "Effective configuration", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "settings", aliases: &[], summary: "Set simultaneous downloads, peer encryption and required VPN connection; --interfaces lists connections", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "capabilities", aliases: &[], summary: "This document", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs keys", aliases: &[], summary: "Keyboard catalogue; --surface tui|dashboard|popup|all, Markdown or --json", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs", aliases: &["docs"], summary: "Agent handbook, plain text or JSON", json: true, mutating: false, needs_backend: false },
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
        "changes": [{"since":"1.2.30","version":"1.2.30","breaking":false,"field":"trackers.edit","cap":"tracker_edit","note":"P7-04 live idempotent tracker add/remove with durable edits and bounded URL validation"},{"since":"1.2.29","version":"1.2.29","breaking":false,"field":"search.torznab","cap":"torznab","note":"native Torznab providers; strict private indexers/key files, HTTPS for keyed remote endpoints, bounded same-origin requests and secret-free results"},{"since":"1.2.28","version":"1.2.28","breaking":false,"field":"search.install_torznab","note":"CLI search --install-torznab/--key-file and Torznab provider kind; credentials stay in private files"},{"since":"1.2.27","version":"1.2.27","breaking":false,"field":"download_limit_bps","cap":"download_limit","note":"P7-06 durable live per-torrent download caps, effective cap field and limit --down CLI without rechecking"},{"since":"1.2.26","version":"1.2.26","breaking":false,"field":"list[].num_seeds","cap":"peers","note":"P5-01 fills connected seed counts for live torrents; show adds peers[] with one-second rates and optional seeds"},{"since":"1.2.25","version":"1.2.25","breaking":false,"field":"playable","cap":"watch","note":"verified video prefix readiness, once-per-lifetime playback notification and wait --until playable|stall-free"},{"since":"1.2.24","version":"1.2.24","breaking":false,"field":"search.results[].providers","note":"CLI search uses daemon or local normalized items with identical fields and deterministic provider provenance"},{"since":"1.2.23","version":"1.2.23","breaking":false,"field":"app.client.dashboard_visible","cap":"dashboard_presence","note":"M1a visible dashboard keeps its shared bar connection interactive; hidden dashboard and bar-only connections do not"},{"since":"1.2.22","version":"1.2.22","breaking":false,"field":"api.staging","note":"stage metainfo without payload access; commit choices or discard without deleting data","cap":"staging"},{"since":"1.2.21","version":"1.2.21","breaking":false,"field":"api.move","note":"pending/ref payload moves preserve verification, save path and CLI act bookkeeping","cap":"move"},{"since":"1.2.20","version":"1.2.20","breaking":false,"field":"shows","cap":"shows","note":"P3-03 daemon Shows inbox, stale interactive checks, read/undo/grab intents and pushed topics"},{"since":"1.2.19","version":"1.2.19","breaking":false,"field":"rss","cap":"rss","note":"P3-02 daemon RSS operations, feed/item topics, interactive stale refresh and bounded feeds"},{"since":"1.2.18","version":"1.2.18","breaking":false,"field":"clipboard","cap":"clipboard","note":"P6-13 opt-in bounded NUL-framed clipboard magnet offers, dismiss operation, notification actions and settings on/off"},{"since":"1.2.17","version":"1.2.17","breaking":false,"field":"add.disk","note":"P5-07 known disk capacity/warnings; summary disk monitoring and disk_full recovery on a 16 MiB/piece free-space rise"},{"since":"1.2.16","version":"1.2.16","breaking":false,"field":"app.free_space","cap":"free_space","note":"P6-11: read-only destination space and writability; native add --save refuses an unwritable folder with exit 2"},{"since":"1.2.15","version":"1.2.15","breaking":false,"field":"caps.imdb","cap":"imdb","note":"P3-04 implemented IMDb capability recorded in exact hello/error fixtures"},{"since":"1.2.14","version":"1.2.14","breaking":false,"field":"limits","cap":"limits","note":"live global/turtle limits, persisted alt switch, structural summary fields and offline CLI edits"},{"since":"1.2.13","version":"1.2.13","breaking":false,"field":"add.confirmation_required","note":"handler confirm opens the dashboard review before downloading; dashboard accepts --hash for staged records"},{"since":"1.2.12","version":"1.2.12","breaking":false,"field":"trackers","cap":"trackers","note":"tracker diagnostics and rate-limited reannounce; show adds trackers and tracker_count"},{"since":"1.2.11","version":"1.2.11","breaking":false,"field":"setup.actions","note":"planned native desktop integration; dry-run, plugin link/copy, uninstall and confirmed purge"},{"since":"1.2.10","version":"1.2.10","breaking":false,"field":"api.search","cap":"search","note":"daemon search sessions, keyed deduplicated results, providers, browse and grab"},{"since":"1.2.9","version":"1.2.9","breaking":false,"field":"api","note":"unchanged raw response envelope; compiled operation catalogue and per-operation startup/retry policy"},{"since":"1.2.8","version":"1.2.8","breaking":false,"field":"events","note":"NDJSON history/follow with instance and seq, replay cursors and resync metadata"},{"since":"1.2.7","version":"1.2.7","breaking":false,"field":"restart.server","note":"restart drains the daemon and waits for its replacement instance; binary replacement is deferred while busy"},{"since":"1.2.6","version":"1.2.6","breaking":false,"field":"log","cap":"log","note":"app.log, log topic and CLI history/follow; app.stats distinguishes active clients and cumulative connections"},{"since":"1.2.5","version":"1.2.5","breaking":false,"field":"api.origin","cap":"origin","note":"durable open origin, manual-bucket filters, keep-files batch undo_until and restore source"},{"since":"1.2.4","version":"1.2.4","breaking":false,"field":"api.queue","cap":"queue","note":"queue capability, force-start, queue movement and positioned torrent rows"},{"since":"1.2.3","version":"1.2.3","breaking":false,"field":"api.torrents.recheck","cap":"recheck","note":"recheck capability, pending ref acknowledgement and measured verification result"},{"since":"1.2.2","version":"1.2.2","breaking":false,"field":"settings","cap":"settings","note":"durable app.settings.get/set; CLI adds applied, restart, running and canonical settings keys"},{"since":"1.2.1","version":"1.2.1","breaking":false,"field":"add.deferred","cap":"deferred_add","note":"parked magnet/local metainfo adds return deferred:true and retain intent until VPN returns"},{"since":"1.2.0","breaking":true,"field":"status.ok","note":"stopped status is successful with running:false"},{"since":"1.2.0","breaking":false,"field":"list[].eta","note":"legacy sentinel remains 8640000; read eta_s for known values"},{"since":"1.2.0","breaking":false,"field":"list[].native_state","note":"normalized state, checked, reason and origin are additive"},{"since":"1.2.0","breaking":false,"field":"add.duplicate","note":"atomic API result; add --origin manual|handler|agent"},{
            "since":"1.2.0", "version": "1.2.0", "field": "list[].num_seeds", "breaking": false,
            "note": "always 0 in native mode: unknown; list[].num_leechs is the connected peer count",
        }],
        "global_flags": ["--json", "--no-start"],
        "description": "Torrent app for Omarchy with an embedded Rust engine, CLI, TUI, bar widget and headless serve command.",
        "engines": { "default": "native", "legacy": "qbittorrent", "native_lifetime": NATIVE_LIFETIME, "serve_limitations": SERVE_LIMITATIONS, "legacy_selection": "set engine = \"qbittorrent\" in config explicitly" },
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
            "socket": crate::ipc::socket_path(&crate::config::Config::default()).with_file_name("api.sock").display().to_string(),
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
  Upload is capped at {upload_policy}.
  Files land in the configured download folder or the desktop Downloads folder.
  Explicit engine = "qbittorrent" retains the legacy WebUI/service path.

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
    Native `reconcile` is a no-op and never connects to the engine.
  - Native sequential and first/last scheduling are intrinsic. Toggle commands
    and --no-seq/--no-first-last require legacy mode; native file priority only
    supports skip/normal. Peer encryption defaults to prefer; require refuses
    plaintext peers. F6 or `settings` changes encryption, queue limit and VPN.
  - Three downloads run at once by default. Extra downloads wait in FIFO order.
    Manual pause remains paused. Completed seeders do not consume queue slots.
  - `settings --vpn proton` requires ProtonVPN's proton0 tunnel. Missing/down
    tunnel blocks engine traffic and the app waits. Feed/search/title websites
    use the usual connection; external torrent download hooks are refused.
  - RSS read state is local. Shows read state is independent of download state;
    `episodes --new` lists unread releases including S01 season bundles.
  - First launch imports available local legacy torrents paused, plus RSS and
    provider preferences. An installed stopped legacy service may run once to
    export saved state. Its login startup is disabled and the service stops only
    after successful import; legacy config and backup files remain for rollback.
    Migration errors preserve the source and keep native imported files paused.

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
    fn exact_hello_caps_have_additive_change_entries() {
        let value = capabilities_json();
        let base = ["torrents", "files", "subscribe", "mutations"];
        for cap in crate::api::ops::app::CAPS.iter().filter(|cap| !base.contains(cap)) {
            assert!(value["changes"].as_array().unwrap().iter().any(|row| row["cap"] == *cap), "missing changes entry for {cap}");
        }
        for fixture in [
            include_str!("../contracts/ipc/v1/hello.ndjson"),
            include_str!("../contracts/ipc/v1/errors.ndjson"),
            include_str!("../contracts/ipc/v1/torrents.ndjson"),
        ] {
            for line in fixture.lines() {
                let row: serde_json::Value = serde_json::from_str(line).unwrap();
                if let Some(caps) = row["frame"]["result"].get("caps") {
                    assert_eq!(*caps, value["caps"]);
                }
            }
        }
    }

    #[test]
    fn additive_contract_versions_are_unique_and_documented() {
        let value = capabilities_json();
        let docs = include_str!("../docs/IPC.md");
        let mut patches = std::collections::BTreeMap::new();
        let changes = value["changes"].as_array().unwrap();
        let mut previous = u32::MAX;
        for row in changes.iter().filter(|row| row["since"] != "1.2.0") {
            let version = row["since"].as_str().unwrap();
            let patch: u32 = version.strip_prefix("1.2.").unwrap().parse().unwrap();
            let field = row["field"].as_str().unwrap();
            assert_eq!(row["version"], version, "since/version mismatch for {field}");
            assert_eq!(row["breaking"], false, "patch must be additive: {field}");
            assert!(patch > 0 && patch < previous, "changes must be newest first: {field}");
            previous = patch;
            assert!(patches.insert(patch, field).is_none(), "duplicate contract patch {version}");
            assert!(docs.lines().any(|line| line == format!("| `{version}` | `{field}` |")),
                "undocumented contract patch {version}: {field}");
        }
        let latest = *patches.keys().next_back().unwrap();
        assert_eq!(patches.keys().copied().collect::<Vec<_>>(), (1..=latest).collect::<Vec<_>>());
        assert_eq!(CONTRACT_VERSION, format!("1.2.{latest}"));
        let documented = docs.lines().filter(|line| line.starts_with("| `1.2.")).count();
        assert_eq!(documented, patches.len(), "stale or duplicate documented contract patches");
        let limits = value["commands"].as_array().unwrap().iter()
            .find(|row| row["name"] == "limits").unwrap();
        let limits_change = changes.iter().find(|row| row["field"] == "limits").unwrap();
        assert_eq!(limits["since"], limits_change["since"]);
    }

    #[test]
    fn native_seed_counts_preserve_unknown_and_historical_contract() {
        let capabilities = capabilities_json();
        assert_eq!(capabilities["contract_version"], CONTRACT_VERSION);
        assert!(capabilities["field_notes"]["list[].num_seeds"].as_str().unwrap().contains("connected seeds"));
        let change = capabilities["changes"].as_array().unwrap().iter()
            .find(|change| change["field"] == "list[].num_seeds" && change["version"]=="1.2.0").unwrap();
        assert!(capabilities["changes"].as_array().unwrap().iter().any(|c|c["cap"]=="peers" && c["version"]=="1.2.26"));
        assert_eq!(change["version"], "1.2.0");
        assert_eq!(change["breaking"], false);
        assert!(change["note"].as_str().unwrap().contains("unknown"));
        assert!(capabilities["field_notes"]["list[].num_leechs"].as_str().unwrap()
            .contains("total connected peers"));
        let guide = ROBOT_GUIDE.to_string();
        assert!(guide.contains(&format!("contract {CONTRACT_VERSION}")));
        assert!(guide.contains("Native `num_seeds` counts connected seeds"));
        println!("{}", capabilities["field_notes"]);
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
        let cap = crate::qbt::NO_UPLOAD_LIMIT;
        let expected = format!(
            "{} KiB/s while downloading (non-zero caps must be at least 16 KiB/s); stops when complete unless seeding is enabled",
            cap / 1024,
        );
        println!("NO_UPLOAD_LIMIT={cap} B/s; derived upload text: {expected}");
        let capabilities = capabilities_json();
        assert_eq!(capabilities["defaults"]["upload"], expected);
        assert_eq!(capabilities["conventions"]["native_upload"], expected);
        let guide = ROBOT_GUIDE.to_string();
        assert!(guide.contains(&format!("Upload is capped at {expected}.")));
        assert!(!guide.contains("{upload_policy}"));
        assert!(!guide.contains("1 B/s"));
    }
}

// ---------------------------------------------------------------- triage

/// The mega-command payload. Everything an agent wants before deciding what to do.
pub fn triage(cfg: &crate::config::Config, c: &crate::qbt::Client) -> serde_json::Value {
    let running = c.version().is_ok();
    let unit_active = !cfg.native() && crate::backend::is_active(&cfg.backend_unit);
    let mut warnings: Vec<String> = Vec::new();
    let mut next: Vec<String> = Vec::new();
    let mut torrents = serde_json::Value::Array(vec![]);
    let mut transfer = serde_json::Value::Null;
    let mut counts = serde_json::json!({"available": false, "active": 0, "queued": 0, "seeding": 0, "stopped": 0, "done": 0, "error": 0, "total": 0});

    if cfg.native() && !running {
        warnings.push("fbittorrent is closed; the native engine is stopped".into());
        next.push("fbittorrent up".into());
    } else if !cfg.native() && !std::path::Path::new("/usr/bin/qbittorrent-nox").exists() {
        warnings.push("qbittorrent-nox is not installed".into());
        next.push("omarchy pkg add qbittorrent-nox && fbittorrent setup --magnet".into());
    } else if !running {
        warnings.push(if unit_active { "backend unit is active but the WebUI does not answer yet".into() } else { "backend is stopped (starts on demand)".into() });
        next.push("fbittorrent up".into());
    } else {
        let _ = c.login();
        if let Ok(ts) = c.torrents() {
            let active = ts.iter().filter(|t| t.is_active()).count();
            let queued = ts.iter().filter(|t| t.is_queued()).count();
            let seeding = ts.iter().filter(|t| t.is_seeding()).count();
            let stopped = ts.iter().filter(|t| t.is_stopped() && t.progress < 1.0).count();
            let done = ts.iter().filter(|t| t.progress >= 1.0).count();
            let errors = ts.iter().filter(|t| t.state == "error" || t.state == "missingFiles").count();
            counts = serde_json::json!({"available": true, "active": active, "queued": queued, "seeding": seeding, "stopped": stopped, "done": done, "error": errors, "total": ts.len()});
            if errors > 0 { warnings.push(format!("{errors} torrent(s) in error state")); }
            if stopped > 0 { next.push("fbittorrent start <hash> --json   # resume a stopped download".into()); }
            torrents = serde_json::json!(ts.iter().map(|t| serde_json::json!({
                "hash": t.hash, "name": t.name, "state": t.state, "progress": t.progress, "progress_known": t.progress_known,
                "size": t.size, "dlspeed": t.dlspeed, "eta": t.eta, "seed_allowed": t.seed_allowed(),
                "open_target": crate::qbt::open_target(t),
            })).collect::<Vec<_>>());
        }
        if let Ok(ti) = c.transfer_info() {
            transfer = serde_json::json!({"dl": ti.dl_info_speed, "up": ti.up_info_speed, "connection": ti.connection_status});
        }
        if cfg.native() {
            let network = crate::network_app::state(cfg);
            if !network.ready { warnings.push(network.message); }
        }
    }
    if cfg.native() || running {
        match c.search_plugins() {
            Ok(ps) if ps.is_empty() => { warnings.push("no search providers installed".into()); next.push("fbittorrent search --install".into()); }
            Ok(ps) if !ps.iter().any(|p| p.enabled) => { warnings.push("all search providers are disabled".into()); }
            _ => {}
        }
    }
    let rss_json = if cfg.native() || running {
        match c.rss_feeds() {
            Ok(feeds) => {
                for feed in &feeds {
                    if feed.has_error { warnings.push(format!("RSS feed {} has a refresh error; cached articles are retained", feed.name)); }
                }
                serde_json::json!({"feeds": feeds.len(), "unread": feeds.iter().flat_map(|feed| &feed.articles).filter(|article| !article.is_read).count()})
            }
            Err(error) => { warnings.push(format!("RSS store unreadable: {error}")); serde_json::Value::Null }
        }
    } else { serde_json::Value::Null };
    let shows_json = match crate::shows::Shows::load() {
        Ok(all) => {
            let new = all.new_total();
            if new > 0 { next.push("fbittorrent shows check --json   # new episodes of followed shows".into()); }
            serde_json::json!({"followed": all.shows.len(), "new": new, "stale": all.shows.iter().filter(|s| s.is_stale(crate::shows::now())).count()})
        }
        Err(e) => { warnings.push(format!("shows list unreadable: {e}")); serde_json::Value::Null }
    };
    if crate::imdb::ratings_stale() { warnings.push("IMDb ratings dataset missing or older than a day (refreshed automatically on first lookup)".into()); }
    next.push("fbittorrent search \"<title>\" --json".into());
    next.push("fbittorrent add \"<magnet>\" --wait --json".into());

    serde_json::json!({
        "ok": true,
        "contract_version": CONTRACT_VERSION,
        "backend": { "engine": cfg.engine, "running": running, "app_open": cfg.native() && running, "unit": if cfg.native() { None } else { Some(&cfg.backend_unit) }, "unit_active": unit_active, "url": if cfg.native() { None } else { Some(&cfg.url) }, "socket": if cfg.native() { Some(crate::ipc::socket_path(cfg)) } else { None }, "autostart": cfg.autostart },
        "defaults": { "sequential": cfg.native() || cfg.sequential, "first_last": cfg.native() || cfg.first_last, "seed_by_default": cfg.seed_by_default, "save_path": cfg.save_path, "max_active_downloads": cfg.max_active_downloads, "peer_encryption": cfg.peer_encryption, "network_interface": cfg.network_interface },
        "transfer": transfer,
        "counts": counts,
        "torrents": torrents,
        "shows": shows_json,
        "rss": rss_json,
        "warnings": warnings,
        "next_commands": next,
        "docs": "fbittorrent robot-docs",
    })
}

fn command_tree(metadata:&[CommandDoc])->Vec<serde_json::Value>{
    use clap::CommandFactory;
    fn walk(command:&clap::Command,prefix:&str,metadata:&[CommandDoc],out:&mut Vec<serde_json::Value>){
        for child in command.get_subcommands().filter(|c|c.get_name()!="help"){
            let name=if prefix.is_empty(){child.get_name().to_string()}else{format!("{prefix} {}",child.get_name())};
            let old=metadata.iter().find(|m|m.name==name);
            let starts=matches!(name.as_str(),"limit"|"add"|"add-clipboard"|"start"|"seed"|"noseed"|"open"|"up"|"dashboard"|"tui"|"rss grab"|"shows grab");
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
