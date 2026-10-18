//! Metadata shared with dispatch and the future `api --list` catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub name: &'static str,
    pub mutating: bool,
    pub idempotent: bool,
    pub since: &'static str,
}
pub const TABLE: &[Operation] = &[
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
