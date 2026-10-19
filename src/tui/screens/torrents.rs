//! Existing screens torrents behavior.
use super::*;

impl TuiApp {
    pub(super) fn selected_torrent(&self) -> Option<&Torrent> {
        self.tor_list.borrow().selected().and_then(|i| self.torrents.get(i))
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
                    let stopped = t.is_stopped();
                    return self.action(if stopped { "started" } else { "stopped" }, move |c| if stopped { c.start(&[t.hash]) } else { c.stop(&[t.hash]) });
                }
            }
            actions::Intent::Sequential if !self.cfg.native() => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled sequential", move |c| c.toggle_sequential(&[t.hash]));
            },
            actions::Intent::FirstLast if !self.cfg.native() => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled first/last", move |c| c.toggle_first_last(&[t.hash]));
            },
            actions::Intent::Seed => if let Some(t) = self.selected_torrent().cloned() {
                let allow = !self.seed_allowed(&t);
                return self.action(if allow { "seeding allowed" } else { "seeding off · upload capped" },
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
                return self.open_files(&t);
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
        let title = if self.cfg.native() && !network_ready { format!(" {} ", network_message) }
            else if queued > 0 { format!(" Torrents ({}) · {queued} queued ", self.torrents.len()) }
            else { format!(" Torrents ({}) ", self.torrents.len()) };
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);

        if !self.online {
            let msg = if self.daemon_stopped {"fbittorrent stopped\n\n(Enter) Start the daemon".into()} else {format!("Torrent engine is not reachable.\n{}\n\n(R)etry", self.backend_error)};
            Paragraph::new(msg).style(th.err()).wrap(WrapMode::Word).render(pad(inner), frame);
        } else if self.torrents.is_empty() {
            let text = if !network_ready { format!("{}\n\nConnect your VPN to start transfers.\n(F6) Download and VPN settings\n\nYour saved downloads stay here.", network_message) }
                else { "No torrents.\n\n(A)dd a magnet link or URL\n(C)lipboard: add a copied link\n(2) Search".into() };
            Paragraph::new(text)
                .style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(torrent_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.torrents.iter().enumerate().map(|(i, t)|
                li(torrent_line(t, self.store.torrents.get(&t.hash), self.store.pending_for_key(&t.hash), inner.width.saturating_sub(2), th, hovered_for(&self.tor_list,i,elapsed)))).collect();
            let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
            StatefulWidget::render(&list, rows[1], frame, &mut self.tor_list.borrow_mut());
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
        let flags = if self.cfg.native() { "streaming priority automatic".to_string() } else {
            format!("{}sequential  {}first/last", if t.seq_dl { "✓ " } else { "✗ " }, if t.f_l_piece_prio { "✓ " } else { "✗ " })
        };
        let metrics = format::Metrics {legacy:t,api:self.store.torrents.get(&t.hash)};
        let mut lines = vec![
            Line::styled(t.name.clone(), th.title()),
            Line::raw(""),
            kv("state", self.store.pending_for_key(&t.hash).map_or_else(||state_label(t),|op|format!("{}…",remote::mutation_verb(op)))),
            kv("progress", metrics.progress()),
            kv("speed", metrics.speed()),
            kv("eta", metrics.eta()),
            kv("peers", metrics.peers(self.cfg.native())),
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


pub(super) fn state_label(t: &Torrent) -> String {
    match t.state.as_str() {
        "downloading" | "forcedDL" => "downloading",
        "metaDL" | "forcedMetaDL" => "fetching metadata",
        "stalledDL" => "stalled",
        "queuedDL" => "queued",
        "checkingDL" | "checkingUP" | "checkingResumeData" => "checking",
        "uploading" | "forcedUP" => "seeding",
        "stalledUP" => "seeding (idle)",
        "queuedUP" => "queued (seed)",
        "stoppedDL" | "pausedDL" => "stopped",
        "stoppedUP" | "pausedUP" => "done",
        "error" => "error",
        "missingFiles" => "missing files",
        s => s,
    }.to_string()
}

pub(super) fn torrent_line(t: &Torrent, api:Option<&crate::api::Torrent>, pending:Option<&str>, width: u16, th: &Theme, selected_for: Option<Duration>) -> Line<'static> {
    let bar_w: u16 = 10;
    let percent = if t.progress_known { format!("{:>6.1}%", t.progress * 100.0) } else { "      —".into() };
    let right = format!(" {percent} {:>11} {:>7} {}{}{}", if t.is_active() { api.map_or_else(||human_rate(t.dlspeed),|row|row.dl_bps.map_or_else(||"—".into(),|rate|human_rate(i64::try_from(rate).unwrap_or(i64::MAX)))) } else { api.map_or_else(||human_bytes(t.size),|row|row.size.map_or_else(||"—".into(),|size|human_bytes(i64::try_from(size).unwrap_or(i64::MAX)))) },
        if t.is_active() { api.map_or_else(||human_eta(t.eta),|row|row.eta_s.map_or_else(||"—".into(),|seconds|human_eta(i64::try_from(seconds).unwrap_or(i64::MAX)))) } else { "".into() }, if t.seq_dl { "S" } else { "·" }, if t.f_l_piece_prio { "F" } else { "·" },
        if api.map_or_else(||t.seed_allowed(),|row|row.seed) { "U" } else { "·" });
    let name_w = (width as usize).saturating_sub(bar_w as usize + 1 + right.len());
    let name = readable_cell(&t.name, name_w, selected_for);
    let padn = 0;
    let bar = if let Some(op)=pending {cell(remote::mutation_verb(op),bar_w as usize)}
        else if api.is_some()&&!t.progress_known&&t.state!="waitingVPN" {cell(if t.state=="metaDL" {"Metadata"} else {"—"},bar_w as usize)}
        else if t.is_queued() { "  Queued  ".into() }
        else if t.state == "waitingVPN" { " Wait VPN ".into() }
        else { MiniBar::new(t.progress, bar_w).show_percent(false).filled_char('█').empty_char('░').render_string() };
    let bar_style = if pending.is_some() {th.dimmed()} else if t.is_active() { Style::new().fg(th.accent) } else if t.is_stopped() { th.dimmed() } else { th.ok() };
    let name_style = if pending.is_some()||t.is_stopped() { th.dimmed() } else if t.state == "error" || t.state == "missingFiles" { th.err() } else { th.text() };
    Line::from_spans([
        Span::styled(name, name_style),
        Span::raw(" ".repeat(padn + 1)),
        Span::styled(bar, bar_style),
        Span::styled(right, th.dimmed()),
    ])
}

pub(super) fn torrent_header(width: u16, th: &Theme) -> Line<'static> {
    let right = format!(" {:>7} {:>11} {:>7} {}", "DONE", "SPEED", "ETA", "SFU");
    let name_w = (width as usize).saturating_sub(10 + 1 + right.len());
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::raw(" ".repeat(11)), Span::styled(right, th.dimmed().bold())])
}
