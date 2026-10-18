use super::{engine_error, error, ApiResult};
use crate::{
    api::*,
    daemon::Daemon,
    engine::ops::{EngineOp, EngineResult},
};

pub fn info(daemon: &Daemon) -> ApiResult<Reply> {
    let result = daemon
        .execute(EngineOp::TransferInfo)
        .map_err(engine_error)?;
    let EngineResult::Transfer(info) = result else {
        return Err(error(
            ErrorCode::Internal,
            "unexpected engine transfer result",
            None,
        ));
    };
    let live = info.connection_status == "connected";
    Ok(Reply::TransferInfo(TransferInfo {
        session_downloaded: info.dl_info_data.max(0) as u64,
        session_uploaded: info.up_info_data.max(0) as u64,
        dl_bps: live.then_some(info.dl_info_speed.max(0) as u64),
        ul_bps: live.then_some(info.up_info_speed.max(0) as u64),
    }))
}
