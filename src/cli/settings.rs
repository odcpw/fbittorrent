//! Settings never start a daemon solely to save configuration.
use crate::{agent::Code, api, config::Config};
use anyhow::Result;
use serde_json::{json, Map, Value};

#[allow(clippy::too_many_arguments)]
pub fn run(
    cfg: &Config,
    json_mode: bool,
    max: Option<usize>,
    encryption: Option<String>,
    vpn: Option<String>,
    interfaces: bool,
    idle: Option<u64>,
    handler: Option<String>,
    clipboard: Option<bool>,
    port: Option<u16>,
) -> Result<Code> {
    anyhow::ensure!(cfg.native(), "these settings require the native engine");
    let mut patch = Map::new();
    if let Some(v) = max {
        patch.insert("max_active_downloads".into(), json!(v));
    }
    if let Some(v) = encryption {
        patch.insert("peer_encryption".into(), json!(v));
    }
    if let Some(v) = vpn {
        patch.insert(
            "network_interface".into(),
            if v == "off" { Value::Null } else { json!(v) },
        );
    }
    if let Some(v) = idle {
        patch.insert("idle_exit_secs".into(), json!(v));
    }
    if let Some(v) = handler {
        patch.insert("handler".into(), json!(v));
    }
    if let Some(v) = clipboard {
        patch.insert("clipboard_watch".into(), json!(v));
    }
    if let Some(v) = port {
        patch.insert("peer_port".into(), json!(v));
    }
    let (settings, ack, running) = match api::client::Client::connect(cfg, json_mode) {
        Ok(client) => {
            let ack = if patch.is_empty() {
                json!({"applied":[],"restart":false})
            } else {
                client.call("app.settings.set", json!(patch))?
            };
            (client.call("app.settings.get", json!({}))?, ack, true)
        }
        Err(error) => {
            if !error.downcast_ref::<std::io::Error>().is_some_and(|io|matches!(io.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::ConnectionRefused)) {return Err(error);}
            let (_lock, _) = super::offline::locked()?;
            let current = Config::load()?;
            let (changed, ack) =
                api::ops::settings::prepare(&current, &patch).map_err(api::errors::app_error)?;
            if !ack.applied.is_empty() {
                changed.save()?;
            }
            (
                serde_json::to_value(api::ops::settings::view(&changed))?,
                serde_json::to_value(ack)?,
                false,
            )
        }
    };
    let available = if interfaces {
        Some(crate::network_app::interfaces()?)
    } else {
        None
    };
    let mut output = settings.clone();
    // Retain contract 1.x's existing CLI keys; API unknowns remain absent.
    output["network_interface"] = settings
        .get("network_interface")
        .cloned()
        .unwrap_or(Value::Null);
    output["ok"] = json!(true);
    output["running"] = json!(running);
    output["applied"] = ack["applied"].clone();
    output["restart"] = ack["restart"].clone();
    let current = Config::load()?;
    output["network"] = json!(crate::network_app::state(&current));
    output["interfaces"] = json!(available);
    crate::emit(json_mode, output, || {
        println!(
            "Simultaneous downloads: {}",
            settings["max_active_downloads"]
        );
        println!(
            "Peer encryption: {}",
            settings["peer_encryption"].as_str().unwrap()
        );
        println!("VPN: {}", crate::network_app::state(&current).message);
        if let Some(names) = available {
            println!("Connections: {}", names.join(", "));
        }
    });
    Ok(Code::Ok)
}
