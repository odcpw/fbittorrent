//! Existing modals settings behavior.
use super::*;

pub(super) struct SettingsModal {
    pub(super) cfg: Config,
    pub(super) list: RefCell<ListState>,
    pub(super) connections: Vec<Option<String>>,
    pub(super) error: String,
}

impl TuiApp {
    pub(super) fn open_settings(&mut self) {
        let cfg = Config::load().unwrap_or_else(|_| self.cfg.clone());
        let mut connections = vec![Some("proton".into()), None];
        for name in crate::network_app::interfaces().unwrap_or_default() {
            if name != "lo" && !connections.contains(&Some(name.clone())) { connections.push(Some(name)); }
        }
        if !connections.contains(&cfg.network_interface) { connections.push(cfg.network_interface.clone()); }
        let mut list = ListState::default();
        list.select(Some(0));
        self.settings = Some(SettingsModal { cfg, connections, list: RefCell::new(list), error: String::new() });
    }

    pub(super) fn on_settings_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        if key.code == KeyCode::Escape { self.settings = None; return Cmd::none(); }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(Modifiers::CTRL) { self.lifecycle.request_stop(StopReason::Quit); return Cmd::quit(); }
        if key.code == KeyCode::Enter {
            let values = self.settings.as_ref().unwrap().cfg.clone();
            let result = (|| -> Result<Config> {
                let mut latest = Config::load()?;
                let reopen = latest.peer_encryption != values.peer_encryption || latest.network_interface != values.network_interface;
                latest.max_active_downloads = values.max_active_downloads;
                latest.peer_encryption = values.peer_encryption;
                latest.network_interface = values.network_interface;
                latest.validate()?;
                if reopen {
                    latest.save()?;
                    self.lifecycle.request_restart(RestartReason::Settings);
                } else {
                    Client::from_config(&self.cfg).set_queue_limit(latest.max_active_downloads)?;
                }
                Ok(latest)
            })();
            return match result {
                Ok(cfg) => {
                    self.cfg = cfg;
                    self.settings = None;
                    if self.lifecycle.pending_request().is_some() { Cmd::quit() }
                    else { self.set_status("download settings saved", false); self.poll() }
                }
                Err(error) => { self.settings.as_mut().unwrap().error = format!("{error:#}"); Cmd::none() }
            };
        }
        let modal = self.settings.as_mut().unwrap();
        let selected = modal.list.borrow().selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => modal.list.borrow_mut().select(Some((selected + 1) % 3)),
            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => modal.list.borrow_mut().select(Some((selected + 2) % 3)),
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Char(' ') => {
                let forward = !matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
                match selected {
                    0 => modal.cfg.max_active_downloads = if forward { modal.cfg.max_active_downloads.saturating_add(1) } else { modal.cfg.max_active_downloads.saturating_sub(1).max(1) },
                    1 => {
                        let policies = ["prefer", "require", "off"];
                        let index = policies.iter().position(|p| *p == modal.cfg.peer_encryption).unwrap_or(0);
                        modal.cfg.peer_encryption = policies[(index + if forward { 1 } else { 2 }) % 3].into();
                    }
                    _ => {
                        let index = modal.connections.iter().position(|v| *v == modal.cfg.network_interface).unwrap_or(0);
                        let count = modal.connections.len();
                        modal.cfg.network_interface = modal.connections[(index + if forward { 1 } else { count - 1 }) % count].clone();
                    }
                }
                modal.error.clear();
            }
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_settings(&self, frame: &mut Frame) {
        let Some(modal) = &self.settings else { return };
        let area = settings_area(frame.bounds());
        let th = &self.theme;
        let bg = Style::new().bg(th.selection).fg(th.fg);
        Paragraph::new(Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), bg)))).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).padding(Sides::horizontal(1)).title(" Download settings ");
        let inner = block.inner(area);
        block.render(area, frame);
        let encryption = match modal.cfg.peer_encryption.as_str() { "require" => "Require encryption", "off" => "Off", _ => "Prefer encryption" };
        let vpn = match modal.cfg.network_interface.as_deref() { None => "Off · ordinary connection".into(), Some("proton") => "ProtonVPN · required".into(), Some(name) => format!("{name} · required") };
        let items = vec![li(Line::raw(format!("Simultaneous downloads   {}", modal.cfg.max_active_downloads))), li(Line::raw(format!("Peer encryption          {encryption}"))), li(Line::raw(format!("VPN connection           {vpn}")))];
        let rows = Rect::new(inner.x, inner.y, inner.width, inner.height.min(3));
        StatefulWidget::render(&List::new(items).style(bg).highlight_style(th.selected()).highlight_symbol("▌"), rows, frame, &mut modal.list.borrow_mut());
        let description = if !modal.error.is_empty() { modal.error.clone() } else {
            format!("↑/↓ choose · ←/→ change · Enter save · Esc cancel\n\nExtra downloads wait their turn. Encryption protects peer traffic; your IP remains visible.\n\nA required VPN blocks torrent transfers when disconnected. Feed and search websites use your usual connection.\n\n{}\nChanging encryption or VPN reopens the app's engine.", crate::network_app::state(&modal.cfg).message)
        };
        let remaining = inner.height.saturating_sub(3);
        Paragraph::new(description).style(if modal.error.is_empty() { bg.fg(th.dim) } else { bg.fg(th.red) }).wrap(WrapMode::Word).render(Rect::new(inner.x, inner.y.saturating_add(3), inner.width, remaining), frame);
    }
}
