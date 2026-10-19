//! Connection actions, lifetime and quit handling.
use super::*;

pub fn run(cfg: Config, screen: Screen, lifecycle: Lifecycle) -> Result<()> {
    let mut model = TuiApp::with_lifecycle(cfg, screen, Shows::load()?, lifecycle);
    model.pointer_motion_on_start = true;
    // AppBuilder::run exits the process on a signal, skipping the outer engine
    // guard. Program returns instead, so our application owner drains first.
    let mut program = ftui::Program::with_config(model, ftui::ProgramConfig::fullscreen().with_mouse())?;
    let result = program.run();
    signals::event("shutdown_once end (Program::run returned)");
    let signal = program.model().termination_signal;
    drop(program);
    if let Some(signal) = signal { return Err(SignalTermination(signal).into()); }
    result?;
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
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login().and_then(|_| Ok((c.torrents()?, c.transfer_info()?)));
            Msg::Torrents(r.map_err(|e| e.to_string()))
        })
    }

    pub(super) fn action<F>(&self, label: impl Into<String>, f: F) -> Cmd<Msg>
    where F: FnOnce(&Client) -> anyhow::Result<()> + Send + 'static {
        let cfg = self.cfg.clone();
        let label = label.into();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
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

    pub(super) fn on_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        if k.kind == KeyEventKind::Release { return Cmd::none(); }
        self.clear_pointer_hover();
        if self.settings.is_some() { return self.on_settings_key(k); }
        if self.show_help {
            let max_scroll = self.help_max_scroll();
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll = self.help_scroll.saturating_add(1).min(max_scroll),
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10).min(max_scroll),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => self.help_scroll = max_scroll,
                _ => self.show_help = false,
            }
            return Cmd::none();
        }
        if self.prompt.is_some() { return self.on_prompt_key(k); }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(Modifiers::CTRL) { self.lifecycle.request_stop(StopReason::Quit); return Cmd::quit(); }
        if self.files.is_some() { return self.on_files_key(k); }
        if self.providers.is_some() { return self.on_providers_key(k); }
        if k.code == KeyCode::F(6) && self.cfg.native() { self.open_settings(); return Cmd::none(); }

        // The search box eats printable keys while focused.
        if self.screen == Screen::Search && self.search_focus_input {
            match k.code {
                KeyCode::Escape => { self.search_focus_input = false; self.search_input.set_focused(false); return Cmd::none(); }
                KeyCode::Enter => {
                    if self.search_input.value().trim().is_empty() {
                        self.search_focus_input = false; self.search_input.set_focused(false);
                        self.search_browse = true;
                        if self.browse_list.borrow().selected().is_none() { self.browse_list.borrow_mut().select(Some(0)); }
                        return Cmd::none();
                    }
                    return self.start_search();
                }
                KeyCode::Tab | KeyCode::BackTab => {}
                KeyCode::Down if !self.search_results.is_empty() => {
                    self.search_focus_input = false; self.search_input.set_focused(false);
                    if self.search_list.borrow().selected().is_none() { self.search_list.borrow_mut().select(Some(0)); }
                    return Cmd::none();
                }
                _ => { self.search_input.handle_event(&Event::Key(k)); return Cmd::none(); }
            }
        }

        match k.code {
            KeyCode::Char('q') => { self.lifecycle.request_stop(StopReason::Quit); return Cmd::quit(); }
            KeyCode::Char('Q') => {
                self.lifecycle.request_stop(StopReason::Quit);
                if !self.cfg.native() { let _ = crate::backend::stop(&self.cfg); }
                return Cmd::quit();
            }
            KeyCode::Char('?') => { self.show_help = true; self.help_scroll = 0; return Cmd::none(); }
            KeyCode::Char('1') => return self.set_screen(Screen::Torrents),
            KeyCode::Char('2') => return self.set_screen(Screen::Search),
            KeyCode::Char('3') => return self.set_screen(Screen::Shows),
            KeyCode::Char('4') => return self.set_screen(Screen::Rss),
            KeyCode::Tab => return self.set_screen(Screen::from_index(self.screen.index() + 1)),
            KeyCode::BackTab => return self.set_screen(Screen::from_index(self.screen.index() + 3)),
            KeyCode::Char('a') => { self.open_prompt(PromptKind::AddMagnet, "Add magnet / URL", false); return Cmd::none(); }
            KeyCode::Char('c') => {
                let opts = self.add_opts();
                return self.action("added from clipboard", move |c| {
                    let out = std::process::Command::new("wl-paste").arg("--no-newline").output()?;
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !(text.starts_with("magnet:") || text.starts_with("http")) { anyhow::bail!("clipboard holds no magnet link or URL"); }
                    c.add_url(&text, &opts)
                });
            }
            KeyCode::Char('r') if !matches!(self.screen, Screen::Rss | Screen::Shows) => {
                if !self.online {
                    // Bring the backend back if it went away under us.
                    let cfg = self.cfg.clone();
                    let up = Cmd::task(move || {
                        let c = Client::from_config(&cfg);
                        match crate::backend::ensure_running(&cfg, &c) {
                            Ok(_) => Msg::ActionDone(Ok("torrent engine ready".into())),
                            Err(e) => Msg::ActionDone(Err(e.to_string())),
                        }
                    });
                    return up;
                }
                return self.poll();
            }
            _ => {}
        }
        match self.screen {
            Screen::Torrents => self.on_torrents_key(k),
            Screen::Search => self.on_search_key(k),
            Screen::Rss => self.on_rss_key(k),
            Screen::Shows => self.on_shows_key(k),
        }
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


