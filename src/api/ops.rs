//! Metadata shared with dispatch and the future `api --list` catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Operation {
    pub name: &'static str,
    pub mutating: bool,
    pub idempotent: bool,
    pub since: &'static str,
    pub args: &'static str,
    pub result: &'static str,
}
pub const TABLE: &[Operation] = &[
    Operation {
        name: "app.free_space",
        mutating: false,
        idempotent: false,
        since: "P6-11",
        args: "`path: str` (absolute local directory, ≤4096 bytes, no controls)",
        result: "`{free_bytes: bytes?, writable: bool}`",
    },
    Operation {
        name: "app.settings.get",
        mutating: false,
        idempotent: false,
        since: "P3-05",
        args: "none",
        result: "`Settings`",
    },
    Operation {
        name: "app.settings.set",
        mutating: true,
        idempotent: true,
        since: "P3-05",
        args: "any subset of writable `Settings` keys",
        result: "`{applied: [str], restart: bool}`",
    },
    Operation {
        name: "imdb.lookup",
        mutating: false,
        idempotent: false,
        since: "P3-04",
        args: "exactly one of `name: str` (1..1024 characters) or `id: str` (`tt` + 1..10 digits), `refresh: bool?`",
        result: "`{card: ImdbCard?}`",
    },
    Operation {
        name: "hello",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "See handshake",
        result: "HelloResult",
    },
    Operation {
        name: "app.info",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "none",
        result: "hello's `proto`, `server`, `engine`, `instance`, `engine_epoch`, `state`, `caps`, `limits`, `policy` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, agent: u64, tui: u64, bar: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08)",
    },
    Operation {
        name: "app.client",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "`interactive: bool?`, `glance: bool?` (either or both; a connection drop resets both)",
        result: "`{}`",
    },
    Operation {
        name: "app.shutdown",
        mutating: true,
        idempotent: false,
        since: "P1-06",
        args: "`reason: str?`",
        result: "`{state: \"pending\"}`, then `bye`",
    },
    Operation {
        name: "app.restart",
        mutating: true,
        idempotent: false,
        since: "P1-06",
        args: "`scope: enum(engine|daemon)`",
        result: "`{state: \"pending\"}`",
    },
    Operation {
        name: "app.network",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "none",
        result: "`{state: NetworkState, interfaces: [str]}`",
    },
    Operation {
        name: "transfer.info",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "none",
        result: "`{dl_bps: bps?, ul_bps: bps?, session_downloaded: bytes, session_uploaded: bytes}`",
    },
    Operation {
        name: "torrents.list",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "`states: [state]?`, `origin: [str]?`",
        result: "`{torrents: [Torrent]}`",
    },
    Operation {
        name: "torrents.get",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "`hash: hash`",
        result: "`{torrent: TorrentDetail}`",
    },
    Operation {
        name: "torrents.add",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`source: {magnet: str} | {url: str} | {path: str} | {restore: hash}` (exactly one), `stage: bool?`, `stopped: bool?`, `seed: bool?`, `save_path: str?`, `files: [u64]?`, `category: str?`, `origin: open(manual|search|rss|shows|clipboard|handler|agent)?`",
        result: "`{state: enum(pending|done), hash: hash?, name: str?, duplicate: bool?, deferred: bool?}`",
    },
    Operation {
        name: "torrents.start",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`hashes: [hash]` (1..256), `force: bool?`",
        result: "`{state: \"done\", started: [hash], skipped: [{hash, reason: open(complete_no_seed|already_running|staged)}]}`",
    },
    Operation {
        name: "torrents.stop",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`hashes: [hash]`",
        result: "`{state: \"done\", stopped: [hash]}`",
    },
    Operation {
        name: "torrents.remove",
        mutating: true,
        idempotent: false,
        since: "P1-06",
        args: "`hashes: [hash]`, `delete_files: bool` (required)",
        result: "`{state: enum(done|pending), undo_until: ts?}` (pending when deleting files; `undo_until` only when files are kept)",
    },
    Operation {
        name: "torrents.files",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "`hash: hash`",
        result: "`{files: [File]}`",
    },
    Operation {
        name: "torrents.set_files",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`hash: hash`, `include: [u64]?`, `skip: [u64]?`",
        result: "`{state: \"done\"}`",
    },
    Operation {
        name: "torrents.set_seed",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`hashes: [hash]`, `on: bool`",
        result: "`{state: \"done\"}`",
    },
    Operation {
        name: "torrents.set_upload_limit",
        mutating: true,
        idempotent: true,
        since: "P1-06",
        args: "`hashes: [hash]`, `bps: bps?` (≥ 16384, or `null` for unlimited)",
        result: "`{state: enum(done|pending)}` (pending until P6-07)",
    },
    Operation {
        name: "torrents.open_target",
        mutating: false,
        idempotent: false,
        since: "P1-06",
        args: "`hash: hash`, `file: u64?`",
        result: "`{path: str, kind: enum(file|folder)}`",
    },
    Operation {
        name: "subscribe",
        mutating: false,
        idempotent: false,
        since: "P1-08",
        args: "`topics: [str]` (1..16), `interval_ms: u64?` (250..10000; applies to every topic of the call and is ignored by the \"on change\" topics of A.7; when omitted, each topic uses its A.7 default), `since_seq: u64?` (only with the `events` topic, A.7)",
        result: "`{sub: u64, topics: [str]}`",
    },
    Operation {
        name: "unsubscribe",
        mutating: false,
        idempotent: false,
        since: "P1-08",
        args: "`sub: u64`",
        result: "`{}`",
    },
];

pub mod app;
pub mod imdb;
pub mod settings;
pub mod torrents;
pub mod transfer;

pub type ApiResult<T> = Result<T, crate::api::Error>;
pub fn error(code: crate::api::ErrorCode, message: &str, key: Option<&str>) -> crate::api::Error {
    use crate::api::ErrorCode::*;
    crate::api::Error {
        code,
        message: match code {
            ProtoUnsupported => "proto unsupported".into(),
            UnknownOp => "unknown op".into(),
            NotFound => "not found".into(),
            EngineUnavailable => "engine unavailable".into(),
            Busy => "busy".into(),
            Timeout => "timeout".into(),
            _ => message.chars().take(200).collect(),
        },
        retryable: matches!(code, EngineUnavailable | Busy | Upstream | Timeout),
        hint: match code {
            EngineUnavailable | Busy | Timeout => Some("retry in a moment".into()),
            ProtoUnsupported | UnknownOp | NotFound => Some("inspect fbittorrent log".into()),
            _ => None,
        },
        details: key.map(|key| serde_json::Map::from_iter([("key".into(), key.into())])),
    }
}

pub fn engine_error(value: anyhow::Error) -> crate::api::Error {
    use crate::api::ErrorCode::*;
    if let Some(state) = value.downcast_ref::<crate::engine::InvalidTorrentState>() {
        let mut failure = error(InvalidState, &state.to_string(), None);
        failure.details = Some(serde_json::Map::from_iter([
            ("hash".into(), state.hash.clone().into()),
            ("reason".into(), state.reason.into()),
        ]));
        return failure;
    }
    let text = format!("{value:#}");
    let code = if text.contains("not found") {
        NotFound
    } else if text.contains("already complete") {
        InvalidState
    } else if text.contains("engine_unavailable")
        || text.contains("engine is closing")
        || text.contains("connect ProtonVPN")
    {
        EngineUnavailable
    } else if text.contains("timed out") {
        Timeout
    } else if text.contains("metadata") || text.contains("no live") || text.contains("out of range")
    {
        InvalidState
    } else {
        Internal
    };
    let mut failure = error(code, &text.chars().take(80).collect::<String>(), None);
    if code == EngineUnavailable {
        let state = if text.contains("state=starting") {
            "starting"
        } else if text.contains("ProtonVPN") {
            "parked"
        } else {
            "stopping"
        };
        failure.details = Some(serde_json::Map::from_iter([
            ("state".into(), state.into()),
            ("message".into(), format!("engine is {state}").into()),
        ]));
    }
    if text.contains("already complete") {
        failure.hint = Some("enable seeding for this torrent, then retry".into());
    }
    failure
}
