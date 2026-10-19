//! Existing screens torrents behavior.
use super::*;

impl TuiApp {
    pub(super) fn selected_torrent(&self) -> Option<&Torrent> {
        if let Some(details) = &self.details {
            self.torrents.iter().find(|t| t.hash == details.hash)
        } else { self.tor_list.borrow().selected().and_then(|i| self.torrents.get(i)) }
    }

    #[cfg(test)]
    pub(super) fn on_torrents_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Torrents), key)
    }

    pub(super) fn on_torrents_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let n = self.torrents.len();
        if matches!(intent,actions::Intent::Pause|actions::Intent::Seed|actions::Intent::Remove|actions::Intent::DeleteFiles)
            &&self.selected_torrent().is_some_and(|torrent|self.store.pending_for_key(&torrent.hash).is_some()) {
            self.set_status("Waiting for confirmation",false);return Cmd::none();
        }
        match intent {
            actions::Intent::Down => { if n > 0 { self.tor_list.borrow_mut().select_next(n); } }
            actions::Intent::Up => self.tor_list.borrow_mut().select_previous(),
            actions::Intent::First => self.tor_list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            actions::Intent::Last => self.tor_list.borrow_mut().select(n.checked_sub(1)),
            actions::Intent::Pause => {
                if let Some(t) = self.selected_torrent().cloned() {
                    let stopped = self.torrent_paused(&t);
                    return self.action(if stopped { "started" } else { "stopped" }, move |c| if stopped { c.start(&[t.hash]) } else { c.stop(&[t.hash]) });
                }
            }
            actions::Intent::Seed => if let Some(t) = self.selected_torrent().cloned() {
                let allow = !self.store.torrents.get(&t.hash).map_or_else(||self.seed_allowed(&t), |row|row.upload_enabled);
                return self.action(if allow { "Upload: on" } else { "Upload: off" },
                    move |c| if allow { c.allow_seeding(&[t.hash]) } else { c.deny_seeding(&[t.hash]) });
            },
            actions::Intent::Remove => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: false },
                    &format!("Remove '{}' (keep files)? y/N", short(&t.name, 50)), true);
            },
            actions::Intent::DeleteFiles => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: true },
                    &format!("DELETE '{}' AND ITS FILES? y/N", short(&t.name, 50)), true);
            },
            actions::Intent::Open => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("open requested",move|client|client.open(&t));
            },
            actions::Intent::Imdb => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.open_imdb(name, None);
            },
            actions::Intent::Wiki => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.open_wiki(name, None);
            },
            actions::Intent::Follow => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.follow(name, None);
            },
            actions::Intent::Primary => if let Some(t) = self.selected_torrent().cloned() {
                return self.open_details(&t);
            },
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_torrents(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = torrent_columns(area);
        let queued = self.torrents.iter().filter(|t| t.state == "queuedDL").count();
        let (network_ready,network_message) = self.network_banner();
        let title = if !network_ready { format!(" {} ", network_message) }
            else if queued > 0 { format!(" Torrents ({}) · {queued} queued ", self.torrents.len()) }
            else { format!(" Torrents ({}) ", self.torrents.len()) };
        let block = Block::bordered().padding(Sides::all(0)).border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);

        if !self.online && !(self.recovery.active && !self.torrents.is_empty()) {
            let msg = if self.daemon_stopped {"fbittorrent stopped\n\n(Enter) Start the daemon".into()} else if let Some(notice)=self.connection_banner() {notice.into_owned()} else {format!("Torrent engine is not reachable.\n{}\n\n(R)etry", self.backend_error)};
            Paragraph::new(msg).style(th.err()).wrap(WrapMode::Word).render(pad(inner), frame);
        } else if self.torrents.is_empty() {
            let text = if !network_ready { format!("{}\n\nConnect your VPN to start transfers.\n(F6) Download and VPN settings\n\nYour saved downloads stay here.", network_message) }
                else { "No torrents.\n\n(A)dd a magnet link or URL\n(C)lipboard: add a copied link\n(2) Search".into() };
            Paragraph::new(text)
                .style(th.dimmed()).render(pad(inner), frame);
        } else {
            self.view_torrent_table(inner,frame);
        }

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" Details ");
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        if let Some(t) = self.selected_torrent() {
            self.view_torrent_details(t, pad(inner), frame);
        }
    }

    pub(super) fn view_torrent_details(&self, t: &Torrent, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let kv = |k: &str, v: String| Line::from_spans([Span::styled(format!("{k:<10}"), th.dimmed()), Span::styled(v, th.text())]);
        let flags = "streaming priority automatic".to_string();
        let metrics = format::Metrics {legacy:t,api:self.store.torrents.get(&t.hash)};
        let mut lines = vec![
            Line::styled(t.name.clone(), th.title()),
            Line::raw(""),
            kv("state", self.store.pending_for_key(&t.hash).map_or_else(||format!("{} · {}",format::state_cell(t,self.store.torrents.get(&t.hash)).trim_end(),format::state_word(t,self.store.torrents.get(&t.hash)).to_lowercase()),|op|format!("{}…",remote::mutation_verb(op)))),
            kv("progress", metrics.progress()),
            kv("speed", metrics.speed()),
            kv("eta", metrics.eta()),
            kv("peers", metrics.peers()),
            kv("ratio", metrics.ratio()),
            kv("flags", flags),
            kv("upload", metrics.upload()),
            kv("path", t.save_path.clone()),
            kv("added", ago(t.added_on)),
        ];
        if let Some(message) = &t.error_message {
            lines.insert(3, Line::styled(format!("error     {message}"), th.err()));
        }
        let head_h = lines.len() as u16;
        let mut text = Text::from_lines(lines.drain(..));
        text.push_line(Line::raw(""));
        let rows = Flex::vertical().constraints([Constraint::Fixed(head_h + 1), Constraint::Fill]).split(area);
        Paragraph::new(text).wrap(WrapMode::None).render(rows[0], frame);
        self.view_imdb_card(&t.name, rows[1], frame);
    }
}
