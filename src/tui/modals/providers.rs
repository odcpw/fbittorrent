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
        let presence = self.presence.clone();
        Cmd::task(move || {
            let c = Client::new(presence);
            Msg::Plugins(c.search_plugins().map_err(|e| e.to_string()))
        })
    }

    pub(super) fn set_plugins(&self, names: Vec<String>, enable: bool) -> Cmd<Msg> {
        let presence = self.presence.clone();
        Cmd::task(move || {
            let c = Client::new(presence);
            let r = (||->Result<()> { for n in &names { c.search_set_plugin(n, enable)?; } Ok(()) })()
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

    pub(super) fn on_providers_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let Some(m) = self.providers.as_ref() else { return Cmd::none() };
        let n = m.plugins.len();
        let cur = m.list.borrow().selected().and_then(|i| m.plugins.get(i)).cloned();
        let all: Vec<String> = m.plugins.iter().map(|p| p.name.clone()).collect();
        match intent {
            actions::Intent::Cancel => { self.providers = None; }
            actions::Intent::Down => { if n > 0 { m.list.borrow_mut().select_next(n); } }
            actions::Intent::Up => m.list.borrow_mut().select_previous(),
            actions::Intent::Toggle => if let Some(p) = cur { return self.set_plugins(vec![p.name], !p.enabled); },
            actions::Intent::All => return self.set_plugins(all, true),
            actions::Intent::None => return self.set_plugins(all, false),
            // A provider's own (third-party) URL: web pages only.
            actions::Intent::Open => if let Some(p) = cur { if !p.url.is_empty() { crate::public_text::open_web(&p.url); } },
            actions::Intent::Reload => return self.load_plugins(),
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_providers(&self, frame: &mut Frame) {
        let th = &self.theme;
        let Some(m) = &self.providers else { return };
        self.rects.borrow_mut().overlay();
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
            Paragraph::new(th.error_message(&m.error)).style(th.err().bg(th.selection)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        if m.plugins.is_empty() {
            Paragraph::new("No search plugins installed. Run: fbittorrent search --install").style(bg.fg(th.dim)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let iw = inner.width.saturating_sub(2) as usize;
        let elapsed = self.marquee_elapsed();
        self.view_rows(MouseList::Providers,rows[0],m.plugins.len(),
            |i|layout::fingerprint(&m.plugins[i]),
            |i|{let p=&m.plugins[i];
            let mark = if p.enabled { "[x] " } else { "[ ] " };
            let st = if p.enabled { bg.fg(th.fg) } else { bg.fg(th.dim) };
            let site = p.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
            Line::from_spans([
                Span::styled(mark, st),
                Span::styled(readable_cell(&format!("{} · {}",if p.kind.is_empty(){"nova"}else{&p.kind},p.full_name), 22, hovered_for(&m.list,i,elapsed)), st),
                Span::styled(" ", st),
                Span::styled(cell(&site, iw.saturating_sub(4 + 22 + 1)), bg.fg(th.dim)),
            ])
            },frame);
        let on = m.plugins.iter().filter(|p| p.enabled).count();
        let foot = format!("{on} of {} enabled · Space toggle · a all · n none · o open site · Esc close", m.plugins.len());
        Paragraph::new(foot).style(bg.fg(th.dim)).wrap(WrapMode::None).render(rows[1], frame);
    }
}
