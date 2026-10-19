//! No-timeout move acknowledgement and its eventual correlated result.
use super::{ApiResult, engine_error, error};
use crate::{
    api::*,
    daemon::Daemon,
    engine::{
        self,
        ops::{EngineOp, EngineResult},
    },
};
use std::{
    path::{Component, Path},
    sync::Arc,
};
pub fn begin(daemon: &Daemon, args: &MoveArgs) -> ApiResult<Arc<engine::moving::Batch>> {
    let path = Path::new(&args.save_path);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || args.save_path.len() > 4096
        || args.save_path.as_bytes().contains(&0)
    {
        return Err(error(
            ErrorCode::InvalidArgs,
            "save_path must be an absolute directory without parent traversal",
            Some("save_path"),
        ));
    }
    let EngineResult::Move(batch) = daemon
        .execute(EngineOp::Move {
            hashes: args
                .hashes
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect(),
            save_path: path.into(),
        })
        .map_err(|e| {
            if let Some(missing) = e.downcast_ref::<engine::recheck::MissingTorrent>() {
                let mut failure = error(ErrorCode::NotFound, "torrent record not found", None);
                failure.details = Some(serde_json::Map::from_iter([(
                    "hash".into(),
                    missing.0.clone().into(),
                )]));
                failure
            } else {
                engine_error(e)
            }
        })?
    else {
        unreachable!("move batch")
    };
    Ok(batch)
}
pub fn complete(result: std::result::Result<(), engine::moving::Failure>) -> ApiResult<Reply> {
    match result {
        Ok(()) => Ok(Reply::TorrentsMove(MutationResult::Done)),
        Err(failure) => {
            use engine::moving::FailureKind;
            let code = match failure.kind {
                FailureKind::Source => ErrorCode::InvalidState,
                FailureKind::Destination => ErrorCode::InvalidArgs,
                FailureKind::Internal => ErrorCode::Internal,
                FailureKind::Stopping => ErrorCode::EngineUnavailable,
            };
            let mut error = error(
                code,
                &failure.message,
                matches!(failure.kind, FailureKind::Destination).then_some("save_path"),
            );
            if matches!(failure.kind, FailureKind::Source) {
                error.details = Some(serde_json::Map::from_iter([(
                    "reason".into(),
                    "source_unavailable".into(),
                )]));
            }
            if matches!(failure.kind, FailureKind::Stopping) {
                error.details = Some(serde_json::Map::from_iter([
                    ("state".into(), "stopping".into()),
                    ("message".into(), "engine is stopping".into()),
                ]));
            }
            if !failure.hash.is_empty() {
                error
                    .details
                    .get_or_insert_with(Default::default)
                    .insert("hash".into(), failure.hash.into());
            }
            Err(error)
        }
    }
}
#[cfg(test)]
#[test]
fn move_fixture_roundtrips() {
    crate::api::fixtures::check_group(
        "move",
        include_str!("../../../contracts/ipc/v1/move.ndjson"),
    )
    .unwrap();
}
