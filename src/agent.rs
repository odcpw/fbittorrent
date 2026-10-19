//! Agent-facing surface: exit-code dictionary, typed errors, JSON envelopes,
//! `capabilities`, `robot-docs`, and the `triage` mega-command.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const CONTRACT_VERSION: &str = "1.1.1";

const NATIVE_LIFETIME: &str = "The TUI owns its engine until quit; fbittorrent serve owns a headless engine until shutdown, independently of windows.";
const SERVE_LIMITATIONS: &str = "serve currently has no idle exit yet; systemd starts it on demand for writes. It uses the framed legacy socket (engine.sock), not API v1.";
const JSON_CONVENTION: &str = "Ordinary CLI commands with --json print one JSON object on stdout; errors use {ok:false,code,code_name,error,hint} and the same exit code. serve is a long-running owner with stderr diagnostics, not a JSON response command; --json only formats its startup errors.";
const UNIT_EXITS: &str = "serve unit-only exits: 70 panic; 75 upgrade (reserved until upgrade checks land). These are outside the CLI 0..8 dictionary.";

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
    if let Some(ae) = e.downcast_ref::<AppError>() {
        return (ae.code, ae.message.clone(), ae.hint.clone());
    }
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
        println!("{}", serde_json::to_string(&env).unwrap_or_default());
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
        CommandDoc { name: "search", aliases: &["find"], summary: "Search local providers; results carry magnet links; native mode does not open the app", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "add", aliases: &["download", "get"], summary: "Add a magnet or URL; returns the hash; --wait blocks until complete", json: true, mutating: true, needs_backend: true },
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
        CommandDoc { name: "restart", aliases: &[], summary: "Restart the running engine; daemon upgrade handling follows", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "down", aliases: &[], summary: "Close the app and wait for its engine to stop; legacy mode stops its service", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "config", aliases: &[], summary: "Effective configuration", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "settings", aliases: &[], summary: "Set simultaneous downloads, peer encryption and required VPN connection; --interfaces lists connections", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "capabilities", aliases: &[], summary: "This document", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs keys", aliases: &[], summary: "Keyboard catalogue; --surface tui|dashboard|popup|all, Markdown or --json", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs", aliases: &["docs"], summary: "Agent handbook, plain text", json: false, mutating: false, needs_backend: false },
        CommandDoc { name: "setup", aliases: &[], summary: "Configure the download folder; --magnet registers desktop handlers; service setup only in explicit legacy mode", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "tui", aliases: &[], summary: "Interactive terminal UI (only useful on a TTY)", json: false, mutating: false, needs_backend: true },
    ];
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
            "list[].num_seeds": "always 0 in native mode: unknown",
            "list[].num_leechs": "total connected peers in native mode; not a count of leechers",
        },
        "changes": [{
            "version": "1.1.1", "field": "list[].num_seeds", "breaking": false,
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
            "native_connect": "write commands start the static user unit on demand unless --no-start, FBITTORRENT_NO_START=1 or autostart=false; up/dashboard override autostart=false. Reads, stop and delete-keeping-files never start it; offline mutations arrive in P1-11. Without a user manager run fbittorrent serve.",
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
            "socket": crate::ipc::socket_path(&crate::config::Config::default()).display().to_string(),
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
  delete-keeping-files never start it (offline mutations arrive in P1-11).
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
    `restart` currently requests an engine-slot restart; P1-17 adds upgrades.
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
  fbittorrent serve                              # foreground daemon; stop with SIGTERM
  fbittorrent list --json                         # every torrent
  fbittorrent stop|start|delete <sel> --json      # {"ok":true,"action":"stop","hashes":[...]}
  fbittorrent seed <sel> --json                   # allow upload for one torrent
  fbittorrent search --plugins --json             # providers and whether enabled
  fbittorrent rss list --json / rss items <feed> --json / rss grab <feed> <id> --json
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
  Native `num_seeds` is always 0: unknown; `num_leechs` counts connected peers,
  not leechers. Seed counts describe connected seeds, never a tracker scrape.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_seed_sentinel_is_documented_as_unknown() {
        let capabilities = capabilities_json();
        assert_eq!(capabilities["contract_version"], "1.1.1");
        assert_eq!(capabilities["field_notes"]["list[].num_seeds"], "always 0 in native mode: unknown");
        let change = capabilities["changes"].as_array().unwrap().iter()
            .find(|change| change["field"] == "list[].num_seeds").unwrap();
        assert_eq!(change["version"], CONTRACT_VERSION);
        assert_eq!(change["breaking"], false);
        assert!(change["note"].as_str().unwrap().contains("unknown"));
        assert!(capabilities["field_notes"]["list[].num_leechs"].as_str().unwrap()
            .contains("total connected peers"));
        let guide = ROBOT_GUIDE.to_string();
        assert!(guide.contains(&format!("contract {CONTRACT_VERSION}")));
        assert!(guide.contains("Native `num_seeds` is always 0: unknown"));
        println!("{}", capabilities["field_notes"]);
    }

    #[test]
    fn no_start_contract_is_advertised() {
        let capabilities = capabilities_json();
        assert_eq!(capabilities["contract_version"], "1.1.1");
        assert!(capabilities["global_flags"].as_array().unwrap().contains(&serde_json::json!("--no-start")));
        assert!(capabilities["env"]["FBITTORRENT_NO_START"].is_string());
        assert!(ROBOT_GUIDE.to_string().contains("contract 1.1.1"));
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
        for limitation in ["no idle exit yet", "on demand", "framed legacy socket"] {
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
        assert_eq!(capabilities["contract_version"], "1.1.1");
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
