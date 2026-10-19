//! Operation catalogue shared by dispatch, retry/start policy and raw CLI documentation.
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
    Operation { name: "torrents.trackers", mutating: false, idempotent: false, since: "P6-01",
        args: "`hash: hash`", result: "`{trackers: [Tracker]}`" },
    Operation { name: "torrents.reannounce", mutating: true, idempotent: true, since: "P6-01",
        args: "`hashes: [hash]` (1..256)", result: "`{state: \"done\"}`" },
    Operation { name: "search.start", mutating: true, idempotent: false, since: "P3-01",
        args: "`query`, `providers?`, `limit?`", result: "`{search_id}`" },
    Operation { name: "search.stop", mutating: true, idempotent: false, since: "P3-01",
        args: "`search_id`", result: "`{state: \"done\"}`" },
    Operation { name: "search.providers", mutating: false, idempotent: false, since: "P3-01",
        args: "none", result: "`{providers: [Provider]}`" },
    Operation { name: "search.set_provider", mutating: true, idempotent: true, since: "P3-01",
        args: "`name`, `enabled`", result: "`{state: \"done\"}`" },
    Operation { name: "search.install", mutating: true, idempotent: false, since: "P3-01",
        args: "`curated?` or `sources?`", result: "pending ref, then `{installed, failed}`" },
    Operation { name: "search.browse_sources", mutating: false, idempotent: false, since: "P3-01",
        args: "none", result: "`{sources: [BrowseSource]}`" },
    Operation { name: "search.browse", mutating: false, idempotent: false, since: "P3-01",
        args: "`source`", result: "`{items: [SearchResult]}`" },
    Operation { name: "search.grab", mutating: true, idempotent: false, since: "P3-01",
        args: "`search_id` and `key`, or `result`; add options", result: "as `torrents.add`" },
    Operation { name: "torrents.force_start", mutating: true, idempotent: true, since: "P6-05",
        args: "`hashes: [hash]` (1..256), `on: bool`", result: "`{state: \"done\"}`" },
    Operation { name: "torrents.queue_move", mutating: true, idempotent: false, since: "P6-05",
        args: "`hash: hash`, `to: enum(top|up|down|bottom)`", result: "`{state: \"done\", queue_pos: u64}`" },
    Operation { name: "torrents.recheck", mutating: true, idempotent: false, since: "P5-03",
        args: "`hashes: [hash]` (1..256)", result: "`{state: \"pending\", ref}`; then `mutation.result` with `{rechecked: [hash], failed: [{hash, message}], have_bytes: {hash: bytes}}`" },
    Operation { name:"shows.list",mutating:false,idempotent:false,since:"P3-03",args:"none",result:"`{shows: [Show]}`" },
    Operation { name:"shows.follow",mutating:true,idempotent:false,since:"P3-03",args:"`query?: str`, `imdb?: str`, `quality?: str`",result:"`{show: Show}`" },
    Operation { name:"shows.unfollow",mutating:true,idempotent:false,since:"P3-03",args:"`imdb: str`",result:"`{}`" },
    Operation { name:"shows.check",mutating:true,idempotent:false,since:"P3-03",args:"`imdb?: str`",result:"`{state: \"pending\", ref: str}`, then `shows.checked` and `mutation.result {new}`" },
    Operation { name:"shows.releases",mutating:false,idempotent:false,since:"P3-03",args:"`imdb?: str`, `inbox?: bool`, `unread_only?: bool`, `quality?: str`",result:"`{releases: [Release]}`" },
    Operation { name:"shows.set_read",mutating:true,idempotent:true,since:"P3-03",args:"`imdb: str`, `key: str`, `read: bool`",result:"`{}`" },
    Operation { name:"shows.mark_show_read",mutating:true,idempotent:true,since:"P3-03",args:"`imdb: str`",result:"`{count: u64}`" },
    Operation { name:"shows.undo_read",mutating:true,idempotent:false,since:"P3-03",args:"none",result:"`{restored?: {imdb: str, key: str}}`" },
    Operation { name:"shows.grab",mutating:true,idempotent:false,since:"P3-03",args:"`imdb: str`, `key: str`, optional add options",result:"`{state: enum(pending|done), hash?: hash, name?: str, duplicate?: bool, deferred?: bool, disk?: DiskSpace}`" },
    Operation { name:"rss.feeds",mutating:false,idempotent:false,since:"P3-02",args:"none",result:"`{feeds: [Feed]}`" },
    Operation { name:"rss.add",mutating:true,idempotent:false,since:"P3-02",args:"`url: str`, `name?: str`",result:"`{feed: Feed}`" },
    Operation { name:"rss.remove",mutating:true,idempotent:false,since:"P3-02",args:"`feed: str`",result:"`{}`" },
    Operation { name:"rss.refresh",mutating:true,idempotent:false,since:"P3-02",args:"`feed?: str`",result:"`{state: enum(pending|done), ref?: str}`" },
    Operation { name:"rss.items",mutating:false,idempotent:false,since:"P3-02",args:"`feed: str`, `unread_only?: bool`",result:"`{items: [FeedItem]}`" },
    Operation { name:"rss.set_read",mutating:true,idempotent:true,since:"P3-02",args:"`feed: str`, `item: str`, `read: bool`",result:"`{}`" },
    Operation { name:"rss.mark_feed_read",mutating:true,idempotent:true,since:"P3-02",args:"`feed: str`",result:"`{count: u64}`" },
    Operation { name:"rss.grab",mutating:true,idempotent:false,since:"P3-02",args:"`feed: str`, `item: str`, optional add options",result:"`{state: enum(pending|done), hash?: hash, name?: str, duplicate?: bool, deferred?: bool, disk?: DiskSpace}`" },
    Operation { name: "app.clipboard.dismiss", mutating: true, idempotent: true, since: "P6-13",
        args: "`hash: hash`", result: "`{}`" },
    Operation { name: "app.log", mutating: false, idempotent: false, since: "P1-14",
        args: "`since_seq?: u64`, `limit?: u64` ≤1000 (default 200), `level?: enum(error|warn|info|debug)`, `source?: str`, `hash?: hash`", result: "`{entries: [LogEntry], next_seq: u64}`" },
    Operation { name: "app.stats", mutating: false, idempotent: false, since: "P1-14",
        args: "none", result: "`{active_clients: ClientCounts, connections: ClientCounts}`; current connections and connections since start" },
    Operation { name: "app.settings.get", mutating: false, idempotent: false, since: "P3-05",
        args: "none", result: "`Settings`" },
    Operation { name: "app.settings.set", mutating: true, idempotent: true, since: "P3-05",
        args: "any subset of writable `Settings` keys", result: "`{applied: [str], restart: bool}`" },
    Operation { name: "imdb.lookup", mutating: false, idempotent: false, since: "P3-04",
        args: "exactly one of `name: str` (1..1024 characters) or `id: str` (`tt` + 1..10 digits), `refresh: bool?`", result: "`{card: ImdbCard?}`" },
    Operation { name: "hello", mutating: false, idempotent: false, since: "P1-06",
        args: "See handshake", result: "HelloResult" },
    Operation { name: "app.info", mutating: false, idempotent: false, since: "P1-06",
        args: "none", result: "hello's `proto`, `server`, `engine`, `instance`, `engine_epoch`, `state`, `caps`, `limits`, `policy` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, agent: u64, tui: u64, bar: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08)" },
    Operation { name: "app.client", mutating: false, idempotent: false, since: "P1-06",
        args: "`interactive: bool?`, `glance: bool?` (either or both; a connection drop resets both)", result: "`{}`" },
    Operation { name: "app.shutdown", mutating: true, idempotent: false, since: "P1-06",
        args: "`reason: str?`", result: "`{state: \"pending\"}`, then `bye`" },
    Operation { name: "app.restart", mutating: true, idempotent: false, since: "P1-06",
        args: "`scope: enum(engine|daemon)`", result: "`{state: \"pending\"}`" },
    Operation { name: "app.network", mutating: false, idempotent: false, since: "P1-06",
        args: "none", result: "`{state: NetworkState, interfaces: [str]}`" },
    Operation { name: "transfer.set_limits", mutating: true, idempotent: true, since: "P5-04",
        args: "`download_bps?: u32|null`, `upload_bps?: u32|null`, `alt_download_bps?: u32|null`, `alt_upload_bps?: u32|null`", result: "`{limits: Limits}`" },
    Operation { name: "transfer.set_alt", mutating: true, idempotent: true, since: "P5-04",
        args: "`on: bool`", result: "`{alt: bool}`" },
    Operation { name: "transfer.info", mutating: false, idempotent: false, since: "P1-06",
        args: "none", result: "`{dl_bps: bps?, ul_bps: bps?, session_downloaded: bytes, session_uploaded: bytes}`" },
    Operation { name: "torrents.list", mutating: false, idempotent: false, since: "P1-06",
        args: "`states: [state]?`, `origin: [str]?`", result: "`{torrents: [Torrent]}`" },
    Operation { name: "torrents.get", mutating: false, idempotent: false, since: "P1-06",
        args: "`hash: hash`", result: "`{torrent: TorrentDetail}`" },
    Operation { name: "torrents.add", mutating: true, idempotent: true, since: "P1-06",
        args: "`source: {magnet: str} | {url: str} | {path: str} | {restore: hash}` (exactly one), `stage: bool?`, `stopped: bool?`, `seed: bool?`, `save_path: str?`, `files: [u64]?`, `category: str?`, `origin: open(manual|search|rss|shows|clipboard|handler|agent)?`", result: "`{state: enum(pending|done), hash: hash?, name: str?, duplicate: bool?, deferred: bool?, disk: {free_bytes: bytes, needed_bytes: bytes}?}`" },
    Operation { name: "torrents.start", mutating: true, idempotent: true, since: "P1-06",
        args: "`hashes: [hash]` (1..256), `force: bool?`", result: "`{state: \"done\", started: [hash], skipped: [{hash, reason: open(complete_no_seed|already_running|staged)}]}`" },
    Operation { name: "torrents.stop", mutating: true, idempotent: true, since: "P1-06",
        args: "`hashes: [hash]`", result: "`{state: \"done\", stopped: [hash]}`" },
    Operation { name: "torrents.remove", mutating: true, idempotent: false, since: "P1-06",
        args: "`hashes: [hash]`, `delete_files: bool` (required)", result: "`{state: enum(done|pending), undo_until: ts?}` (pending when deleting files; `undo_until` only when files are kept)" },
    Operation { name: "torrents.files", mutating: false, idempotent: false, since: "P1-06",
        args: "`hash: hash`", result: "`{files: [File]}`" },
    Operation { name: "torrents.set_files", mutating: true, idempotent: true, since: "P1-06",
        args: "`hash: hash`, `include: [u64]?`, `skip: [u64]?`", result: "`{state: \"done\"}`" },
    Operation { name: "torrents.set_seed", mutating: true, idempotent: true, since: "P1-06",
        args: "`hashes: [hash]`, `on: bool`", result: "`{state: \"done\"}`" },
    Operation { name: "torrents.set_upload_limit", mutating: true, idempotent: true, since: "P1-06",
        args: "`hashes: [hash]`, `bps: bps?` (≥ 16384, or `null` for unlimited)", result: "`{state: enum(done|pending)}` (pending until P6-07)" },
    Operation { name: "torrents.open_target", mutating: false, idempotent: false, since: "P1-06",
        args: "`hash: hash`, `file: u64?`", result: "`{path: str, kind: enum(file|folder)}`" },
    Operation { name: "subscribe", mutating: false, idempotent: false, since: "P1-08",
        args: "`topics: [str]` (1..16), `interval_ms: u64?` (250..10000; applies to every topic of the call and is ignored by the \"on change\" topics of A.7; when omitted, each topic uses its A.7 default), `since_seq: u64?` (only with the `events` topic, A.7)", result: "`{sub: u64, topics: [str]}`" },
    Operation { name: "unsubscribe", mutating: false, idempotent: false, since: "P1-08",
        args: "`sub: u64`", result: "`{}`" },
];

pub mod app;
pub mod search;
pub mod recheck;
pub mod queue;
pub mod trackers;
pub mod rss;
pub mod shows;
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

/// Bound input before dynamic programming; only near spellings are suggestions.
pub fn nearest(input: &str) -> Vec<&'static str> {
    if input.len() > 80 {
        return Vec::new();
    }
    let a: Vec<_> = input.chars().collect();
    let mut candidates: Vec<_> = TABLE
        .iter()
        .filter_map(|op| {
            let b: Vec<_> = op.name.chars().collect();
            let mut previous: Vec<usize> = (0..=b.len()).collect();
            let mut next = vec![0; b.len() + 1];
            for (i, x) in a.iter().enumerate() {
                next[0] = i + 1;
                for (j, y) in b.iter().enumerate() {
                    next[j + 1] = (previous[j + 1] + 1)
                        .min(next[j] + 1)
                        .min(previous[j] + usize::from(x != y));
                }
                std::mem::swap(&mut previous, &mut next);
            }
            (previous[b.len()] <= 2).then_some((previous[b.len()], op.name))
        })
        .collect();
    candidates.sort_unstable();
    candidates
        .into_iter()
        .take(3)
        .map(|(_, name)| name)
        .collect()
}
pub fn markdown() -> String {
    let mut text=String::from("| Operation | Arguments | Result | Mutating | Idempotent | Since |\n|---|---|---|---|---|---|\n");
    for op in TABLE {
        text.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            op.name,
            op.args.replace('|', "\\|"),
            op.result.replace('|', "\\|"),
            op.mutating,
            op.idempotent,
            op.since
        ));
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_matches_ipc_md() {
        let mut documented = std::collections::BTreeMap::new();
        for line in include_str!("../../docs/IPC.md").lines() {
            let escaped = line.replace("\\|", "<PIPE>");
            let cells: Vec<_> = escaped.split('|').map(str::trim).collect();
            if cells.len() == 7 && matches!(cells[4], "yes" | "no") {
                documented.insert(
                    cells[1].trim_matches('`').to_owned(),
                    (
                        cells[2].replace("<PIPE>", "|"),
                        cells[3].replace("<PIPE>", "|"),
                        cells[4] == "yes",
                    ),
                );
            }
        }
        assert_eq!(documented.len(), TABLE.len());
        for op in TABLE {
            assert_eq!(
                documented[op.name],
                (op.args.into(), op.result.into(), op.mutating),
                "{}",
                op.name
            );
        }
    }
    #[test]
    fn suggestions_are_bounded_near_spellings() {
        assert!(nearest("torrents.lsit").contains(&"torrents.list"));
        assert!(nearest("app.inof").contains(&"app.info"));
        assert!(nearest("nope").is_empty());
        assert!(nearest(&"x".repeat(81)).is_empty());
    }
}
