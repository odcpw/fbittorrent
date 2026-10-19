//! Draft settings fields and a minimal API patch preserve concurrent edits.
use super::*;
const LIMIT_NAMES: [&str; 4] = [
    "Download limit",
    "Upload limit",
    "Turtle download limit",
    "Turtle upload limit",
];
pub(super) const SETTINGS_ROWS: usize = 8;
pub(super) fn limit(cfg: &Config, index: usize) -> Option<u32> {
    match index {
        0 => cfg.download_limit_bps,
        1 => cfg.upload_limit_bps,
        2 => cfg.alt_download_limit_bps,
        _ => cfg.alt_upload_limit_bps,
    }
}
pub(super) fn rate_text(value: Option<u32>) -> String {
    value
        .map(|n| format!("{n} B/s"))
        .unwrap_or_else(|| "Unlimited".into())
}
pub(super) fn settings_patch(original: &Config, draft: &Config) -> serde_json::Value {
    let original = serde_json::to_value(original).unwrap();
    let draft = serde_json::to_value(draft).unwrap();
    let mut patch = serde_json::Map::new();
    for key in [
        "max_active_downloads",
        "peer_encryption",
        "network_interface",
        "reduce_motion",
        "download_limit_bps",
        "upload_limit_bps",
        "alt_download_limit_bps",
        "alt_upload_limit_bps",
    ] {
        if original[key] != draft[key] {
            patch.insert(key.into(), draft[key].clone());
        }
    }
    patch.into()
}
pub(super) fn native_settings(cfg: &mut Config, settings: &crate::api::Settings) -> Result<()> {
    cfg.max_active_downloads = usize::try_from(settings.max_active_downloads)?;
    cfg.peer_encryption = match settings.peer_encryption {
        crate::api::PeerEncryption::Prefer => "prefer",
        crate::api::PeerEncryption::Require => "require",
        crate::api::PeerEncryption::Off => "off",
    }
    .into();
    cfg.network_interface = settings.network_interface.clone();
    cfg.reduce_motion = settings.reduce_motion.unwrap_or(false);
    cfg.alt_limits_on = settings.alt_limits_on;
    cfg.download_limit_bps = settings.download_limit_bps.map(u32::try_from).transpose()?;
    cfg.upload_limit_bps = settings.upload_limit_bps.map(u32::try_from).transpose()?;
    cfg.alt_download_limit_bps = settings
        .alt_download_limit_bps
        .map(u32::try_from)
        .transpose()?;
    cfg.alt_upload_limit_bps = settings
        .alt_upload_limit_bps
        .map(u32::try_from)
        .transpose()?;
    cfg.validate()?;
    Ok(())
}
fn parse_rate(value: &str) -> Result<Option<u32>> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(None);
    }
    let (digits, scale) = if let Some(v) = value.strip_suffix('k') {
        (v, 1024u32)
    } else if let Some(v) = value.strip_suffix('m') {
        (v, 1024u32 * 1024)
    } else {
        (value.as_str(), 1)
    };
    let n = digits
        .parse::<u32>()
        .ok()
        .and_then(|v| v.checked_mul(scale))
        .filter(|v| *v == 0 || *v >= 16384)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Use empty or 0 for unlimited, or at least 16384 B/s; K/M suffixes are accepted"
            )
        })?;
    Ok((n != 0).then_some(n))
}
impl TuiApp {
    pub(super) fn edit_limit(&mut self, index: usize) -> Cmd<Msg> {
        let value = self.settings.as_ref().and_then(|m| limit(&m.cfg, index));
        self.open_prompt(
            PromptKind::SettingLimit(index),
            &format!("{} · B/s, K/M · empty = unlimited", LIMIT_NAMES[index]),
            false,
        );
        if let Some(value) = value {
            self.prompt
                .as_mut()
                .unwrap()
                .input
                .set_value(value.to_string());
        }
        Cmd::none()
    }
    pub(super) fn apply_limit_input(&mut self, index: usize, value: &str) -> Cmd<Msg> {
        match parse_rate(value) {
            Ok(value) => {
                if let Some(modal) = self.settings.as_mut() {
                    match index {
                        0 => modal.cfg.download_limit_bps = value,
                        1 => modal.cfg.upload_limit_bps = value,
                        2 => modal.cfg.alt_download_limit_bps = value,
                        _ => modal.cfg.alt_upload_limit_bps = value,
                    };
                    modal.error.clear();
                }
            }
            Err(error) => {
                if let Some(modal) = self.settings.as_mut() {
                    modal.error = error.to_string();
                }
            }
        }
        Cmd::none()
    }
}
pub(super) fn limit_label(cfg: &Config, index: usize) -> String {
    format!(
        "{:<24} {}",
        LIMIT_NAMES[index],
        rate_text(limit(cfg, index))
    )
}
