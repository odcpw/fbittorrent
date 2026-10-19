//! Existing imdb_card behavior.
use super::*;

impl TuiApp {
    pub(super) fn lookup_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        if matches!(self.imdb.get(&name), Some(ImdbState::Loading) | Some(ImdbState::Ready(..))) { return Cmd::none(); }
        self.imdb.insert(name.clone(), ImdbState::Loading);
        let key = name.clone();
        Cmd::task(move || {
            if imdb::ratings_stale() { let _ = imdb::refresh_ratings(); }
            let found = match id.as_deref().filter(|i| i.starts_with("tt")) {
                Some(i) => imdb::lookup_id(i),
                None => imdb::lookup(&name),
            };
            let r = found.map_err(|e| e.to_string()).map(|t| t.map(|t| {
                let poster = if t.poster_url.is_empty() { None } else {
                    imdb::fetch_poster(&t.poster_url, 360).ok()
                        .and_then(|p| std::fs::read(p).ok())
                        .and_then(|b| Poster::from_bytes(&b))
                };
                (t, poster)
            }));
            Msg::Imdb(key, r)
        })
    }

    pub(super) fn load_ratings(&self) -> Cmd<Msg> {
        Cmd::task(|| {
            if imdb::ratings_stale() { let _ = imdb::refresh_ratings(); }
            Msg::Ratings(imdb::RatingsIndex::load().map(Arc::new).map_err(|e| e.to_string()))
        })
    }

    /// Rating for a result: from the in-memory index when the source gave an IMDb id,
    /// otherwise from a finished name lookup.
    pub(super) fn rating_for(&self, r: &SearchResult) -> Option<f32> {
        let id = if !r.imdb.is_empty() { Some(r.imdb.as_str()) } else { self.name_ids.get(&r.file_name).map(|s| s.as_str()) };
        if let (Some(id), Some(idx)) = (id, &self.ratings) {
            if let Some((v, _)) = idx.get(id) { return Some(v); }
        }
        match self.imdb.get(&r.file_name) { Some(ImdbState::Ready(t, _)) => t.rating, _ => None }
    }

    /// Resolve IMDb ids for the best results that came without one (top of the list only:
    /// one network call each, cached on disk by release name).
    pub(super) fn resolve_ids(&mut self) -> Cmd<Msg> {
        if self.ids_inflight { return Cmd::none(); }
        let names: Vec<String> = self.search_results.iter()
            .filter(|r| r.imdb.is_empty() && !self.name_ids.contains_key(&r.file_name))
            .take(24)
            .map(|r| r.file_name.clone())
            .collect();
        if names.is_empty() { return Cmd::none(); }
        self.ids_inflight = true;
        Cmd::task(move || {
            // Names that resolve to nothing are remembered as empty so we stop retrying them.
            let pairs = names.into_iter().map(|n| { let id = imdb::resolve_id(&n).unwrap_or_default(); (n, id) }).collect();
            Msg::Ids(pairs)
        })
    }

    /// Open the IMDb page (or start the lookup) for a release name.
    pub(super) fn open_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        if let Some(ImdbState::Ready(t, _)) = self.imdb.get(&name) {
            let _ = std::process::Command::new("xdg-open").arg(t.url()).spawn();
            self.set_status("opened IMDb in browser", false);
            Cmd::none()
        } else {
            self.lookup_imdb(name, id)
        }
    }

    /// Open the Wikipedia article behind the IMDb card, if there is one.
    pub(super) fn open_wiki(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        match self.imdb.get(&name) {
            Some(ImdbState::Ready(t, _)) if !t.wiki_url.is_empty() => {
                let _ = std::process::Command::new("xdg-open").arg(&t.wiki_url).spawn();
                self.set_status("opened Wikipedia in browser", false);
                Cmd::none()
            }
            Some(ImdbState::Ready(..)) => { self.set_status("no Wikipedia article found for this title", true); Cmd::none() }
            _ => self.lookup_imdb(name, id),
        }
    }

    pub(super) fn view_imdb_card(&self, key: &str, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        if area.height == 0 { return; }
        match self.imdb.get(key) {
            None => Paragraph::new("IMDb card appears when the cursor rests here").style(th.dimmed()).render(area, frame),
            Some(ImdbState::Loading) => Paragraph::new(format!("{} looking up IMDb…", SPINNER[self.tick % 4])).style(th.dimmed()).render(area, frame),
            Some(ImdbState::Missing) => Paragraph::new("IMDb: no match for this name").style(th.dimmed()).render(area, frame),
            Some(ImdbState::Failed(e)) => Paragraph::new(format!("IMDb lookup failed: {e}")).style(th.err()).wrap(WrapMode::Word).render(area, frame),
            Some(ImdbState::Ready(t, poster)) => {
                let has_poster = poster.is_some() && area.width >= poster::CELL_W + 20;
                let cols = Flex::horizontal()
                    .constraints([Constraint::Fixed(if has_poster { poster::CELL_W + 1 } else { 0 }), Constraint::Fill])
                    .split(area);
                if let (true, Some(p)) = (has_poster, poster) {
                    let parea = Rect::new(cols[0].x, cols[0].y, poster::CELL_W.min(cols[0].width), poster::CELL_H.min(cols[0].height));
                    let overlay = self.files.is_some() || self.providers.is_some() || self.show_help || self.prompt.is_some();
                    match (&self.kitty, overlay) {
                        (Some(k), false) => { k.borrow_mut().place(key, p, parea); self.poster_drawn.set(true); }
                        _ => p.render_blocks(parea, frame),
                    }
                }
                let year = t.year.map(|y| format!(" ({y})")).unwrap_or_default();
                let rating = match (t.rating, t.votes) {
                    (Some(r), Some(v)) => format!("★ {r:.1}  ·  {} votes", human_count(v)),
                    _ => "★ no rating yet".into(),
                };
                let mut lines = vec![
                    Line::from_spans([Span::styled(t.name.clone(), th.title()), Span::styled(year, th.dimmed()), Span::styled(format!("  {}", t.kind), th.dimmed())]),
                    Line::styled(rating, th.warn()),
                ];
                if !t.stars.is_empty() { lines.push(Line::styled(t.stars.clone(), th.dimmed())); }
                lines.push(Line::raw(""));
                if !t.blurb.is_empty() { lines.push(Line::styled(t.blurb.clone(), th.text())); }
                lines.push(Line::raw(""));
                lines.push(Line::from_spans([Span::styled("i", th.key()), Span::styled(" open on IMDb    ", th.dimmed()), Span::styled("w", th.key()), Span::styled(" open Wikipedia", th.dimmed())]));
                Paragraph::new(Text::from_lines(lines)).wrap(WrapMode::Word).render(cols[1], frame);
            }
        }
    }
}
