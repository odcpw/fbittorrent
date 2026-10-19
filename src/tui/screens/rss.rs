//! Existing screens rss behavior.
use super::*;

impl TuiApp {
    pub(super) fn load_rss(&mut self) -> Cmd<Msg> {
        if self.rss_busy || self.rss_loading { return Cmd::none(); }
        self.rss_loading = true;
        let revision = self.rss_revision;
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            Msg::Rss { revision, result: c.login().and_then(|_| c.rss_feeds()).map_err(|e| e.to_string()) }
        })
    }

    /// Execute the feed mutation and subsequent reload in the same task.
    pub(super) fn rss_action<F>(&mut self, label: impl Into<String>, action: F) -> Cmd<Msg>
    where F: FnOnce(&Client) -> Result<()> + Send + 'static {
        if self.rss_busy { return Cmd::none(); }
        self.rss_busy = true;
        self.rss_loading = false;
        self.rss_revision += 1;
        self.rss_retry_until = Some(std::time::Instant::now() + Duration::from_secs(45));
        let revision = self.rss_revision;
        let cfg = self.cfg.clone();
        let label = label.into();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let (result, error) = match c.login() {
                Ok(()) => {
                    let error = action(&c).err().map(|error| error.to_string());
                    (c.rss_feeds().map_err(|error| error.to_string()), error)
                }
                Err(error) => (Err(error.to_string()), None),
            };
            Msg::RssAction { revision, label, result, error }
        })
    }

    pub(super) fn refresh_rss(&mut self) -> Cmd<Msg> {
        if self.rss_busy { return Cmd::none(); }
        let names: Vec<String> = self.feeds.iter().map(|f| f.name.clone()).collect();
        if names.is_empty() { return Cmd::none(); }
        self.set_status("refreshing feeds…", false);
        self.rss_action("feeds refreshed", move |c| {
            let mut errors = Vec::new();
            for name in names {
                if let Err(error) = c.rss_refresh(&name) { errors.push(format!("{name}: {error}")); }
            }
            if errors.is_empty() { Ok(()) } else { anyhow::bail!(errors.join("; ")) }
        })
    }

    pub(super) fn selected_feed(&self) -> Option<&RssFeed> {
        self.feed_list.borrow().selected().and_then(|i| self.feeds.get(i))
    }

    #[cfg(test)]
    pub(super) fn on_rss_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Rss), key)
    }

    pub(super) fn on_rss_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let nf = self.feeds.len();
        let ni = self.selected_feed().map(|f| f.articles.len()).unwrap_or(0);
        match intent {
            actions::Intent::Left => self.rss_focus_items = false,
            actions::Intent::Right => { if ni > 0 { self.rss_focus_items = true; if self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } } }
            actions::Intent::Down => {
                if self.rss_focus_items { if ni > 0 { self.item_list.borrow_mut().select_next(ni); } }
                else if nf > 0 { self.feed_list.borrow_mut().select_next(nf); self.item_list.borrow_mut().select(None); }
            }
            actions::Intent::Up => {
                if self.rss_focus_items { self.item_list.borrow_mut().select_previous(); }
                else { self.feed_list.borrow_mut().select_previous(); self.item_list.borrow_mut().select(None); }
            }
            actions::Intent::First | actions::Intent::Last => {
                let (state,count)=if self.rss_focus_items { (&self.item_list,ni) } else { (&self.feed_list,nf) };
                state.borrow_mut().select(if intent==actions::Intent::First { (count>0).then_some(0) } else { count.checked_sub(1) });
                if !self.rss_focus_items { self.item_list.borrow_mut().select(None); }
            }
            actions::Intent::New => { self.open_prompt(PromptKind::AddFeed, "Add RSS feed URL", false); }
            actions::Intent::Remove => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                self.open_prompt(PromptKind::ConfirmRemoveFeed { name: name.clone() }, &format!("Remove feed '{name}'? y/N"), true);
            },
            actions::Intent::Refresh => return self.refresh_rss(),
            actions::Intent::Reload => return self.load_rss(),
            actions::Intent::Mark => {
                if !self.rss_focus_items {
                    self.set_status("Enter opens items · Shift+M marks this feed's items read", false);
                } else if let Some((feed, id, read)) = self.selected_feed().and_then(|f| self.item_list.borrow().selected().and_then(|i| f.articles.get(i)).map(|a| (f.name.clone(), a.id.clone(), !self.cfg.native() || !a.is_read))) {
                    return self.rss_action(format!("item marked {}", if read { "read" } else { "unread" }), move |c| c.rss_set_read(&feed, &id, read));
                }
            },
            actions::Intent::MarkAll => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                return self.rss_action(format!("marked {name} as read"), move |c| c.rss_mark_read(&name, None));
            },
            actions::Intent::Primary if !self.rss_focus_items => {
                if ni > 0 { self.rss_focus_items = true; if self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } }
            },
            actions::Intent::Primary => if self.rss_focus_items {
                let selected = self.item_list.borrow().selected();
                if let (Some(f), Some(i)) = (self.selected_feed(), selected) {
                    if let Some(a) = f.articles.get(i) {
                        let url = if a.torrent_url.is_empty() { a.link.clone() } else { a.torrent_url.clone() };
                        let (feed, id, label) = (f.name.clone(), a.id.clone(), short(&a.title, 40));
                        let opts = self.add_opts();
                        return self.rss_action(format!("added {label}"), move |c| { c.add_url(&url, &opts)?; c.rss_mark_read(&feed, Some(&id)) });
                    }
                }
            },
            actions::Intent::Imdb => if self.rss_focus_items {
                let idx = self.item_list.borrow().selected();
                let name = idx.and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| a.title.clone()));
                if let Some(name) = name { return self.lookup_imdb(name, None); }
            },
            actions::Intent::Follow => if self.rss_focus_items {
                let idx = self.item_list.borrow().selected();
                let name = idx.and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| a.title.clone()));
                if let Some(name) = name { return self.follow(name, None); }
            },
            _ => {}
        }
        Cmd::none()
    }

    pub(super) fn view_rss(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Percentage(34.0), Constraint::Fill]).split(area);
        let title = format!(" Feeds ({}) ", self.feeds.len());
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.rss_focus_items { th.border() } else { th.border_focused() }).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);
        if self.feeds.is_empty() {
            Paragraph::new("No feeds yet.\n\n(N)ew feed: add an RSS URL").style(th.dimmed()).render(pad(inner), frame);
        } else {
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.feeds.iter().enumerate().map(|(i, f)| {
                let unread = f.articles.iter().filter(|a| !a.is_read).count();
                let w = inner.width.saturating_sub(2) as usize;
                let count = format!(" {unread}/{}", f.articles.len());
                let name = readable_cell(&f.name, w.saturating_sub(count.len() + 1),
                    hovered_for(&self.feed_list,i,elapsed));
                let padn = 1;
                let mut spans = vec![Span::styled(name, if unread > 0 { th.text().bold() } else { th.text() }), Span::raw(" ".repeat(padn)), Span::styled(count, th.dimmed())];
                if f.has_error { spans.push(Span::styled(" !", th.err())); }
                li(Line::from_spans(spans))
            }).collect();
            let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
            StatefulWidget::render(&list, inner, frame, &mut self.feed_list.borrow_mut());
        }

        let feed = self.selected_feed();
        let title = match feed { Some(f) => format!(" {} ", if f.title.is_empty() { f.name.clone() } else { f.title.clone() }), None => " Items ".into() };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.rss_focus_items { th.border_focused() } else { th.border() }).title(&title);
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        let Some(f) = feed else { return };
        if f.articles.is_empty() {
            Paragraph::new(if f.is_loading { "Loading…" } else { "No items.\n(R)efresh feeds" }).style(th.dimmed()).render(pad(inner), frame);
            return;
        }
        let w = inner.width.saturating_sub(2) as usize;
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = f.articles.iter().enumerate().map(|(i, a)| {
            let date = if a.ts > 0 { ymd(a.ts) } else { a.date.chars().take(11).collect() };
            let size = a.size.map(human_bytes).unwrap_or_default();
            let tail = format!(" {size:>9}  {date}");
            let title = readable_cell(&a.title, w.saturating_sub(tail.len() + 2),
                hovered_for(&self.item_list,i,elapsed));
            let padn = 0;
            li(Line::from_spans([
                Span::styled(if a.is_read { "  " } else { "● " }, th.accented()),
                Span::styled(title, if a.is_read { th.dimmed() } else { th.text() }),
                Span::raw(" ".repeat(padn)),
                Span::styled(tail, th.dimmed()),
            ]))
        }).collect();
        let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
        StatefulWidget::render(&list, inner, frame, &mut self.item_list.borrow_mut());
    }
}
