//! Durable settings transactions. The daemon owns the gate across slot restarts.
use super::{engine_error, error, ApiResult};
use crate::{
    api::*,
    config::Config,
    daemon::{lifecycle::RestartReason, Daemon},
};
use serde_json::{Map, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

pub const RESTART_CLASS: [&str; 3] = ["peer_encryption", "network_interface", "peer_port"];
pub use crate::api::request::SETTINGS_KEYS as KEYS;

#[derive(Default)]
pub struct Service {
    transaction: Mutex<()>,
    generation: AtomicU64,
}
impl Service {
    /// P1-08 can sample this generation and publish the settings state on change.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

pub fn view(cfg: &Config) -> Settings {
    Settings {
        max_active_downloads: cfg.max_active_downloads as u64,
        peer_encryption: match cfg.peer_encryption.as_str() {
            "off" => PeerEncryption::Off,
            "require" => PeerEncryption::Require,
            _ => PeerEncryption::Prefer,
        },
        network_interface: cfg.network_interface.clone(),
        save_path: cfg.save_path.clone().unwrap_or_else(|| {
            dirs::download_dir()
                .unwrap_or_else(|| crate::config::data_dir().join("downloads"))
                .to_string_lossy()
                .into_owned()
        }),
        seed_by_default: cfg.seed_by_default,
        autostart: cfg.autostart,
        idle_exit_secs: cfg.idle_exit_secs,
        notify: cfg.notify,
        timeout_secs: cfg.timeout_secs,
        handler: if cfg.handler == "add" {
            Handler::Add
        } else {
            Handler::Confirm
        },
        open_command: cfg.open_command.clone(),
        clipboard_watch: cfg.clipboard_watch,
        reduce_motion: Some(cfg.reduce_motion),
        peer_port: u64::from(cfg.peer_port),
        show_quality: cfg.show_quality.clone(),
        alt_limits_on: cfg.alt_limits_on,
        download_limit_bps: cfg.download_limit_bps.map(u64::from),
        upload_limit_bps: cfg.upload_limit_bps.map(u64::from),
        alt_download_limit_bps: cfg.alt_download_limit_bps.map(u64::from),
        alt_upload_limit_bps: cfg.alt_upload_limit_bps.map(u64::from),
        restart_class: RESTART_CLASS.iter().map(|s| (*s).into()).collect(),
    }
}

/// Validate every supplied key before saving or touching a live session.
/// Merging against Config preserves legacy-only keys and validates numeric types
/// without lossy casts. Null clears only the optional binding and speed limits.
pub fn prepare(cfg: &Config, patch: &Map<String, Value>) -> ApiResult<(Config, SettingsSetResult)> {
    let mut merged = serde_json::to_value(cfg).unwrap();
    let mut applied = Vec::new();
    for (key, value) in patch {
        if !KEYS.contains(&key.as_str()) {
            return Err(error(
                ErrorCode::InvalidArgs,
                "unknown or read-only settings key",
                Some(key),
            ));
        }
        if value.is_null() && key != "network_interface" && !key.ends_with("_limit_bps") {
            return Err(error(
                ErrorCode::InvalidArgs,
                "setting must not be null",
                Some(key),
            ));
        }
        let old = merged[key].clone();
        merged[key] = value.clone();
        let candidate: Config = serde_json::from_value(merged.clone()).map_err(|_| {
            error(
                ErrorCode::InvalidArgs,
                "invalid settings value or type",
                Some(key),
            )
        })?;
        candidate
            .validate()
            .map_err(|e| error(ErrorCode::InvalidArgs, &e.to_string(), Some(key)))?;
        if old != *value {
            applied.push(key.clone());
        }
    }
    let cfg = serde_json::from_value(merged).unwrap();
    let restart = applied
        .iter()
        .any(|key| RESTART_CLASS.contains(&key.as_str()));
    Ok((cfg, SettingsSetResult { applied, restart }))
}

pub fn get(daemon: &Daemon) -> ApiResult<Settings> {
    let _transaction = daemon.settings.transaction.lock().unwrap();
    let mut cfg = Config::load().map_err(engine_error)?;
    cfg.idle_exit_secs = daemon.idle_exit_secs();
    if daemon.no_notify {
        cfg.notify = false;
    }
    Ok(view(&cfg))
}

pub fn set(daemon: &Daemon, patch: &Map<String, Value>) -> ApiResult<SettingsSetResult> {
    let _transaction = daemon.settings.transaction.lock().unwrap();
    let previous = Config::load().map_err(engine_error)?;
    let (cfg, mut result) = prepare(&previous, patch)?;
    // A serve --idle-exit-secs override is effective process policy. An explicit
    // API write must replace it even if the durable file already has that value.
    if patch.contains_key("idle_exit_secs")
        && cfg.idle_exit_secs != daemon.idle_exit_secs()
        && !result.applied.iter().any(|key| key == "idle_exit_secs")
    {
        result.applied.push("idle_exit_secs".into());
        result.applied.sort();
    }
    if result.applied.is_empty() {
        return Ok(result);
    }
    if let Err(failure) = cfg.save() {
        previous.save().map_err(engine_error)?;
        return Err(engine_error(failure));
    }
    if let Err(failure) = daemon.apply_settings(&cfg) {
        // A failed apply is never acknowledged as applied. Restore both the
        // durable policy and the previous session policy before returning.
        previous
            .save()
            .and_then(|()| daemon.apply_settings(&previous))
            .map_err(engine_error)?;
        return Err(engine_error(failure));
    }
    if patch.contains_key("idle_exit_secs") {
        daemon.policy.lock().unwrap().replace_delay(cfg.idle_exit_secs, std::time::Instant::now());
    }
    daemon.settings.generation.fetch_add(1, Ordering::Release);
    if result.restart {
        daemon.lifecycle.request_restart(RestartReason::Settings);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
