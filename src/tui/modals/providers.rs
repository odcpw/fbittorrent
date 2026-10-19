//! Existing modals providers behavior.
use super::*;

pub(super) struct ProvidersModal {
    pub(super) plugins: Vec<SearchPlugin>,
    pub(super) list: RefCell<ListState>,
    pub(super) loading: bool,
    pub(super) error: String,
}

impl TuiApp {
    pub(super) fn load_plugins(&self) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            Msg::Plugins(c.login().and_then(|_| c.search_plugins()).map_err(|e| e.to_string()))
        })
    }

    pub(super) fn set_plugins(&self, names: Vec<String>, enable: bool) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login()
                .and_then(|_| { for n in &names { c.search_set_plugin(n, enable)?; } Ok(()) })
                .and_then(|_| c.search_plugins())
                .map_err(|e| e.to_string());
            Msg::Plugins(r)
        })
    }

    pub(super) fn open_providers(&mut self) -> Cmd<Msg> {
        self.providers = Some(ProvidersModal { plugins: self.plugins.clone(), list: RefCell::new(ListState::default()), loading: self.plugins.is_empty(), error: String::new() });
        if let Some(m) = &self.providers { if !m.plugins.is_empty() { m.list.borrow_mut().select(Some(0)); } }
        self.load_plugins()
    }

    pub(super) fn on_providers_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let Some(m) = self.providers.as_ref() else { return Cmd::none() };
        let n = m.plugins.len();
        let cur = m.list.borrow().selected().and_then(|i| m.plugins.get(i)).cloned();
        let all: Vec<String> = m.plugins.iter().map(|p| p.name.clone()).collect();
        match k.code {
            KeyCode::Escape | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('s') => { self.providers = None; }
            KeyCode::Down | KeyCode::Char('j') => { if n > 0 { m.list.borrow_mut().select_next(n); } }
            KeyCode::Up | KeyCode::Char('k') => m.list.borrow_mut().select_previous(),
            KeyCode::Char(' ') => if let Some(p) = cur { return self.set_plugins(vec![p.name], !p.enabled); },
            KeyCode::Char('a') => return self.set_plugins(all, true),
            KeyCode::Char('n') => return self.set_plugins(all, false),
            KeyCode::Char('o') => if let Some(p) = cur { if !p.url.is_empty() { let _ = std::process::Command::new("xdg-open").arg(&p.url).spawn(); } },
            KeyCode::Char('r') => return self.load_plugins(),
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_providers(&self, frame: &mut Frame) {
        let th = &self.theme;
        let Some(m) = &self.providers else { return };
        let area = providers_area(frame.bounds(), m.plugins.len());
        let bg = Style::new().bg(th.selection);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), bg)));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Search providers ").padding(Sides::horizontal(1));
        let inner = block.inner(area);
        block.render(area, frame);
        if m.loading {
            Paragraph::new(format!("{} loading providers…", SPINNER[self.tick % 4])).style(bg.fg(th.dim)).render(inner, frame);
            return;
        }
        if !m.error.is_empty() {
            Paragraph::new(m.error.clone()).style(bg.fg(th.red)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        if m.plugins.is_empty() {
            Paragraph::new("No search plugins installed. Run: fbittorrent search --install").style(bg.fg(th.dim)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let iw = inner.width.saturating_sub(2) as usize;
        let selected = readable_index(&m.list);
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = m.plugins.iter().enumerate().map(|(i, p)| {
            let mark = if p.enabled { "[x] " } else { "[ ] " };
            let st = if p.enabled { bg.fg(th.fg) } else { bg.fg(th.dim) };
            let site = p.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
            li(Line::from_spans([
                Span::styled(mark, st),
                Span::styled(readable_cell(&p.full_name, 22, (selected == Some(i)).then_some(elapsed)), st),
                Span::styled(cell(&site, iw.saturating_sub(4 + 22 + 1)), bg.fg(th.dim)),
            ]))
        }).collect();
        let list = List::new(items).style(bg).highlight_style(th.selected().bg(th.accent).fg(th.selection)).highlight_symbol("▌");
        StatefulWidget::render(&list, rows[0], frame, &mut m.list.borrow_mut());
        let on = m.plugins.iter().filter(|p| p.enabled).count();
        let foot = format!("{on} of {} enabled · Space toggle · a all · n none · o open site · Esc close", m.plugins.len());
        Paragraph::new(foot).style(bg.fg(th.dim)).wrap(WrapMode::None).render(rows[1], frame);
    }
}

