//! Existing screens search behavior.
use super::*;

pub(super) struct SearchSub { pub(super) id: SubId, pub(super) query: String, pub(super) cfg: Config }

impl TuiApp {
    pub(super) fn selected_result(&self) -> Option<&SearchResult> {
        self.search_list.borrow().selected().and_then(|i| self.search_results.get(i))
    }

    pub(super) fn add_search_result(&self, result: SearchResult) -> Cmd<Msg> {
        let opts = self.add_opts();
        let label = short(&result.file_name, 40);
        self.action(format!("added {label}"), move |c| c.add_search_result(&result, &opts))
    }

    pub(super) fn load_browse(&mut self, entry: browse::Entry) -> Cmd<Msg> {
        if let browse::Source::Site(url) = &entry.source {
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            self.set_status(format!("opened {} in browser", entry.provider), false);
            return Cmd::none();
        }
        let label = format!("{} · {}", entry.provider, entry.label);
        self.browse_loading = Some(label.clone());
        self.set_status(format!("loading {label}…"), false);
        let source = entry.source.clone();
        Cmd::task(move || Msg::Browse(label, browse::fetch(&source).map_err(|e| format!("{e:#}"))))
    }

    pub(super) fn start_search(&mut self) -> Cmd<Msg> {
        let q = self.search_input.value().trim().to_string();
        if q.is_empty() { return Cmd::none(); }
        self.search_seq += 1;
        self.search_results.clear();
        self.search_list.borrow_mut().select(None);
        self.search_running = true;
        self.search_sub = Some((1000 + self.search_seq, q.clone()));
        self.set_status(format!("searching \"{q}\"…"), false);
        Cmd::none()
    }

    #[cfg(test)]
    pub(super) fn on_search_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Search), key)
    }

    pub(super) fn on_search_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        if intent==actions::Intent::Providers { return self.open_providers(); }
        if self.search_browse {
            let n = self.browse_entries.len();
            match intent {
                actions::Intent::Cancel | actions::Intent::Browse => { self.search_browse = false; }
                actions::Intent::Down => self.browse_list.borrow_mut().select_next(n),
                actions::Intent::Up => self.browse_list.borrow_mut().select_previous(),
                actions::Intent::First => self.browse_list.borrow_mut().select((n>0).then_some(0)),
                actions::Intent::Last => self.browse_list.borrow_mut().select(n.checked_sub(1)),
                actions::Intent::Query => { self.search_browse = false; self.search_focus_input = true; self.search_input.set_focused(true); }
                actions::Intent::Primary => {
                    let sel = self.browse_list.borrow().selected();
                    if let Some(e) = sel.and_then(|i| self.browse_entries.get(i)).cloned() { return self.load_browse(e); }
                }
                _ => {}
            }
            return Cmd::none();
        }
        let n = self.search_results.len();
        match intent {
            actions::Intent::Browse => {
                self.search_browse = true;
                if self.browse_list.borrow().selected().is_none() { self.browse_list.borrow_mut().select(Some(0)); }
            }
            actions::Intent::Query => { self.search_focus_input = true; self.search_input.set_focused(true); }
            actions::Intent::Cancel => { self.search_sub = None; self.search_running = false; }
            actions::Intent::Down => { if n > 0 { self.search_list.borrow_mut().select_next(n); } }
            actions::Intent::Up => {
                if self.search_list.borrow().selected() == Some(0) { self.search_focus_input = true; self.search_input.set_focused(true); }
                else { self.search_list.borrow_mut().select_previous(); }
            }
            actions::Intent::First => self.search_list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            actions::Intent::Last => self.search_list.borrow_mut().select(n.checked_sub(1)),
            actions::Intent::Primary => if let Some(r) = self.selected_result().cloned() {
                return self.add_search_result(r);
            },
            actions::Intent::Imdb => if let Some(r) = self.selected_result() {
                let (name, id) = (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()));
                return self.open_imdb(name, id);
            },
            actions::Intent::Wiki => if let Some(r) = self.selected_result() {
                let (name, id) = (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()));
                return self.open_wiki(name, id);
            },
            actions::Intent::Source => if let Some(r) = self.selected_result() {
                if !r.descr_link.is_empty() { let _ = std::process::Command::new("xdg-open").arg(&r.descr_link).spawn(); }
            },
            actions::Intent::Follow => if let Some(r) = self.selected_result() {
                let id = if !r.imdb.is_empty() { Some(r.imdb.clone()) } else { self.name_ids.get(&r.file_name).filter(|i| !i.is_empty()).cloned() };
                let name = r.file_name.clone();
                return self.follow(name, id);
            },
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_search(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let rows = Flex::vertical().constraints([Constraint::Fixed(3), Constraint::Fill]).split(area);
        let title = if self.search_running { format!(" Search {} ", SPINNER[self.tick % 4]) } else { " Search ".to_string() };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input { th.border_focused() } else { th.border() })
            .title(&title).padding(Sides::horizontal(1));
        let inner = block.inner(rows[0]);
        block.render(rows[0], frame);
        self.search_input.render(inner, frame);

        let cols = detail_columns(rows[1]);
        if self.search_browse {
            self.view_browse(cols[0], cols[1], frame);
            return;
        }
        let enabled = self.plugins.iter().filter(|p| p.enabled).count();
        let title = if self.plugins.is_empty() { format!(" Results ({}) ", self.search_results.len()) }
            else { format!(" Results ({}) · {}/{} providers ", self.search_results.len(), enabled, self.plugins.len()) };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input { th.border() } else { th.border_focused() }).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);
        if self.search_results.is_empty() {
            let msg = if self.search_running { "Searching… Results appear as providers reply." }
                else { "Type a query, then (Enter) Search.\nEmpty query + Enter browses provider lists.\n(/) Edit query from results · (B)rowse lists" };
            Paragraph::new(msg).style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(result_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.search_results.iter().enumerate().map(|(i, r)|
                li(result_line(r, self.rating_for(r), inner.width.saturating_sub(2), th, hovered_for(&self.search_list,i,elapsed)))).collect();
            let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
            StatefulWidget::render(&list, rows[1], frame, &mut self.search_list.borrow_mut());
        }

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" Details ");
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        if let Some(r) = self.selected_result() {
            let kv = |k: &str, v: String| Line::from_spans([Span::styled(format!("{k:<10}"), th.dimmed()), Span::styled(v, th.text())]);
            let mut lines = vec![
                Line::styled(r.file_name.clone(), th.title()),
                Line::raw(""),
                kv("size", human_bytes(r.file_size)),
            ];
            let peers = [(r.seeders, "seeds"), (r.leechers, "leechers")].into_iter()
                .filter(|(count, _)| *count >= 0).map(|(count, label)| format!("{count} {label}"))
                .collect::<Vec<_>>().join(", ");
            if !peers.is_empty() { lines.push(kv("peers", peers)); }
            lines.extend([
                kv("uploaded", if r.pub_date > 0 { ymd(r.pub_date) } else { "unknown".into() }),
                kv("source", format!("{}  {}", r.engine, r.site_url)),
                Line::raw(""),
            ]);
            let inner = pad(inner);
            let rows = Flex::vertical().constraints([Constraint::Fixed(7), Constraint::Fill]).split(inner);
            Paragraph::new(Text::from_lines(lines)).wrap(WrapMode::Word).render(rows[0], frame);
            self.view_imdb_card(&r.file_name, rows[1], frame);
        }
    }

    pub(super) fn view_browse(&self, left: Rect, right: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Browse providers ");
        let inner = block.inner(left);
        block.render(left, frame);
        let w = inner.width.saturating_sub(2) as usize;
        let mut last = "";
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = self.browse_entries.iter().enumerate().map(|(i, e)| {
            let prov = if e.provider == last { String::new() } else { e.provider.to_string() };
            last = e.provider;
            let is_site = matches!(e.source, browse::Source::Site(_));
            let glyph = if is_site { "󰖟 " } else { "󰈙 " };
            let label = format!("{glyph}{}", e.label);
            li(Line::from_spans([
                Span::styled(cell(&prov, 16), th.accented()),
                Span::styled(readable_cell(&label, w.saturating_sub(16), hovered_for(&self.browse_list,i,elapsed)), if is_site { th.dimmed() } else { th.text() }),
            ]))
        }).collect();
        let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
        StatefulWidget::render(&list, inner, frame, &mut self.browse_list.borrow_mut());

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" About ");
        let inner = block.inner(right);
        block.render(right, frame);
        let sel = self.browse_list.borrow().selected().and_then(|i| self.browse_entries.get(i));
        let text = match sel.map(|e| e.provider) {
            Some("1337x") => "1337x sits behind a browser challenge, so its lists cannot be loaded here.\nEach entry opens the page in your browser.",
            Some("YTS") => "Latest YTS releases (1080p picked per film) via the YTS JSON API.\nThe main domain does not resolve from this network; mirrors are tried, otherwise use Open site.",
            Some("EZTV") => "The 100 most recent TV episodes on EZTV, sorted by seeders.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            Some("The Pirate Bay") => "Top-100 lists from The Pirate Bay's public JSON feed.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            _ => "",
        };
        let mut lines = vec![Line::styled(text.to_string(), th.text()), Line::raw("")];
        if let Some(l) = &self.browse_loading { lines.push(Line::styled(format!("{} loading {l}…", SPINNER[self.tick % 4]), th.warn())); }
        lines.push(Line::raw(""));
        lines.push(Line::from_spans([Span::styled("Enter", th.key()), Span::styled(" load list or open site   ", th.dimmed()), Span::styled("Esc", th.key()), Span::styled(" back to results", th.dimmed())]));
        Paragraph::new(Text::from_lines(lines)).wrap(WrapMode::Word).render(pad(inner), frame);
    }
}


impl Subscription<Msg> for SearchSub {
    fn id(&self) -> SubId { self.id }
    fn run(&self, sender: Sender<Msg>, stop: StopSignal) {
        let id = self.id;
        let fail = |e: String| Msg::SearchBatch { id, results: vec![], done: true, error: Some(e) };
        let c = Client::from_config(&self.cfg);
        if let Err(e) = c.login() { let _ = sender.send(fail(e.to_string())); return; }
        let sid = match c.search_start(&self.query) {
            Ok(i) => i,
            Err(e) => { let _ = sender.send(fail(e.to_string())); return; }
        };
        let mut misses = 0;
        // One straggling plugin can keep a job "Running" for a minute; the useful
        // results are in long before that, so cut it off.
        let started = std::time::Instant::now();
        let budget = Duration::from_secs(30);
        loop {
            if stop.wait_timeout(Duration::from_millis(700)) {
                let _ = c.search_stop(sid);
                let _ = c.search_delete(sid);
                return;
            }
            let timed_out = started.elapsed() > budget;
            if timed_out { let _ = c.search_stop(sid); }
            match c.search_results(sid, 1000, 0) {
                Ok(r) => {
                    let done = r.status != "Running" || timed_out;
                    if sender.send(Msg::SearchBatch { id, results: r.results, done, error: r.error }).is_err() || done {
                        let _ = c.search_delete(sid);
                        return;
                    }
                }
                Err(e) => {
                    misses += 1;
                    if misses > 6 { let _ = sender.send(fail(e.to_string())); return; }
                }
            }
        }
    }
}

/// Collapse the same torrent reported by several providers (same info hash),
/// keeping the copy with the most seeders.
pub(super) fn dedupe_results(rs: Vec<SearchResult>) -> Vec<SearchResult> {
    fn key(r: &SearchResult) -> String {
        let u = r.file_url.to_ascii_lowercase();
        if let Some(i) = u.find("urn:btih:") {
            let h: String = u[i + 9..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if h.len() >= 32 { return h; }
        }
        u
    }
    let mut best: HashMap<String, SearchResult> = HashMap::new();
    for r in rs {
        let k = key(&r);
        match best.get(&k) {
            Some(b) if b.seeders >= r.seeders => {}
            _ => { best.insert(k, r); }
        }
    }
    best.into_values().collect()
}

pub(super) fn result_tail_width(width: u16) -> usize {
    if width >= 88 { RESULT_TAIL_W } else if width >= 60 { 5 + 1 + 6 + 1 + 9 } else { 6 + 1 + 9 }
}

pub(super) fn result_line(r: &SearchResult, rating: Option<f32>, width: u16, th: &Theme, selected_for: Option<Duration>) -> Line<'static> {
    let stars = match rating { Some(v) => format!("{v:>5.1}"), None => "     ".into() };
    let star_style = match rating { Some(v) if v >= 7.5 => th.ok(), Some(v) if v >= 6.0 => th.warn(), Some(_) => th.dimmed(), None => th.dimmed() };
    let seeds = format!("{:>6}", if r.seeders < 0 { String::new() } else { r.seeders.to_string() });
    let size = format!(" {:>9}", human_bytes(r.file_size));
    let name_w = (width as usize).saturating_sub(result_tail_width(width) + 1);
    let seed_style = if r.seeders >= 20 { th.ok() } else if r.seeders > 0 { th.warn() } else { th.dimmed() };
    let mut spans = vec![Span::styled(readable_cell(&r.file_name, name_w, selected_for), th.text()), Span::raw(" ")];
    if width >= 60 { spans.extend([Span::styled(stars, star_style), Span::raw(" ")]); }
    spans.push(Span::styled(seeds, seed_style));
    if width >= 88 { spans.push(Span::styled(format!(" {:>6}", if r.leechers < 0 { String::new() } else { r.leechers.to_string() }), th.dimmed())); }
    spans.push(Span::styled(size, th.text()));
    if width >= 88 {
        spans.extend([Span::styled(format!(" {:>10}", ymd(r.pub_date)), th.dimmed()), Span::styled(format!("  {}", cell(&r.engine, 14)), th.dimmed())]);
    }
    Line::from_spans(spans)
}

pub(super) fn result_header(width: u16, th: &Theme) -> Line<'static> {
    let name_w = (width as usize).saturating_sub(result_tail_width(width) + 1);
    let tail = if width >= 88 { format!(" {:>5} {:>6} {:>6} {:>9} {:>10}  {:<14}", "IMDB", "SEEDS", "PEERS", "SIZE", "UPLOADED", "SOURCE") }
        else if width >= 60 { format!(" {:>5} {:>6} {:>9}", "IMDB", "SEEDS", "SIZE") }
        else { format!(" {:>6} {:>9}", "SEEDS", "SIZE") };
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::styled(tail, th.dimmed().bold())])
}

pub(super) const MAX_RESULTS: usize = 100;

pub(super) const RESULT_TAIL_W: usize = 5 + 1 + 6 + 1 + 6 + 1 + 9 + 1 + 10 + 2 + 14;
