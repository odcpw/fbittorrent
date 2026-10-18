//! App controls and the no-network waiting state around the native engine.
use anyhow::{Context, Result};
use serde::Serialize;
use std::ffi::CString;
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};

use crate::{config::Config, ipc::{InstanceLock, Request, Server}};

static RESTART: AtomicBool = AtomicBool::new(false);
static CLOSE: AtomicBool = AtomicBool::new(false);
static EXPECTED: Mutex<Option<NetworkState>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NetworkState {
    pub selection: Option<String>,
    pub interface: Option<String>,
    pub index: Option<u32>,
    pub ready: bool,
    pub message: String,
}

/// The installed ProtonVPN OpenVPN and WireGuard backends both use proton0.
/// An absent tunnel is never replaced by a different available interface.
pub fn state(cfg: &Config) -> NetworkState {
    let Some(selection) = &cfg.network_interface else {
        return NetworkState { selection: None, interface: None, index: None, ready: true, message: "VPN binding off".into() };
    };
    let name = if selection == "proton" { "proton0" } else { selection.as_str() };
    let index = CString::new(name).ok().map(|name| unsafe { libc::if_nametoindex(name.as_ptr()) }).filter(|index| *index != 0);
    // Query the kernel through the same namespace-aware guard as engine sockets.
    let ready = rqbit_runtime::net::InterfaceBinding::new(name).is_ok();
    let message = if ready { format!("VPN bound: {name}") }
        else if selection == "proton" { "Waiting for ProtonVPN".into() }
        else { format!("Waiting for {name}") };
    NetworkState { selection: Some(selection.clone()), interface: Some(name.into()), index, ready, message }
}

pub fn interfaces() -> Result<Vec<String>> {
    // SAFETY: libc returns a terminated array owned until if_freenameindex.
    let entries = unsafe { libc::if_nameindex() };
    if entries.is_null() { return Err(std::io::Error::last_os_error().into()); }
    let mut names = Vec::new();
    let mut entry = entries;
    // SAFETY: walk only until the sentinel, copying names before freeing.
    unsafe {
        while (*entry).if_index != 0 && !(*entry).if_name.is_null() {
            if let Ok(name) = std::ffi::CStr::from_ptr((*entry).if_name).to_str() { names.push(name.to_owned()); }
            entry = entry.add(1);
        }
        libc::if_freenameindex(entries);
    }
    names.sort();
    Ok(names)
}

pub fn initialize(snapshot: NetworkState) {
    CLOSE.store(false, Ordering::Release);
    RESTART.store(false, Ordering::Release);
    *EXPECTED.lock().unwrap_or_else(|error| error.into_inner()) = Some(snapshot);
}
pub fn request_restart() { RESTART.store(true, Ordering::Release); }
pub fn request_close() { CLOSE.store(true, Ordering::Release); }
pub fn close_requested() -> bool { CLOSE.load(Ordering::Acquire) }
pub fn restart_requested() -> bool { RESTART.load(Ordering::Acquire) }

pub fn check_transition(cfg: &Config) -> bool {
    if restart_requested() { return true; }
    let expected = EXPECTED.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(expected) = expected.as_ref() {
        if expected.selection.is_some() && *expected != state(cfg) {
            request_restart();
            return true;
        }
    }
    false
}

pub fn controls(request: &Request, snapshot: &NetworkState) -> Option<Result<String>> {
    match request.path.as_str() {
        "app/network" => Some(serde_json::to_string(snapshot).map_err(Into::into)),
        "app/settings" => Some(Config::load().and_then(|cfg| Ok(serde_json::json!({
            "max_active_downloads": cfg.max_active_downloads,
            "peer_encryption": cfg.peer_encryption,
            "network_interface": cfg.network_interface,
        }).to_string()))),
        "app/restart" if request.method == "POST" => { request_restart(); Some(Ok("Ok.".into())) },
        "app/shutdown" if request.method == "POST" => { CLOSE.store(true, Ordering::Release); Some(Ok("Ok.".into())) },
        _ => None,
    }
}

pub struct ParkedEngine {
    // Drop the listener and joined handlers before releasing the instance lock.
    _server: Server,
    _instance: InstanceLock,
}
impl ParkedEngine {
    pub fn start(cfg: &Config, snapshot: NetworkState) -> Result<Self> {
        let instance = InstanceLock::acquire()?;
        let server = Server::start(cfg, &instance, move |request| {
            controls(request, &snapshot).or_else(|| crate::engine::dispatch_parked(request))
                .unwrap_or_else(|| Err(anyhow::anyhow!("{}; connect the selected VPN before this action", snapshot.message)))
        }).context("opening the app while waiting for its VPN")?;
        Ok(Self { _server: server, _instance: instance })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_missing_interface_never_uses_available_network() {
        let mut cfg = Config::default();
        cfg.network_interface = Some("fbmissing0".into());
        let state = state(&cfg);
        assert!(!state.ready);
        assert_eq!(state.interface.as_deref(), Some("fbmissing0"));
        assert_eq!(state.index, None);
    }
    #[test]
    fn proton_selection_is_exact_and_excludes_killswitch_devices() {
        let mut cfg = Config::default();
        cfg.network_interface = Some("proton".into());
        assert_eq!(state(&cfg).interface.as_deref(), Some("proton0"));
    }
}
