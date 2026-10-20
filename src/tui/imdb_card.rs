//! Existing imdb_card behavior.
use super::*;

impl TuiApp {
    pub(super) fn lookup_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        let known=id.as_deref().filter(|id|id.starts_with("tt")).or_else(||self.imdb_aliases.get(&name).map(String::as_str));
        let key=known.unwrap_or(&name).to_owned();
        if self.imdb_aliases.len()>=1024&&!self.imdb_aliases.contains_key(&name) {if let Some(old)=self.imdb_aliases.keys().next().cloned(){self.imdb_aliases.remove(&old);}}
        self.imdb_aliases.insert(name.clone(),key.clone());
        if matches!(self.imdb.get(&key),Some(ImdbState::Loading|ImdbState::Ready(..)|ImdbState::Missing)){return Cmd::none();}
        let connection=self.presence.clone().filter(|c|c.alive());
        self.cache_imdb(key.clone(),ImdbState::Loading);
        let id=id.or_else(||key.starts_with("tt").then(||key.clone()));
        Cmd::task(move || Msg::Imdb(key,connection.ok_or_else(||anyhow::anyhow!("fbittorrent stopped")).and_then(|connection|remote::imdb::lookup(&connection,&name,id.as_deref())).map_err(|e|e.to_string())))
    }

    /// Rating for a result: from the in-memory index when the source gave an IMDb id,
    /// otherwise from a finished name lookup.
    pub(super) fn rating_for(&self, r: &SearchResult) -> Option<f32> {
        if let Some(rating)=self.wire_result(r).and_then(|row|row.rating){return Some(rating as f32);}
        let id = if !r.imdb.is_empty() { Some(r.imdb.as_str()) } else { self.name_ids.get(&r.file_name).map(|s| s.as_str()) };
        if let (Some(id), Some(idx)) = (id, &self.ratings) {
            if let Some((v, _)) = idx.get(id) { return Some(v); }
        }
        match self.imdb_state(&r.file_name) { Some(ImdbState::Ready(t, _)) => t.rating, _ => None }
    }

    /// Open the IMDb page (or start the lookup) for a release name.
    pub(super) fn open_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        if let Some(ImdbState::Ready(t, _)) = self.imdb_state(&name) {
            crate::public_text::open_web(&t.url());
            self.set_status("opened IMDb in browser", false);
            Cmd::none()
        } else {
            self.lookup_imdb(name, id)
        }
    }

    /// Open the Wikipedia article behind the IMDb card, if there is one.
    pub(super) fn open_wiki(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        match self.imdb_state(&name) {
            Some(ImdbState::Ready(t, _)) if !t.wiki_url.is_empty() => {
                // The article URL comes from a remote API response: web pages only.
                let opened = crate::public_text::open_web(&t.wiki_url);
                self.set_status(if opened { "opened Wikipedia in browser" } else { "Wikipedia link is not a web page" }, !opened);
                Cmd::none()
            }
            Some(ImdbState::Ready(..)) => { self.set_status("no Wikipedia article found for this title", true); Cmd::none() }
            _ => self.lookup_imdb(name, id),
        }
    }

    pub(super) fn view_imdb_card(&self, key: &str, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        if area.is_empty() { return; }
        self.rects.borrow_mut().poster=Some(area);
        match self.imdb_state(key) {
            None => Paragraph::new("IMDb card appears when the cursor rests here").style(th.dimmed()).render(area, frame),
            Some(ImdbState::Loading) => Paragraph::new(format!("{} looking up IMDb…", SPINNER[self.tick % 4])).style(th.dimmed()).render(area, frame),
            Some(ImdbState::Missing) => Paragraph::new("IMDb: no match for this name").style(th.dimmed()).render(area, frame),
            Some(ImdbState::Failed(e)) => Paragraph::new(format!("IMDb lookup failed: {e}")).style(th.err()).wrap(WrapMode::Word).render(area, frame),
            Some(ImdbState::Ready(t, poster)) => {
                let has_poster = area.width >= poster::CELL_W + 20 && area.height>=poster::CELL_H;
                let cols = Flex::horizontal()
                    .constraints([Constraint::Fixed(if has_poster { poster::CELL_W + 1 } else { 0 }), Constraint::Fill])
                    .split(area);
                if let (true, Some(p)) = (has_poster, poster) {
                    let parea = Rect::new(cols[0].x, cols[0].y, poster::CELL_W.min(cols[0].width), poster::CELL_H.min(cols[0].height));
                    let overlay = self.graphics_suppressed.get();
                    match (&self.kitty, overlay) {
                        (Some(k), false) => { k.borrow_mut().place(&t.id, p, parea); self.poster_drawn.set(true); }
                        _ => p.render_blocks(parea, frame),
                    }
                }
                if has_poster && poster.is_none(){
                    let placeholder=Rect::new(cols[0].x,cols[0].y,poster::CELL_W,poster::CELL_H);
                    Paragraph::new(Text::from_lines((0..poster::CELL_H).map(|_|Line::styled(" ".repeat(poster::CELL_W as usize),Style::new().bg(th.selection))))).render(placeholder,frame);
                }
                let year = t.year.map(|y| format!(" ({y})")).unwrap_or_default();
                let rating = match (t.rating, t.votes) {
                    (Some(r), Some(v)) => format!("★ {r:.1}  ·  {} votes", human_count(v)),
                    _ => "★ no rating yet".into(),
                };
                let mut lines = vec![
                    Line::from_spans([Span::styled(t.name.clone(), th.title()), Span::styled(year, th.title()), Span::styled(format!("  {}", t.kind), th.dimmed())]),
                    Line::styled(rating, th.dimmed()),
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
