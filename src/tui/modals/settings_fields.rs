//! Draft settings fields and a minimal API patch preserve concurrent edits.
use super::*;
const LIMIT_NAMES: [&str; 4] = [
    "Download limit",
    "Upload limit",
    "Turtle download limit",
    "Turtle upload limit",
];
pub(super) const SETTINGS_ROWS: usize = 13;
pub(super) const DESTINATION_ROW: usize = 9;
pub(super) const SEED_ROW: usize = 10;
pub(super) const IDLE_ROW: usize = 11;
pub(super) const CLIPBOARD_ROW: usize = 12;
pub(super) const PORT_ROW: usize = 8;
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
        "save_path",
        "seed_by_default",
        "idle_exit_secs",
        "clipboard_watch",
        "peer_port",
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
    cfg.save_path=Some(settings.save_path.clone());
    cfg.seed_by_default=settings.seed_by_default;
    cfg.idle_exit_secs=settings.idle_exit_secs;
    cfg.clipboard_watch=settings.clipboard_watch;
    cfg.show_quality=settings.show_quality.clone();
    cfg.peer_port = u16::try_from(settings.peer_port)?;
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
    pub(super) fn edit_peer_port(&mut self) -> Cmd<Msg> {
        let port = self.settings.as_ref().unwrap().cfg.peer_port;
        self.open_prompt(
            PromptKind::SettingPeerPort,
            "Peer port · 0 automatic · 1–65535",
            false,
        );
        self.prompt
            .as_mut()
            .unwrap()
            .input
            .set_value(port.to_string());
        Cmd::none()
    }
    pub(super) fn apply_peer_port_input(&mut self, value: &str) -> Cmd<Msg> {
        if let Some(modal) = self.settings.as_mut() {
            match value.trim().parse::<u16>() {
                Ok(port) => {
                    modal.cfg.peer_port = port;
                    modal.error.clear();
                }
                Err(_) => {
                    modal.error = "Use a peer port from 0 to 65535 (0 chooses automatically)".into()
                }
            };
        }
        Cmd::none()
    }

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

impl TuiApp {
    pub(super) fn edit_destination(&mut self)->Cmd<Msg> {
        let value=self.settings.as_ref().and_then(|m|m.cfg.save_path.clone()).unwrap_or_default();
        self.open_prompt(PromptKind::SettingDestination,"Download destination · empty uses Downloads",false);
        self.prompt.as_mut().unwrap().input.set_value(value);Cmd::none()
    }
    pub(super) fn apply_destination(&mut self,value:&str)->Cmd<Msg> {
        if let Some(modal)=self.settings.as_mut() {
            if value.is_empty() {modal.cfg.save_path=None;modal.error.clear();}
            else if !std::path::Path::new(value).is_absolute() {modal.error="Use an absolute download destination".into();}
            else {modal.cfg.save_path=Some(value.into());modal.error.clear();}
        }
        Cmd::none()
    }
    pub(super) fn edit_idle_exit(&mut self)->Cmd<Msg> {
        let value=self.settings.as_ref().unwrap().cfg.idle_exit_secs.to_string();
        self.open_prompt(PromptKind::SettingIdleExit,"Idle exit · seconds · 0 disables it",false);
        self.prompt.as_mut().unwrap().input.set_value(value);Cmd::none()
    }
    pub(super) fn apply_idle_exit(&mut self,value:&str)->Cmd<Msg> {
        if let Some(modal)=self.settings.as_mut() {
            match value.parse::<u64>() {
                Ok(seconds)=>{modal.cfg.idle_exit_secs=seconds;modal.error.clear();},
                Err(_)=>modal.error="Use a nonnegative number of seconds; 0 disables idle exit".into(),
            }
        }
        Cmd::none()
    }
}
