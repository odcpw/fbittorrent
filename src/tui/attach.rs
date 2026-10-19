//! Attachment and UI lifetime are independent of the daemon owner's lifetime.
use super::*;
mod notices;
mod presence;
mod recovery;
pub(super) use recovery::{Failure, Notice, Recovery};
mod wire;
pub(super) use presence::{Presence, Update as ApiUpdate};
#[cfg(test)]
mod tests;

pub(super) fn connect(cfg: &Config) -> Result<Arc<Presence>> {
    // An existing owner can answer hello while its engine is busy checking files.
    // Do not turn a slow legacy version probe into an attempted manager start.
    match Presence::connect() {
        Ok(presence) => return Ok(presence),
        Err(error) if error.downcast_ref::<wire::RemoteError>().is_some_and(|error|error.0.code==crate::api::ErrorCode::ProtoUnsupported) => return Err(error),
        Err(_) => {}
    }
    ensure(cfg)?;
    Presence::connect()
}

fn ensure(cfg: &Config) -> Result<()> {
    let mut explicit = cfg.clone();
    explicit.autostart = true;
    crate::api::client::ensure_daemon(&explicit, crate::backend::start_prevented())?;
    Ok(())
}

pub(super) fn after_detach(show_notice: bool) {
    if !show_notice {
        return;
    }
    let result = dirs::state_dir()
        .ok_or_else(|| anyhow::anyhow!("no state directory"))
        .and_then(|directory| notices::first_quit(&directory.join("fbittorrent")));
    match result {
        Ok(false) => {}
        Ok(true) => println!("{}", notices::QUIT_NOTICE),
        Err(error) => eprintln!(
            "{}\nCould not save the notice preference: {error}",
            notices::QUIT_NOTICE
        ),
    }
}

impl TuiApp {
    pub(super) fn close_ui(&mut self) -> Cmd<Msg> {
        self.quit_notice = self.cfg.native();
        Cmd::quit()
    }
    pub(super) fn request_shutdown(&mut self) -> Cmd<Msg> {
        let active = self
            .torrents
            .iter()
            .filter(|torrent| torrent.is_active() || torrent.is_queued() || torrent.is_seeding())
            .count();
        if active > 0 {
            self.open_prompt(
                PromptKind::Shutdown,
                &format!("Stop fbittorrent? {active} downloads pause until it starts again. y/N"),
                true,
            );
            return Cmd::none();
        }
        self.shutdown_daemon()
    }
    pub(super) fn shutdown_daemon(&mut self) -> Cmd<Msg> {
        let presence = self.presence.clone();
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let result = if cfg.native() {
                presence
                    .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))
                    .and_then(|connection| {
                        connection.request("app.shutdown", serde_json::json!({"reason":"user"}))
                    })
                    .map(|_| ())
            } else {
                crate::backend::stop(&cfg)
            };
            Msg::OwnerShutdown(result.map_err(|error| error.to_string()))
        })
    }
    pub(super) fn start_daemon(&mut self) -> Cmd<Msg> {
        if !self.recovery.begin(std::time::Instant::now()) {return Cmd::none();}
        self.daemon_stopped=false;
        let cfg = self.cfg.clone();
        Cmd::task(move || Msg::Attached(connect(&cfg).map_err(Failure::capture)))
    }
}

impl TuiApp {
    pub(in crate::tui) fn connection_lost(&mut self, intentional: bool) {
        if !self.recovery.lost(std::time::Instant::now(),intentional) {return;}
        let pending = !self.store.pending.is_empty();
        self.store.disconnected();
        self.online=false;
        self.poll_inflight=false;
        self.daemon_stopped=self.recovery.stopped(std::time::Instant::now());
        if pending {self.set_status("Result unknown; the list will tell.",true);}
    }
    pub(in crate::tui) fn recovery_tick(&mut self) -> Cmd<Msg> {
        if !self.cfg.native() {return Cmd::none();}
        if self.presence.as_ref().is_some_and(|connection|!connection.alive()) {
            self.connection_lost(false);
        }
        if !self.recovery.active {return Cmd::none();}
        let now=std::time::Instant::now();
        self.daemon_stopped=self.recovery.stopped(now);
        if self.recovery.retry_due(now) {
            // Reconnect is a read: never call ensure_daemon or retry a mutation here.
            return Cmd::task(||Msg::Attached(Presence::connect().map_err(Failure::capture)));
        }
        Cmd::none()
    }
    pub(in crate::tui) fn connection_banner(&self) -> Option<std::borrow::Cow<'static,str>> {
        self.recovery.notice(std::time::Instant::now()).map(Notice::text)
    }
}
