//! Existing modals files behavior.
use super::*;

pub(super) struct FilesModal {
    pub(super) hash: String,
    pub(super) name: String,
    pub(super) files: Vec<TorrentFile>,
    pub(super) list: RefCell<ListState>,
    pub(super) loading: bool,
    pub(super) saving:bool,
    pub(super) error: String,
}

impl TuiApp {
    pub(super) fn open_files(&mut self, t: &Torrent) -> Cmd<Msg> {
        self.files = Some(FilesModal { hash: t.hash.to_string(), name: t.name.clone(), files: vec![], list: RefCell::new(ListState::default()), loading: true, saving:false,error: String::new() });
        self.reload_files(t.hash.to_string())
    }

    pub(super) fn reload_files(&self, hash: String) -> Cmd<Msg> {
        let presence = self.presence.clone();
        Cmd::task(move || {
            let id=presence.as_ref().map(|connection|connection.id);
            let result=presence.ok_or_else(||anyhow::anyhow!("fbittorrent stopped")).and_then(|connection|remote::files(&connection,&hash));
            Msg::ApiFiles {connection:id,hash,result:result.map_err(|error|error.to_string())}
        })
    }

    pub(super) fn files_editable(&self)->bool {self.files.as_ref().is_some_and(|m|!m.loading&&!m.saving)}
    pub(super) fn set_file_prio(&mut self, hash: String, ids: Vec<i64>, prio: i64) -> Cmd<Msg> {
        if !self.files_editable(){return Cmd::none();}
        self.files.as_mut().unwrap().saving=true;
        let presence = self.presence.clone();
        let h = hash.clone();
        Cmd::task(move || {
            let id=presence.as_ref().map(|connection|connection.id);
            let result=presence.ok_or_else(||anyhow::anyhow!("fbittorrent stopped")).and_then(|connection|remote::set_files(&connection,&h,&ids,prio));
            Msg::FilesSaved {connection:id,hash,result:result.map_err(|error|error.to_string())}
        })
    }

    #[cfg(test)]
    pub(super) fn on_files_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Modal(actions::ModalKind::Files), key)
    }

    pub(super) fn on_files_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        if matches!(intent,actions::Intent::Toggle|actions::Intent::Include|actions::Intent::All|actions::Intent::None) && !self.files_editable(){return Cmd::none();}
        let Some(m) = self.files.as_ref() else { return Cmd::none() };
        let n = m.files.len();
        let hash = m.hash.clone();
        let sel = m.list.borrow().selected();
        let cur = sel.and_then(|i| m.files.get(i)).cloned();
        let all_ids: Vec<i64> = m.files.iter().map(|f| f.index).collect();
        match intent {
            actions::Intent::Cancel => { self.files = None; self.details = None; }
            actions::Intent::Down => { if n > 0 { m.list.borrow_mut().select_next(n); } }
            actions::Intent::Up => m.list.borrow_mut().select_previous(),
            actions::Intent::First => m.list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            actions::Intent::Last => m.list.borrow_mut().select(n.checked_sub(1)),
            actions::Intent::Toggle => if let Some(f) = cur {
                let prio = if f.priority == 0 { 1 } else { 0 };
                return self.set_file_prio(hash, vec![f.index], prio);
            },
            actions::Intent::Include => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 1); },
            actions::Intent::All => return self.set_file_prio(hash, all_ids, 1),
            actions::Intent::None => return self.set_file_prio(hash, all_ids, 0),
            actions::Intent::Reload => return self.reload_files(hash),
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_files(&self, frame: &mut Frame) {
        let th = &self.theme;
        let Some(m) = &self.files else { return };
        self.rects.borrow_mut().overlay();
        let area = files_area(frame.bounds());
        let w = area.width;
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let title = format!(" Files · {} ", short(&m.name, (w as usize).saturating_sub(12)));
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title).padding(Sides::horizontal(1));
        let inner = block.inner(area);
        block.render(area, frame);
        self.view_file_contents(inner,frame);
    }

    pub(super) fn view_file_contents(&self, inner: Rect, frame: &mut Frame) {
        let Some(m) = &self.files else { return };
        let th = &self.theme;
        let bg = Style::new().bg(th.selection);
        if m.loading {
            Paragraph::new(format!("{} loading file list…", SPINNER[self.tick % 4])).style(bg.fg(th.dim)).render(inner, frame);
            return;
        }
        if !m.error.is_empty() {
            Paragraph::new(th.error_message(&m.error)).style(th.err().bg(th.selection)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        if m.files.is_empty() {
            Paragraph::new("No file list yet. Metadata has not arrived (start the torrent and wait a moment), then press r.").style(bg.fg(th.dim)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        self.rects.borrow_mut().files=Some(rows[1]);
        let iw = inner.width.saturating_sub(2) as usize;
        let tail_w = 1 + 9 + 1 + 6 + 1 + 7;
        let name_w = iw.saturating_sub(4 + tail_w);
        let header = Line::from_spans([Span::styled(format!("      {}", cell("FILE", name_w)), bg.fg(th.dim).bold()), Span::styled(format!(" {:>9} {:>6} {:>7}", "SIZE", "DONE", "PRIO"), bg.fg(th.dim).bold())]);
        Paragraph::new(Text::from_line(header)).wrap(WrapMode::None).render(rows[0], frame);
        let elapsed = self.marquee_elapsed();
        self.view_rows(MouseList::Files,rows[1],m.files.len(),
            |i|layout::fingerprint(&m.files[i]),
            |i|{let f=&m.files[i];
            let (mark, prio, pst) = match f.priority {
                0 => ("[ ]", "skip", bg.fg(th.dim)),
                _ => ("[x]", "include", bg.fg(th.fg)),
            };
            let name_style = if f.priority == 0 { bg.fg(th.dim) } else { bg.fg(th.fg) };
            Line::from_spans([
                Span::styled(format!("{mark} "), name_style),
                Span::styled(readable_cell(&f.name, name_w, hovered_for(&m.list,i,elapsed)), name_style),
                Span::styled(format!(" {:>9} {:>6}", human_bytes(f.size), if f.progress_known { format!("{:.0}%", f.progress * 100.0) } else { "—".into() }), bg.fg(th.dim)),
                Span::styled(format!(" {prio:>7}"), pst),
            ])
            },frame);
        let selected:i64=m.files.iter().filter(|f|f.priority>0).map(|f|f.size).sum();
        let controls = if m.saving {"Saving file selection… · Esc close"}else{"Space include/skip · a/n all/none · Esc close"};
        let foot = format!("{}/{} · {} · {controls}",
            m.files.iter().filter(|f| f.priority > 0).count(), m.files.len(), human_bytes(selected));
        Paragraph::new(foot).style(bg.fg(th.dim)).wrap(WrapMode::None).render(rows[2], frame);
    }
}
