//! Owner operations: lifecycle and process identity never belong to an engine slot.
use super::{error, ApiResult};
use crate::api::{self, *};
use crate::daemon::{
    lifecycle::{RestartReason, StopReason},
    Daemon,
};
use serde_json::{json, Value};

pub const CAPS:&[&str]=&["torrents","files","subscribe","mutations","deferred_add","imdb","settings","log","limits","clipboard","rss","shows","peers"];

pub fn hello(daemon: &Daemon) -> HelloResult {
    let mut info = daemon.info();
    if daemon.is_stopping() {
        info["state"] = json!("stopping");
    }
    info["proto"] = json!(PROTO);
    info["caps"] = json!(CAPS);
    info["limits"] =
        json!({"line_in":LINE_IN,"line_out":LINE_OUT,"inflight":8,"subs":16,"hashes":256});
    serde_json::from_value(info).expect("daemon identity has the contract shape")
}

pub fn dispatch(
    daemon: &Daemon,
    call: &Call,
    reference: &str,
    counts: Value,
    active_counts: Value,
    uptime: u64,
    subscriptions: Subscriptions,
) -> ApiResult<Reply> {
    Ok(match call {
        Call::AppClipboardDismiss { args } => {
            daemon.clipboard.dismiss(args.hash.as_str());
            Reply::AppClipboardDismiss(EmptyResult {})
        }
        Call::AppLog { args } => Reply::AppLog(daemon.logs.query(&args.clone().unwrap_or_default())),
        Call::AppStats { .. } => Reply::AppStats(StatsResult {
            connections: serde_json::from_value(counts).unwrap(),
            active_clients: serde_json::from_value(active_counts).unwrap(),
        }),
        Call::AppSettingsGet { .. } => Reply::AppSettingsGet(super::settings::get(daemon)?),
        Call::AppSettingsSet { args } => Reply::AppSettingsSet(super::settings::set(daemon, args)?),
        Call::AppInfo { .. } => {
            let mut value = serde_json::to_value(hello(daemon)).unwrap();
            value["uptime_s"] = json!(uptime);
            value["upgrade_pending"] = json!(daemon.upgrade_pending.load(std::sync::atomic::Ordering::Acquire));
            value["clients"] = counts;
            value["subscriptions"] = serde_json::to_value(subscriptions).unwrap();
            Reply::AppInfo(serde_json::from_value(value).unwrap())
        }
        Call::AppNetwork { .. } => {
            let network = daemon.network();
            Reply::AppNetwork(NetworkResult {
                state: api::NetworkState {
                    ready: network.ready,
                    message: network.message,
                    selection: network.selection,
                    interface: network.interface,
                },
                interfaces: crate::network_app::interfaces().map_err(super::engine_error)?,
            })
        }
        Call::AppShutdown { .. } => {
            daemon.lifecycle.request_stop(StopReason::Quit);
            Reply::AppShutdown(PendingResult {
                state: PendingState::Pending,
                reference: MutationRef::new(reference).unwrap(),
            })
        }
        Call::AppRestart { args } => {
            match args.scope {
                RestartScope::Engine => daemon.lifecycle.request_restart(RestartReason::Explicit),
                RestartScope::Daemon => daemon.lifecycle.request_stop(StopReason::Upgrade),
            }
            Reply::AppRestart(PendingResult {
                state: PendingState::Pending,
                reference: MutationRef::new(reference).unwrap(),
            })
        }
        _ => {
            return Err(error(
                ErrorCode::Unsupported,
                "operation is not an owner operation",
                None,
            ))
        }
    })
}
