//! Connection actions, lifetime and quit handling.
use super::*;

pub fn run(cfg: Config, screen: Screen) -> Result<()> {
    // This guard owns only UI cleanup, never an engine or the instance lock.
    let _terminal=signals::AppGuard::new(());
    let native=cfg.native();
    let mut model = TuiApp::new(cfg, screen, Shows::load()?);
    model.pointer_motion_on_start = true;
    // Program returns on signals, allowing the presence connection and terminal
    // to close while the separate daemon continues its work.
    let mut program = ftui::Program::with_config(model, ftui::ProgramConfig::fullscreen().with_mouse())?;
    let result = program.run();
    if let Some(connection)=program.model().presence.as_ref() {connection.close();}
    signals::event("shutdown_once end (Program::run returned)");
    let signal = program.model().termination_signal;
    let notice = native && program.model().quit_notice;
    drop(program);
    if let Some(signal) = signal { return Err(SignalTermination(signal).into()); }
    result?;
    attach::after_detach(notice);
    Ok(())
}

impl TuiApp {
    pub(super) fn acknowledge_signal(&mut self) -> bool {
        if let Some(signal) = ftui::core::shutdown_signal::pending_termination_signal() {
            signals::event("loop observed signal (model)");
            self.termination_signal.get_or_insert(signal);
            // Preserve the request if terminal teardown blocks after this ack.
            signals::request(signal);
            ftui::core::shutdown_signal::clear_pending_termination_signal();
            signals::event("clear (model)");
            true
        } else { false }
    }

    pub(super) fn poll(&mut self) -> Cmd<Msg> {
        if self.poll_inflight { return Cmd::none(); }
        self.poll_inflight = true;
        let mut cfg = self.cfg.clone();
        if cfg.native() { cfg.timeout_secs=1; }
        let presence = self.presence.clone();
        Cmd::task(move || {
            if cfg.native() {
                let identity=presence.as_ref().map(|connection|connection.id);
                let result=presence.ok_or_else(||anyhow::anyhow!("fbittorrent stopped")).and_then(|connection| {
                    anyhow::ensure!(connection.alive(),"fbittorrent stopped");remote::read(&connection)
                });
                return Msg::ApiRead {connection:identity,result:result.map_err(|error|error.to_string())};
            }
            let c = Client::from_config(&cfg, presence.clone());
            let r = c.login().and_then(|_| Ok((c.torrents()?, c.transfer_info()?)));
            Msg::Torrents(r.map_err(|e| e.to_string()))
        })
    }

    pub(super) fn action<F>(&self, label: impl Into<String>, f: F) -> Cmd<Msg>
    where F: FnOnce(&Client) -> anyhow::Result<()> + Send + 'static {
        let cfg = self.cfg.clone();
        let presence = self.presence.clone();
        let label = label.into();
        Cmd::task(move || {
            let c = Client::from_config(&cfg, presence.clone());
            match c.login().and_then(|_| f(&c)) {
                Ok(()) => Msg::ActionDone(Ok(label)),
                Err(e) => Msg::ActionDone(Err(e.to_string())),
            }
        })
    }

    pub(super) fn add_opts(&self) -> AddOptions {
        AddOptions {
            sequential: self.cfg.native() || self.cfg.sequential,
            first_last: self.cfg.native() || self.cfg.first_last,
            seed: self.cfg.seed_by_default,
            stopped: false,
            save_path: self.cfg.save_path.clone(),
            category: self.cfg.category.clone(),
        }
    }

    /// Re-read Omarchy colors when the theme changed under us.
    pub(super) fn maybe_reload_theme(&mut self) {
        let m = std::fs::metadata(theme::colors_path()).and_then(|m| m.modified()).ok();
        if m != self.theme_mtime {
            self.theme_mtime = m;
            self.theme = Theme::load();
            let value = self.search_input.value().to_string();
            let focused = self.search_input.focused();
            let mut input = TextInput::new()
                .with_placeholder("search torrents, Enter to run")
                .with_style(self.theme.text())
                .with_cursor_style(Style::new().bg(self.theme.accent).fg(self.theme.selection))
                .with_value(value);
            input.set_focused(focused);
            self.search_input = input;
        }
    }

    pub(super) fn on_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch(self, key)
    }

    pub(super) fn set_screen(&mut self, s: Screen) -> Cmd<Msg> {
        self.clear_pointer_hover();
        self.screen = s;
        self.tabs.borrow_mut().select(s.index(), 4);
        // Landing on Search with nothing to browse means the user wants to type.
        if s == Screen::Search && self.search_results.is_empty() { self.search_focus_input = true; }
        self.search_input.set_focused(s == Screen::Search && self.search_focus_input);
        if s == Screen::Rss && !self.rss_loaded { self.rss_loaded = true; return self.load_rss(); }
        Cmd::none()
    }
}


#[derive(Debug)]
pub(crate) struct SignalTermination(pub i32);
impl std::fmt::Display for SignalTermination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "terminated by signal {}", self.0) }
}
impl std::error::Error for SignalTermination {}
