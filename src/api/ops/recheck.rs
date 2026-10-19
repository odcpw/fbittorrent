//! API adapter for engine-owned verification. Dispatch never waits for hashing.
use super::{engine_error, ApiResult};
use crate::{
    api::*,
    daemon::Daemon,
    engine::{
        self,
        ops::{EngineOp, EngineResult},
    },
};
use std::sync::Arc;

pub fn begin(daemon: &Daemon, args: &HashesArgs) -> ApiResult<Arc<engine::recheck::Batch>> {
    let hashes = args
        .hashes
        .as_slice()
        .iter()
        .map(ToString::to_string)
        .collect();
    let EngineResult::Recheck(batch) =
        daemon
            .execute(EngineOp::RecheckBatch { hashes })
            .map_err(|e| {
                if let Some(missing) = e.downcast_ref::<engine::recheck::MissingTorrent>() {
                    let mut failure =
                        super::error(ErrorCode::NotFound, "torrent record not found", None);
                    failure.details = Some(serde_json::Map::from_iter([(
                        "hash".into(),
                        serde_json::json!(missing.0),
                    )]));
                    failure
                } else {
                    engine_error(e)
                }
            })?
    else {
        unreachable!("typed recheck result")
    };
    Ok(batch)
}
pub fn complete(report: engine::recheck::Report) -> Reply {
    Reply::TorrentsRecheck(RecheckReply::Complete(RecheckResult {
        rechecked: report
            .rechecked
            .into_iter()
            .map(|hash| Hash::new(hash).unwrap())
            .collect(),
        failed: report
            .failed
            .into_iter()
            .map(|(hash, message)| RecheckFailure {
                hash: Hash::new(hash).unwrap(),
                message,
            })
            .collect(),
        have_bytes: report
            .have_bytes
            .into_iter()
            .map(|(hash, bytes)| (Hash::new(hash).unwrap(), bytes))
            .collect(),
    }))
}

#[cfg(test)]
#[test]
fn recheck_fixture_roundtrips() {
    crate::api::fixtures::check_group(
        "recheck",
        include_str!("../../../contracts/ipc/v1/recheck.ndjson"),
    )
    .unwrap();
}
