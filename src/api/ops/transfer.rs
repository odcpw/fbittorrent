use super::{ApiResult, engine_error, error};
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

/// Persisted limits retain both sets; `alt` selects which set the session uses.
pub fn limits_from_settings(settings: &serde_json::Value) -> serde_json::Value {
    let mut limits = serde_json::Map::new();
    for (wire, key) in [
        ("download_bps", "download_limit_bps"),
        ("upload_bps", "upload_limit_bps"),
        ("alt_download_bps", "alt_download_limit_bps"),
        ("alt_upload_bps", "alt_upload_limit_bps"),
    ] {
        if let Some(value) = settings.get(key).filter(|v| !v.is_null()) {
            limits.insert(wire.into(), value.clone());
        }
    }
    limits.into()
}

pub fn limits_patch(args: &SetLimitsArgs) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::{Value, json};
    let mut patch = serde_json::Map::new();
    for (key, update) in [
        ("download_limit_bps", &args.download_bps),
        ("upload_limit_bps", &args.upload_bps),
        ("alt_download_limit_bps", &args.alt_download_bps),
        ("alt_upload_limit_bps", &args.alt_upload_bps),
    ] {
        match update {
            Update::Unset => {}
            Update::Clear | Update::Set(0) => {
                patch.insert(key.into(), Value::Null);
            }
            Update::Set(value) => {
                patch.insert(key.into(), json!(value));
            }
        }
    }
    patch
}

pub fn set_limits(daemon: &Daemon, args: &SetLimitsArgs) -> ApiResult<Reply> {
    super::settings::set(daemon, &limits_patch(args))?;
    let settings = serde_json::to_value(super::settings::get(daemon)?).unwrap();
    Ok(Reply::TransferSetLimits(LimitsResult {
        limits: serde_json::from_value(limits_from_settings(&settings)).unwrap(),
    }))
}
pub fn set_alt(daemon: &Daemon, on: bool) -> ApiResult<Reply> {
    super::settings::set(
        daemon,
        &serde_json::from_value(serde_json::json!({"alt_limits_on":on})).unwrap(),
    )?;
    Ok(Reply::TransferSetAlt(AltResult { alt: on }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn limits_omission_null_zero_and_validation() {
        let args: SetLimitsArgs = serde_json::from_value(
            json!({"download_bps":null,"upload_bps":0,"alt_download_bps":65536}),
        )
        .unwrap();
        let patch = limits_patch(&args);
        assert_eq!(
            serde_json::to_value(&args).unwrap(),
            json!({"download_bps":null,"upload_bps":0,"alt_download_bps":65536})
        );
        let cfg = crate::config::Config {
            alt_upload_limit_bps: Some(32768),
            ..Default::default()
        };
        let (changed, _) = super::super::settings::prepare(&cfg, &patch).unwrap();
        assert_eq!(changed.alt_upload_limit_bps, Some(32768));
        assert_eq!(
            limits_from_settings(
                &serde_json::to_value(super::super::settings::view(&changed)).unwrap()
            ),
            json!({"alt_download_bps":65536,"alt_upload_bps":32768})
        );
        for bad in [
            json!({"upload_bps":-1}),
            json!({"upload_bps":4294967296_u64}),
            json!({"upload_bps":"1M"}),
            json!({"extra":1}),
        ] {
            assert!(serde_json::from_value::<SetLimitsArgs>(bad).is_err());
        }
        let args = serde_json::from_value(json!({"upload_bps":1})).unwrap();
        assert!(super::super::settings::prepare(&cfg, &limits_patch(&args)).is_err());
    }
}
