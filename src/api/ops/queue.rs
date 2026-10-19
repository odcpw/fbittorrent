//! Queue controls retain the engine's durable intent and typed refusal reasons.
use super::{engine_error, ApiResult};
use crate::{
    api::*,
    daemon::Daemon,
    engine::ops::{EngineOp, EngineResult, HashSelection, QueueDirection},
};

fn execute(daemon: &Daemon, op: EngineOp) -> ApiResult<EngineResult> {
    daemon.execute(op).map_err(|e| {
        let text = e.to_string();
        let missing = e
            .downcast_ref::<crate::engine::recheck::MissingTorrent>()
            .map(|m| m.0.clone())
            .or_else(|| {
                text.split_whitespace()
                    .last()
                    .filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_owned)
            });
        let mut failure = engine_error(e);
        if failure.code == ErrorCode::NotFound {
            failure.details = missing
                .map(|hash| serde_json::Map::from_iter([("hash".into(), serde_json::json!(hash))]));
        }
        if failure
            .details
            .as_ref()
            .is_some_and(|d| d.get("reason").is_some_and(|v| v == "complete_no_seed"))
        {
            let hash = failure.details.as_ref().unwrap()["hash"].as_str().unwrap();
            failure.hint = Some(format!("fbittorrent seed {hash}"));
        }
        failure
    })
}
fn selection(hashes: &Hashes) -> HashSelection {
    HashSelection(hashes.as_slice().iter().map(ToString::to_string).collect())
}
pub fn dispatch(daemon: &Daemon, call: &Call) -> ApiResult<Reply> {
    Ok(match call {
        Call::TorrentsForceStart { args } => {
            execute(
                daemon,
                EngineOp::ForceStart {
                    hashes: selection(&args.hashes),
                    on: args.on,
                },
            )?;
            Reply::TorrentsForceStart(DoneResult {
                state: DoneState::Done,
            })
        }
        Call::TorrentsQueueMove { args } => {
            let to = match args.to {
                QueueTo::Top => QueueDirection::Top,
                QueueTo::Up => QueueDirection::Up,
                QueueTo::Down => QueueDirection::Down,
                QueueTo::Bottom => QueueDirection::Bottom,
            };
            let EngineResult::QueuePosition(queue_pos) = execute(
                daemon,
                EngineOp::QueueMove {
                    hash: args.hash.to_string(),
                    to,
                },
            )?
            else {
                unreachable!("queue position");
            };
            Reply::TorrentsQueueMove(QueueMoveResult {
                state: DoneState::Done,
                queue_pos,
            })
        }
        _ => unreachable!("queue operation"),
    })
}
pub fn start_forced(daemon: &Daemon, args: &StartArgs) -> ApiResult<Reply> {
    let mut hashes = args.hashes.as_slice().to_vec();
    let mut seen = std::collections::HashSet::new();
    hashes.retain(|h| seen.insert(h.clone()));
    execute(
        daemon,
        EngineOp::ForceStart {
            hashes: HashSelection(hashes.iter().map(ToString::to_string).collect()),
            on: true,
        },
    )?;
    Ok(Reply::TorrentsStart(StartResult {
        state: DoneState::Done,
        started: hashes,
        skipped: Vec::new(),
    }))
}

#[cfg(test)]
#[test]
fn queue_fixture_roundtrips() {
    crate::api::fixtures::check_group(
        "queue",
        include_str!("../../../contracts/ipc/v1/queue.ndjson"),
    )
    .unwrap();
}
