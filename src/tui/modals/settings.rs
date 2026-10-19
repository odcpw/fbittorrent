//! Existing modals settings behavior.
use super::*;

pub(super) struct SettingsModal {
    pub(super) cfg: Config,
    pub(super) list: RefCell<ListState>,
    pub(super) connections: Vec<Option<String>>,
    pub(super) error: String,
    pub(super) saving: bool,
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
        self.settings = Some(SettingsModal { cfg, connections, list: RefCell::new(list), error: String::new(), saving:false });
    }

    pub(super) fn on_settings_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Modal(actions::ModalKind::Settings), key)
    }

    pub(super) fn on_settings_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        if intent == actions::Intent::Cancel { self.settings = None; return Cmd::none(); }
        if intent == actions::Intent::CloseUi { return self.close_ui(); }
        if self.settings.as_ref().is_some_and(|modal|modal.saving) {return Cmd::none();}
        if intent == actions::Intent::Primary {
            let values=self.settings.as_ref().unwrap().cfg.clone();
            self.settings.as_mut().unwrap().saving=true;
            let presence=self.presence.clone();let previous=self.cfg.clone();
            return Cmd::task(move|| {
                let result=(||->Result<Config> {
                    let mut latest=Config::load()?;
                    let queue_changed=latest.max_active_downloads!=values.max_active_downloads;
                    let reopen=latest.peer_encryption!=values.peer_encryption || latest.network_interface!=values.network_interface;
                    latest.max_active_downloads=values.max_active_downloads;
                    latest.peer_encryption=values.peer_encryption;latest.network_interface=values.network_interface;
                    latest.reduce_motion=values.reduce_motion;latest.validate()?;
                    if reopen {
                        latest.save()?;
                        presence.ok_or_else(||anyhow::anyhow!("fbittorrent stopped"))?
                            .request("app.restart",serde_json::json!({"scope":"engine"}))?;
                    } else {
                        if queue_changed {Client::from_config(&previous,presence.clone()).set_queue_limit(latest.max_active_downloads)?;}
                        latest.save()?;
                    }
                    Ok(latest)
                })();
                Msg::SettingsSaved(result.map_err(|error|format!("{error:#}")))
            });
        }
        let modal = self.settings.as_mut().unwrap();
        let selected = modal.list.borrow().selected().unwrap_or(0);
        match intent {
            actions::Intent::Down => modal.list.borrow_mut().select(Some((selected + 1) % 4)),
            actions::Intent::Up => modal.list.borrow_mut().select(Some((selected + 3) % 4)),
            actions::Intent::Left | actions::Intent::Right => {
                let forward = !matches!(intent, actions::Intent::Left);
                match selected {
                    0 => modal.cfg.max_active_downloads = if forward { modal.cfg.max_active_downloads.saturating_add(1) } else { modal.cfg.max_active_downloads.saturating_sub(1).max(1) },
                    1 => {
                        let policies = ["prefer", "require", "off"];
                        let index = policies.iter().position(|p| *p == modal.cfg.peer_encryption).unwrap_or(0);
                        modal.cfg.peer_encryption = policies[(index + if forward { 1 } else { 2 }) % 3].into();
                    }
                    2 => {
                        let index = modal.connections.iter().position(|v| *v == modal.cfg.network_interface).unwrap_or(0);
                        let count = modal.connections.len();
                        modal.cfg.network_interface = modal.connections[(index + if forward { 1 } else { count - 1 }) % count].clone();
                    }
                    _ => modal.cfg.reduce_motion = !modal.cfg.reduce_motion,
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
        let items = vec![li(Line::raw(format!("Simultaneous downloads   {}", modal.cfg.max_active_downloads))), li(Line::raw(format!("Peer encryption          {encryption}"))), li(Line::raw(format!("VPN connection           {vpn}"))), li(Line::raw(format!("Reduce motion            {}",if modal.cfg.reduce_motion { "On" } else { "Off" })))];
        let rows = Rect::new(inner.x, inner.y, inner.width, inner.height.min(4));
        StatefulWidget::render(&List::new(items).style(bg).highlight_style(th.selected()).highlight_symbol("▌"), rows, frame, &mut modal.list.borrow_mut());
        let description = if !modal.error.is_empty() { modal.error.clone() } else {
            format!("↑/↓ choose · ←/→ change · Enter save · Esc cancel\n\nExtra downloads wait their turn. Encryption protects peer traffic; your IP remains visible.\n\nA required VPN blocks torrent transfers when disconnected. Feed and search websites use your usual connection.\n\n{}\nChanging encryption or VPN reopens the app's engine.", crate::network_app::state(&modal.cfg).message)
        };
        let remaining = inner.height.saturating_sub(4);
        Paragraph::new(description).style(if modal.error.is_empty() { bg.fg(th.dim) } else { bg.fg(th.red) }).wrap(WrapMode::Word).render(Rect::new(inner.x, inner.y.saturating_add(4), inner.width, remaining), frame);
    }
}
