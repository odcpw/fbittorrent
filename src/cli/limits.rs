//! Global/turtle policy, live when connected and saved under the owner lock offline.
use crate::{
    agent::{AppError, Code},
    api,
    config::Config,
};
use anyhow::Result;
use serde_json::json;

fn rate(text: Option<String>) -> Result<api::Update<u32>> {
    let Some(text) = text else {
        return Ok(api::Update::Unset);
    };
    let lower = text.to_ascii_lowercase();
    let (digits, scale) = if let Some(v) = lower.strip_suffix('k') {
        (v, 1024_u32)
    } else if let Some(v) = lower.strip_suffix('m') {
        (v, 1024_u32 * 1024)
    } else {
        (lower.as_str(), 1)
    };
    let bps = digits
        .parse::<u32>()
        .ok()
        .and_then(|v| v.checked_mul(scale))
        .filter(|v| *v == 0 || *v >= 16384)
        .ok_or_else(|| {
            AppError::new(
                Code::Usage,
                "limit must be 0 (unlimited) or at least 16384 bytes/s; K/M suffixes are accepted",
            )
        })?;
    Ok(if bps == 0 {
        api::Update::Clear
    } else {
        api::Update::Set(bps)
    })
}
#[allow(clippy::too_many_arguments)]
pub fn run(
    cfg: &Config,
    json_mode: bool,
    down: Option<String>,
    up: Option<String>,
    alt: Option<String>,
    alt_down: Option<String>,
    alt_up: Option<String>,
) -> Result<Code> {
    let args = api::SetLimitsArgs {
        download_bps: rate(down)?,
        upload_bps: rate(up)?,
        alt_download_bps: rate(alt_down)?,
        alt_upload_bps: rate(alt_up)?,
    };
    let mut patch = api::ops::transfer::limits_patch(&args);
    if let Some(on) = &alt {
        patch.insert("alt_limits_on".into(), json!(on == "on"));
    }
    let (settings, running) = match api::client::Client::connect(cfg, json_mode) {
        Ok(client) => {
            if !api::ops::transfer::limits_patch(&args).is_empty() {
                client.call("transfer.set_limits", serde_json::to_value(&args)?)?;
            }
            if let Some(on) = alt {
                client.call("transfer.set_alt", json!({"on":on == "on"}))?;
            }
            (client.call("app.settings.get", json!({}))?, true)
        }
        Err(_) => {
            let (_lock, _) = super::offline::locked()?;
            let (changed, ack) = api::ops::settings::prepare(&Config::load()?, &patch)
                .map_err(api::errors::app_error)?;
            if !ack.applied.is_empty() {
                changed.save()?;
            }
            (
                serde_json::to_value(api::ops::settings::view(&changed))?,
                false,
            )
        }
    };
    let output = json!({"ok":true,"running":running,"alt":settings["alt_limits_on"],"limits":api::ops::transfer::limits_from_settings(&settings)});
    crate::emit(json_mode, output.clone(), || println!("{}", output));
    Ok(Code::Ok)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_cli_rate_syntax() {
        assert_eq!(rate(None).unwrap(), api::Update::Unset);
        assert_eq!(rate(Some("0".into())).unwrap(), api::Update::Clear);
        assert_eq!(rate(Some("64K".into())).unwrap(), api::Update::Set(65536));
        assert_eq!(rate(Some("2m".into())).unwrap(), api::Update::Set(2097152));
        for bad in ["-1", "1", "NaN", "4294967296", "5000M"] {
            assert!(rate(Some(bad.into())).is_err());
        }
    }
}
