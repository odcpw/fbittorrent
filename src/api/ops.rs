//! Metadata shared with dispatch and the future `api --list` catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub name: &'static str,
    pub mutating: bool,
    pub idempotent: bool,
    pub since: &'static str,
}
pub const TABLE: &[Operation] = &[
    Operation { name: "search.start", mutating: true, idempotent: false, since: "P3-01" },
    Operation { name: "search.stop", mutating: true, idempotent: false, since: "P3-01" },
    Operation { name: "search.providers", mutating: false, idempotent: false, since: "P3-01" },
    Operation { name: "search.set_provider", mutating: true, idempotent: true, since: "P3-01" },
    Operation { name: "search.install", mutating: true, idempotent: false, since: "P3-01" },
    Operation { name: "search.browse_sources", mutating: false, idempotent: false, since: "P3-01" },
    Operation { name: "search.browse", mutating: false, idempotent: false, since: "P3-01" },
    Operation { name: "search.grab", mutating: true, idempotent: false, since: "P3-01" },

    Operation { name: "torrents.recheck", mutating: true, idempotent: false, since: "P5-03" },
    Operation { name: "app.settings.get", mutating: false, idempotent: false, since: "P3-05" },
    Operation { name: "app.settings.set", mutating: true, idempotent: true, since: "P3-05" },
    Operation {
        name: "imdb.lookup",
        mutating: false,
        idempotent: false,
        since: "P3-04",
    },
    Operation {
        name: "hello",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "app.info",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "app.client",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "app.shutdown",
        mutating: true,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "app.restart",
        mutating: true,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "app.network",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "transfer.info",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "torrents.list",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "torrents.get",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "torrents.add",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.start",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.stop",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.remove",
        mutating: true,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "torrents.files",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "torrents.set_files",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.set_seed",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.set_upload_limit",
        mutating: true,
        idempotent: true,
        since: "P1-06",
    },
    Operation {
        name: "torrents.open_target",
        mutating: false,
        idempotent: false,
        since: "P1-06",
    },
    Operation {
        name: "subscribe",
        mutating: false,
        idempotent: false,
        since: "P1-08",
    },
    Operation {
        name: "unsubscribe",
        mutating: false,
        idempotent: false,
        since: "P1-08",
    },
];

pub mod app;
pub mod search;
pub mod settings;
pub mod recheck;
pub mod imdb;
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
