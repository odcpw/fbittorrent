//! Existing modals settings behavior.
use super::*;

pub(super) struct SettingsModal {
    pub(super) cfg: Config,
    pub(super) original:Config,
    pub(super) list: RefCell<ListState>,
    pub(super) connections: Vec<Option<String>>,
    pub(super) error: String,
    pub(super) saving: bool,
}

impl TuiApp {
    pub(super) fn open_settings(&mut self)->Cmd<Msg> {
        if self.settings_loading{return Cmd::none();}
        if let Some(connection)=self.presence.clone(){
            self.settings_loading=true;
            return Cmd::task(move||Msg::SettingsOpened{connection:connection.id,result:connection.request("app.settings.get",serde_json::json!({})).and_then(|v|Ok(serde_json::from_value(v)?)).map_err(|e|e.to_string())});
        }
        self.show_settings(self.cfg.clone());Cmd::none()
    }
    pub(super) fn show_settings(&mut self,cfg:Config) {
        let mut connections = vec![Some("proton".into()), None];
        for name in crate::network_app::interfaces().unwrap_or_default() {
            if name != "lo" && !connections.contains(&Some(name.clone())) { connections.push(Some(name)); }
        }
        if !connections.contains(&cfg.network_interface) { connections.push(cfg.network_interface.clone()); }
        let mut list = ListState::default();
        list.select(Some(0));
        self.settings = Some(SettingsModal { original:cfg.clone(),cfg, connections, list: RefCell::new(list), error: String::new(), saving:false });
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
            let patch=settings_fields::settings_patch(&self.settings.as_ref().unwrap().original,&values);
            let presence=self.presence.clone();
            return Cmd::task(move|| {
                let result=(||->Result<Config> {
                    let connection=presence.ok_or_else(||anyhow::anyhow!("fbittorrent stopped"))?;
                    let _:crate::api::SettingsSetResult=serde_json::from_value(connection.request("app.settings.set",patch)?)?;
                    // app.settings.set owns the restart; sending app.restart here can restart twice.
                    let settings:crate::api::Settings=serde_json::from_value(connection.request("app.settings.get",serde_json::json!({}))?)?;
                    let mut cfg=values;settings_fields::native_settings(&mut cfg,&settings)?;Ok(cfg)
                })();
                Msg::SettingsSaved(result.map_err(|error|format!("{error:#}")))
            });
        }
        if matches!(intent,actions::Intent::Left|actions::Intent::Right){
            let selected=self.settings.as_ref().unwrap().list.borrow().selected().unwrap_or(0);
            if selected==settings_fields::PORT_ROW{return self.edit_peer_port();}
            if (4..=7).contains(&selected){return self.edit_limit(selected-4);}
            if selected==settings_fields::DESTINATION_ROW{return self.edit_destination();}
            if selected==settings_fields::IDLE_ROW{return self.edit_idle_exit();}
        }

        let modal = self.settings.as_mut().unwrap();
        let selected = modal.list.borrow().selected().unwrap_or(0);
        match intent {
            actions::Intent::Down => modal.list.borrow_mut().select(Some((selected + 1) % settings_fields::SETTINGS_ROWS)),
            actions::Intent::Up => modal.list.borrow_mut().select(Some((selected + settings_fields::SETTINGS_ROWS - 1) % settings_fields::SETTINGS_ROWS)),
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
                    3 => modal.cfg.reduce_motion = !modal.cfg.reduce_motion,
                    settings_fields::SEED_ROW=>modal.cfg.seed_by_default=!modal.cfg.seed_by_default,
                    settings_fields::CLIPBOARD_ROW=>modal.cfg.clipboard_watch=!modal.cfg.clipboard_watch,
                    _=>{},
                }
                modal.error.clear();
            }
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_settings(&self, frame: &mut Frame) {
        let Some(modal) = &self.settings else { return };
        self.rects.borrow_mut().overlay();
        let area = settings_area(frame.bounds());
        let th = &self.theme;
        let bg = Style::new().bg(th.selection).fg(th.fg);
        Paragraph::new(Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), bg)))).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).padding(Sides::horizontal(1)).title(" Download settings ");
        let inner = block.inner(area);
        block.render(area, frame);
        let rows=Rect::new(inner.x,inner.y,inner.width,inner.height.min(settings_fields::SETTINGS_ROWS as u16));
        self.view_rows(MouseList::Settings,rows,settings_fields::SETTINGS_ROWS,
            |i|layout::fingerprint(&(i,modal.cfg.max_active_downloads,&modal.cfg.peer_encryption,&modal.cfg.network_interface,modal.cfg.reduce_motion,settings_fields::limit(&modal.cfg,i.saturating_sub(4)),modal.cfg.peer_port,&modal.cfg.save_path,modal.cfg.seed_by_default,modal.cfg.idle_exit_secs,modal.cfg.clipboard_watch)),
            |i|Line::raw(match i {
                0=>format!("Simultaneous downloads   {}",modal.cfg.max_active_downloads),
                1=>format!("Peer encryption          {}",match modal.cfg.peer_encryption.as_str(){"require"=>"Require encryption","off"=>"Off",_=>"Prefer encryption"}),
                2=>format!("VPN connection           {}",match modal.cfg.network_interface.as_deref(){None=>"Off · ordinary connection".into(),Some("proton")=>"ProtonVPN · required".into(),Some(name)=>format!("{name} · required")}),
                3=>format!("Reduce motion            {}",if modal.cfg.reduce_motion{"On"}else{"Off"}),
                settings_fields::PORT_ROW=>format!("Peer port                {}",if modal.cfg.peer_port==0{"Automatic (0)".into()}else{modal.cfg.peer_port.to_string()}),
                settings_fields::DESTINATION_ROW=>format!("Download destination     {}",modal.cfg.save_path.as_deref().unwrap_or("Downloads (default)")),
                settings_fields::SEED_ROW=>format!("Seed by default          {}",if modal.cfg.seed_by_default {"On"}else{"Off"}),
                settings_fields::IDLE_ROW=>format!("Idle exit                {}",if modal.cfg.idle_exit_secs==0 {"Disabled (0)".into()}else{format!("{} seconds",modal.cfg.idle_exit_secs)}),
                settings_fields::CLIPBOARD_ROW=>format!("Clipboard offers         {}",if modal.cfg.clipboard_watch {"On"}else{"Off"}),
                _=>settings_fields::limit_label(&modal.cfg,i-4),
            }),frame);
        let description = if !modal.error.is_empty() { th.error_message(&modal.error) } else {
            format!("↑/↓ choose · ←/→ change/edit · Enter save · Esc cancel\nLimits: bytes/s; K/M suffixes; empty = unlimited.\n\nExtra downloads wait. Encryption leaves your IP visible.\nA required VPN blocks transfers while disconnected.\n{}\nChanging encryption, VPN or port reopens the engine.", self.store.network.as_ref().map(|n|n.message.as_str()).unwrap_or("Save to apply draft settings."))
        };
        let remaining = inner.height.saturating_sub(settings_fields::SETTINGS_ROWS as u16);
        Paragraph::new(description).style(if modal.error.is_empty() { bg.fg(th.dim) } else { th.err().bg(th.selection) }).wrap(WrapMode::Word).render(Rect::new(inner.x, inner.y.saturating_add(settings_fields::SETTINGS_ROWS as u16), inner.width, remaining), frame);
    }
}
