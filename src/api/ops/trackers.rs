//! Tracker diagnostics contain sanitized messages but retain the private URL.
use super::{ApiResult, engine_error};
use crate::{
    api::*,
    daemon::Daemon,
    engine::ops::{EngineOp, EngineResult},
};
fn execute(daemon: &Daemon, op: EngineOp, hash: Option<String>) -> ApiResult<EngineResult> {
    daemon.execute(op).map_err(|error| {
        let missing = hash.or_else(|| {
            error
                .to_string()
                .split_whitespace()
                .last()
                .filter(|word| word.len() == 40 && word.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_owned)
        });
        let mut failure = engine_error(error);
        if failure.code == ErrorCode::NotFound {
            failure.details =
                missing.map(|hash| serde_json::Map::from_iter([("hash".into(), hash.into())]));
        }
        failure
    })
}
pub(crate) fn read(daemon: &Daemon, hash: &Hash)->ApiResult<Vec<Tracker>> {
    let EngineResult::Trackers(rows)=execute(daemon,EngineOp::Trackers{hash:hash.to_string()},Some(hash.to_string()))? else {unreachable!("typed tracker result")};
    Ok(rows.into_iter().map(crate::api::trackers::wire).collect())
}
pub fn dispatch(daemon: &Daemon, call: &Call) -> ApiResult<Reply> {
    match call {
        Call::TorrentsTrackers { args } => Ok(Reply::TorrentsTrackers(TrackersResult {trackers:read(daemon,&args.hash)?})),
        Call::TorrentsReannounce { args } => {
            execute(
                daemon,
                EngineOp::Reannounce {
                    hashes: args
                        .hashes
                        .as_slice()
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                },
                None,
            )?;
            Ok(Reply::TorrentsReannounce(DoneResult {
                state: DoneState::Done,
            }))
        }
        _ => unreachable!("tracker operation"),
    }
}
#[cfg(test)]
#[test]
fn tracker_fixture_roundtrips() {
    crate::api::fixtures::check_group(
        "trackers",
        include_str!("../../../contracts/ipc/v1/trackers.ndjson"),
    )
    .unwrap();
}
