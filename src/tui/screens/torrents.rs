//! Existing screens torrents behavior.
use super::*;

impl TuiApp {
    pub(super) fn selected_torrent(&self) -> Option<&Torrent> {
        if let Some(hash) = self.files.as_ref().map(|files|files.hash.as_str())
            .or_else(||self.details.as_ref().map(|details|details.hash.as_str())) {
            self.torrents.iter().find(|t| t.hash.as_str() == hash)
        } else { self.tor_list.borrow().selected().and_then(|i| self.torrents.get(i)) }
    }

    #[cfg(test)]
    pub(super) fn on_torrents_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Torrents), key)
    }

    pub(super) fn on_torrents_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let n = self.torrents.len();
        if matches!(intent,actions::Intent::Pause|actions::Intent::Remove|actions::Intent::DeleteFiles)
            &&self.selected_torrent().is_some_and(|torrent|self.store.pending_for_key(torrent.hash.as_str()).is_some()) {
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
                    return self.action(if stopped { "started" } else { "stopped" }, move |c| if stopped { c.start(&[t.hash.to_string()]) } else { c.stop(&[t.hash.to_string()]) });
                }
            }
            actions::Intent::Remove => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.to_string(), files: false },
                    &format!("Remove '{}' (keep files)? y/N", short(&t.name, 50)), true);
            },
            actions::Intent::DeleteFiles => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.to_string(), files: true },
                    &format!("DELETE '{}' AND ITS FILES? y/N", short(&t.name, 50)), true);
            },
            actions::Intent::Open => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("file open requested",move|client|client.open(t.hash.as_str(),None,false));
            },
            actions::Intent::OpenFolder => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("folder open requested",move|client|client.open(t.hash.as_str(),None,true));
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
        let queued = self.torrents.iter().filter(|t| t.state == crate::api::TorrentState::Queued).count();
        let (network_ready,network_message) = self.network_banner();
        let title = if !network_ready { network_message.to_string() }
            else if queued > 0 { format!("Torrents ({}) · {queued} queued", self.torrents.len()) }
            else { format!("Torrents ({})", self.torrents.len()) };
        let gutter = if cols[0].width >= 66 { 3 } else { 1 };
        let page = cols[0].inner(Sides::horizontal(gutter));
        let title_height = if area.height >= 18 { 2 } else { 1 };
        let rows = Flex::vertical().constraints([Constraint::Fixed(title_height), Constraint::Fill]).split(page);
        Paragraph::new(title).style(th.title()).render(rows[0], frame);
        if rows[0].height >= 2 {
            let rule = Rect::new(page.x, page.y + 1, page.width, 1);
            frame.buffer.fill(rule, ftui::Cell::from_char('─').with_fg(th.rule).with_bg(th.bg));
        }
        let inner = rows[1];

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

        let rail = cols[1];
        if !rail.is_empty() {
            frame.buffer.fill(rail, ftui::Cell::from_char(' ').with_bg(th.panel));
            let rule = Rect::new(rail.x, rail.y, 1, rail.height);
            frame.buffer.fill(rule, ftui::Cell::from_char('│').with_fg(th.rule).with_bg(th.panel));
            let content = rail.inner(Sides::horizontal(3));
            let rows = Flex::vertical().constraints([Constraint::Fixed(2), Constraint::Fill]).split(content);
            Paragraph::new("DETAILS").style(th.accented()).render(rows[0], frame);
            if let Some(t) = self.selected_torrent() {
                self.view_torrent_details(t, rows[1], frame);
            }
        }
    }

    pub(super) fn view_torrent_details(&self, t: &Torrent, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let kv = |k: &str, v: String| Line::from_spans([Span::styled(format!("{k:<10}"), th.dimmed()), Span::styled(v, th.text())]);
        let flags = "streaming priority automatic".to_string();
        let metrics = format::Metrics(t);
        let mut lines = vec![
            Line::styled(t.name.clone(), th.title()),
            Line::raw(""),
            kv("state", self.store.pending_for_key(t.hash.as_str()).map_or_else(||format!("{} · {}",format::state_cell(t).trim_end(),format::state_word(t).to_lowercase()),|op|format!("{}…",remote::mutation_verb(op)))),
            kv("progress", metrics.progress()),
            kv("speed", metrics.speed()),
            kv("eta", metrics.eta()),
            kv("peers", metrics.peers()),
            kv("ratio", metrics.ratio()),
            kv("flags", flags),
            kv("upload", metrics.upload()),
            kv("path", t.save_path.clone()),
            kv("added", ago(t.added_at.min(i64::MAX as u64) as i64)),
        ];
        if let Some(message) = &t.error {
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
