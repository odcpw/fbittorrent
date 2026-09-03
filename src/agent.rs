//! Agent-facing surface: exit-code dictionary, typed errors, JSON envelopes,
//! `capabilities`, `robot-docs`, and the `triage` mega-command.

use serde::Serialize;
use std::fmt;

pub const CONTRACT_VERSION: &str = "1.0.0";

/// Exit codes. Stable; documented by `capabilities --json` and `robot-docs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
    Ok = 0,
    /// Something unexpected (bug, I/O). Retrying rarely helps.
    Failure = 1,
    /// Bad arguments or input. Same code clap uses for usage errors.
    Usage = 2,
    /// Refused for safety; the hint names the flag that overrides.
    Safety = 3,
    /// qBittorrent is not installed, not running, or would not start.
    Backend = 4,
    /// qBittorrent or a network provider answered with an error.
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
            Code::Backend => "qBittorrent unavailable; run `fbittorrent up` or `fbittorrent setup`",
            Code::Upstream => "qBittorrent or a provider returned an error; retry later",
            Code::NotFound => "no torrent, feed or plugin matched; `fbittorrent list --json` shows what exists",
            Code::Ambiguous => "selector matched several items; use a longer hash prefix",
            Code::Timeout => "`wait` timed out; the torrent is still there, poll `show <hash> --json`",
        }
    }
    pub const ALL: [Code; 9] = [Code::Ok, Code::Failure, Code::Usage, Code::Safety, Code::Backend, Code::Upstream, Code::NotFound, Code::Ambiguous, Code::Timeout];
}

/// An error with an exit code and a copy-pasteable next step.
#[derive(Debug)]
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
    if low.contains("connecting to qbittorrent") || low.contains("not running") || low.contains("not installed") || low.contains("did not come up") {
        return (Code::Backend, msg, Some("fbittorrent up   # starts qbittorrent-nox; `fbittorrent setup` if it was never installed".into()));
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

pub fn capabilities_json() -> serde_json::Value {
    let cmds = [
        CommandDoc { name: "triage", aliases: &["robot-triage"], summary: "One-call overview: backend, rates, torrents, warnings, next commands", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "status", aliases: &[], summary: "Rates and counts; never starts the backend", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "list", aliases: &["ls"], summary: "All torrents with hash, progress, speed, flags", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "show", aliases: &["info"], summary: "One torrent in full, including its files and the path to open", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "search", aliases: &["find"], summary: "Search the enabled providers; results carry magnet links", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "add", aliases: &["download", "get"], summary: "Add a magnet or URL; returns the hash; --wait blocks until complete", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "wait", aliases: &[], summary: "Block until torrents complete (or --timeout); progress on stderr", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "start", aliases: &["resume"], summary: "Start torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "stop", aliases: &["pause"], summary: "Stop torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "delete", aliases: &["rm", "remove"], summary: "Remove torrents; --files deletes data and needs --yes; --dry-run previews", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "seed", aliases: &[], summary: "Allow upload for torrents", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "noseed", aliases: &[], summary: "Forbid upload (default for new torrents)", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "seq", aliases: &[], summary: "Toggle sequential download", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "first-last", aliases: &[], summary: "Toggle first/last piece priority", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "open", aliases: &[], summary: "Open the file or folder of a torrent in the desktop", json: true, mutating: false, needs_backend: true },
        CommandDoc { name: "reconcile", aliases: &[], summary: "Apply default flags to torrents added elsewhere", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "rss", aliases: &[], summary: "list | add | remove | refresh | items | grab", json: true, mutating: true, needs_backend: true },
        CommandDoc { name: "imdb", aliases: &[], summary: "Title, rating, blurb and poster for a release name", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "up", aliases: &[], summary: "Start qbittorrent-nox", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "down", aliases: &[], summary: "Stop qbittorrent-nox", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "config", aliases: &[], summary: "Effective configuration", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "capabilities", aliases: &[], summary: "This document", json: true, mutating: false, needs_backend: false },
        CommandDoc { name: "robot-docs", aliases: &["docs"], summary: "Agent handbook, plain text", json: false, mutating: false, needs_backend: false },
        CommandDoc { name: "setup", aliases: &[], summary: "First-run install of the backend service and magnet handler", json: true, mutating: true, needs_backend: false },
        CommandDoc { name: "tui", aliases: &[], summary: "Interactive terminal UI (only useful on a TTY)", json: false, mutating: false, needs_backend: true },
    ];
    let exit_codes: Vec<serde_json::Value> = Code::ALL.iter().map(|c| serde_json::json!({
        "code": *c as i32, "name": c.name(), "meaning": c.describe()
    })).collect();
    serde_json::json!({
        "ok": true,
        "tool": "fbittorrent",
        "version": env!("CARGO_PKG_VERSION"),
        "contract_version": CONTRACT_VERSION,
        "description": "qBittorrent front-end for Omarchy: CLI, TUI, bar widget. Downloads are sequential, first/last piece first, upload off by default.",
        "conventions": {
            "json": "pass --json (global flag) for a JSON object on stdout; errors become {ok:false,code,code_name,error,hint} on stdout with the same exit code",
            "stdout": "data only; diagnostics and progress go to stderr",
            "selectors": "torrents are selected by hash prefix (>= 4 hex chars) or a unique case-insensitive substring of the name; `all` selects every torrent",
            "bare_invocation": "on a TTY opens the TUI; when stdout is not a TTY prints `triage --json`",
            "destructive": "`delete --files` requires --yes; `delete --dry-run` previews any deletion",
        },
        "defaults": {
            "sequential": true, "first_last_piece": true, "upload": "off (32 KiB/s cap, stops when complete)",
            "save_path": "qBittorrent's default, ~/Downloads after setup",
        },
        "commands": cmds,
        "exit_codes": exit_codes,
        "env": {
            "XDG_CONFIG_HOME": "config at $XDG_CONFIG_HOME/fbittorrent/config.toml",
            "XDG_CACHE_HOME": "IMDb caches at $XDG_CACHE_HOME/fbittorrent",
            "NO_COLOR": "accepted; the CLI never emits colour anyway",
        },
        "files": {
            "config": crate::config::config_path().display().to_string(),
            "cache": crate::imdb::cache_dir().display().to_string(),
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

pub const ROBOT_GUIDE: &str = r#"fbittorrent — agent handbook (contract 1.0.0)

WHAT IT IS
  A qBittorrent front-end. qbittorrent-nox runs as a systemd user service and is
  started on demand by any command that needs it. Every add is sequential,
  first/last piece first, and upload is OFF (32 KiB/s cap, torrent stops when
  complete). Files land in ~/Downloads unless configured otherwise.

RULES
  - Pass --json (global) for machine output. Every command then prints ONE JSON
    object on stdout. Errors also come as JSON on stdout:
      {"ok":false,"code":6,"code_name":"not_found","error":"...","hint":"..."}
    and the process exits with `code`.
  - stdout is data. Progress and diagnostics are on stderr.
  - Select torrents by hash prefix (>= 4 hex chars) or a unique substring of
    the name. `all` means every torrent. Ambiguous -> exit 7 with the matches.
  - Destructive: `delete --files` needs --yes. `delete --dry-run` previews.
  - Bare `fbittorrent` opens the TUI on a TTY. Without a TTY it prints
    `triage --json`, so agents can always call it safely.

EXIT CODES
  0 ok            1 failure       2 usage         3 safety (needs --yes)
  4 backend down  5 upstream err  6 not found     7 ambiguous     8 timeout

DOWNLOAD SOMETHING (the canonical flow)
  fbittorrent triage --json                       # backend state, torrents, warnings
  fbittorrent search "Sintel 2010" --json         # {"results":[{fileName,fileUrl,seeders,imdb,...}]}
  fbittorrent add "<fileUrl or magnet>" --json    # {"ok":true,"hash":"...","name":"..."}
  fbittorrent wait <hash> --timeout 3600 --json   # blocks; exit 0 complete, 8 timeout
  fbittorrent show <hash> --json                  # files[], open_target (file or folder)
  fbittorrent open <hash>                         # hands it to the desktop

  Shortcut: `fbittorrent add "<magnet>" --wait --json` does add + wait.

OTHER USEFUL CALLS
  fbittorrent list --json                         # every torrent
  fbittorrent stop|start|delete <sel> --json      # {"ok":true,"action":"stop","hashes":[...]}
  fbittorrent seed <sel> --json                   # allow upload for one torrent
  fbittorrent search --plugins --json             # providers and whether enabled
  fbittorrent rss list --json / rss items <feed> --json / rss grab <feed> <id> --json
  fbittorrent imdb "<release name>" --json        # rating, blurb, poster path
  fbittorrent capabilities --json                 # this contract, machine readable
  fbittorrent down                                # stop the backend when done

JSON SHAPES (stable within contract 1.x)
  torrent: {hash,name,state,progress(0..1),size,completed,dlspeed,upspeed,eta,
            num_seeds,num_leechs,ratio,seq_dl,f_l_piece_prio,save_path,
            content_path,category,tags,added_on,completion_on,up_limit,ratio_limit}
  search result: {fileName,fileUrl,fileSize,nbSeeders,nbLeechers,siteUrl,
                  descrLink,engineName,imdb,pubDate}
  states: downloading, metaDL, stalledDL, queuedDL, stoppedDL, uploading,
          stalledUP, stoppedUP (= done), checkingDL, error, missingFiles
"#;

// ---------------------------------------------------------------- triage

/// The mega-command payload. Everything an agent wants before deciding what to do.
pub fn triage(cfg: &crate::config::Config, c: &crate::qbt::Client) -> serde_json::Value {
    let running = c.version().is_ok();
    let unit_active = crate::backend::is_active(&cfg.backend_unit);
    let mut warnings: Vec<String> = Vec::new();
    let mut next: Vec<String> = Vec::new();
    let mut torrents = serde_json::Value::Array(vec![]);
    let mut transfer = serde_json::Value::Null;
    let mut counts = serde_json::json!({"active": 0, "seeding": 0, "stopped": 0, "done": 0, "error": 0, "total": 0});

    if !std::path::Path::new("/usr/bin/qbittorrent-nox").exists() {
        warnings.push("qbittorrent-nox is not installed".into());
        next.push("omarchy pkg add qbittorrent-nox && fbittorrent setup --magnet".into());
    } else if !running {
        warnings.push(if unit_active { "backend unit is active but the WebUI does not answer yet".into() } else { "backend is stopped (starts on demand)".into() });
        next.push("fbittorrent up".into());
    } else {
        let _ = c.login();
        if let Ok(ts) = c.torrents() {
            let active = ts.iter().filter(|t| t.is_active()).count();
            let seeding = ts.iter().filter(|t| t.is_seeding()).count();
            let stopped = ts.iter().filter(|t| t.is_stopped() && t.progress < 1.0).count();
            let done = ts.iter().filter(|t| t.progress >= 1.0).count();
            let errors = ts.iter().filter(|t| t.state == "error" || t.state == "missingFiles").count();
            counts = serde_json::json!({"active": active, "seeding": seeding, "stopped": stopped, "done": done, "error": errors, "total": ts.len()});
            if errors > 0 { warnings.push(format!("{errors} torrent(s) in error state")); }
            if stopped > 0 { next.push("fbittorrent start <hash> --json   # resume a stopped download".into()); }
            torrents = serde_json::json!(ts.iter().map(|t| serde_json::json!({
                "hash": t.hash, "name": t.name, "state": t.state, "progress": t.progress,
                "size": t.size, "dlspeed": t.dlspeed, "eta": t.eta, "seed_allowed": t.seed_allowed(),
                "open_target": crate::qbt::open_target(t),
            })).collect::<Vec<_>>());
        }
        if let Ok(ti) = c.transfer_info() {
            transfer = serde_json::json!({"dl": ti.dl_info_speed, "up": ti.up_info_speed, "connection": ti.connection_status});
        }
        match c.search_plugins() {
            Ok(ps) if ps.is_empty() => { warnings.push("no search providers installed".into()); next.push("fbittorrent search --install".into()); }
            Ok(ps) if !ps.iter().any(|p| p.enabled) => { warnings.push("all search providers are disabled".into()); }
            _ => {}
        }
    }
    if crate::imdb::ratings_stale() { warnings.push("IMDb ratings dataset missing or older than a day (refreshed automatically on first lookup)".into()); }
    next.push("fbittorrent search \"<title>\" --json".into());
    next.push("fbittorrent add \"<magnet>\" --wait --json".into());

    serde_json::json!({
        "ok": true,
        "contract_version": CONTRACT_VERSION,
        "backend": { "running": running, "unit": cfg.backend_unit, "unit_active": unit_active, "url": cfg.url, "autostart": cfg.autostart },
        "defaults": { "sequential": cfg.sequential, "first_last": cfg.first_last, "seed_by_default": cfg.seed_by_default, "save_path": cfg.save_path },
        "transfer": transfer,
        "counts": counts,
        "torrents": torrents,
        "warnings": warnings,
        "next_commands": next,
        "docs": "fbittorrent robot-docs",
    })
}
