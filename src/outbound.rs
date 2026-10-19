//! App-owned outbound HTTP: RSS feeds, IMDb/Wikipedia cards, EZTV show checks,
//! browse lists and provider installs. With `network_interface` set (the VPN
//! binding), sockets and DNS use the same bound transport as Torznab; when the
//! interface is unavailable the request is refused, never sent unbound.
use anyhow::Result;

use crate::config::Config;

/// One request context: an agent plus, when bound, the runtime its sockets
/// and resolver run on. Dropping it closes that runtime.
pub(crate) struct Http {
    agent: ureq::Agent,
    bound: Option<crate::search::torznab::bound::Context>,
}

impl Http {
    /// The configured route for app traffic; `what` names it in refusals.
    pub(crate) fn new(config: ureq::config::Config, what: &str) -> Result<Self> {
        Self::for_config(&network_config()?, config, what)
    }
    pub(crate) fn for_config(cfg: &Config, config: ureq::config::Config, what: &str) -> Result<Self> {
        match crate::search::torznab::bound::Context::for_config(cfg, what)? {
            Some(bound) => Ok(Self { agent: bound.agent(config), bound: Some(bound) }),
            None => Ok(Self { agent: ureq::Agent::new_with_config(config), bound: None }),
        }
    }
    pub(crate) fn agent(&self) -> &ureq::Agent { &self.agent }
}

impl Drop for Http {
    fn drop(&mut self) {
        if let Some(bound) = self.bound.take() { let _ = bound.close(); }
    }
}

/// External helpers (Nova Python providers) open their own sockets, which
/// cannot be bound to the VPN interface; they are refused while it is set.
pub(crate) fn unbound_helpers_allowed(what: &str) -> Result<()> {
    anyhow::ensure!(network_config()?.network_interface.is_none(),
        "{what} cannot use the VPN binding (network_interface); use a Torznab indexer or turn the binding off");
    Ok(())
}

#[cfg(not(test))]
fn network_config() -> Result<Config> { Config::load() }

// Unit tests never read the owner's real configuration; a test chooses its route.
#[cfg(test)]
thread_local! { pub(crate) static TEST_CONFIG: std::cell::RefCell<Config> = std::cell::RefCell::new(Config::default()); }
#[cfg(test)]
fn network_config() -> Result<Config> { Ok(TEST_CONFIG.with(|cfg| cfg.borrow().clone())) }

#[cfg(test)]
pub(crate) fn with_interface<T>(name: Option<&str>, run: impl FnOnce() -> T) -> T {
    let previous = TEST_CONFIG.with(|cfg| std::mem::replace(&mut cfg.borrow_mut().network_interface, name.map(Into::into)));
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) { let value = self.0.take(); TEST_CONFIG.with(|cfg| cfg.borrow_mut().network_interface = value); }
    }
    let _restore = Restore(previous);
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn serve_once() -> (String, std::thread::JoinHandle<bool>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/probe", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket.set_nonblocking(false).unwrap();
                    let mut request = [0u8; 2048];
                    let _ = socket.read(&mut request);
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            false
        });
        (url, worker)
    }

    #[test]
    fn missing_vpn_interface_refuses_instead_of_sending_unbound() {
        let (url, worker) = serve_once();
        let config = || ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(2))).build();
        let refused = with_interface(Some("fbmissing0"), || Http::new(config(), "RSS refresh").map(|_| ()));
        assert!(refused.is_err(), "an unavailable interface must refuse the request");
        assert!(with_interface(Some("fbmissing0"), || unbound_helpers_allowed("Nova search providers")).is_err());
        assert!(unbound_helpers_allowed("Nova search providers").is_ok());
        // The ordinary route still works without a binding.
        let http = Http::new(config(), "RSS refresh").unwrap();
        let body = http.agent().get(&url).call().unwrap().body_mut().read_to_string().unwrap();
        assert_eq!(body, "ok");
        assert!(worker.join().unwrap(), "the unbound request reached the local server once");
    }

    #[test]
    fn interface_binding_carries_requests_over_the_bound_transport() {
        // Loopback stands in for the VPN interface: the request must go through
        // the bound resolver/connector and still reach the local server.
        if rqbit_runtime::net::InterfaceBinding::new("lo").is_err() { return; }
        let (url, worker) = serve_once();
        let config = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(3))).build();
        let http = with_interface(Some("lo"), || Http::new(config, "RSS refresh")).unwrap();
        assert!(http.bound.is_some());
        println!("request carried over the bound transport on lo");
        let body = http.agent().get(&url).call().unwrap().body_mut().read_to_string().unwrap();
        assert_eq!(body, "ok");
        assert!(worker.join().unwrap());
    }
}
