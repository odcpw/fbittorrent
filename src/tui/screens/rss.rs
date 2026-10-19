//! Existing screens rss behavior.
use super::*;

impl TuiApp {
    pub(super) fn load_rss(&mut self) -> Cmd<Msg> {
        self.sync_topics()
    }

    pub(super) fn refresh_rss(&mut self) -> Cmd<Msg> {
        self.service_action("rss.refresh",serde_json::json!({}),"feeds refreshed")
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
            actions::Intent::Right => { if nf > 0 { self.rss_focus_items = true; if ni>0 && self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } } }
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
                } else if let Some((feed, id, read)) = self.selected_feed().and_then(|f| self.item_list.borrow().selected().and_then(|i| f.articles.get(i)).map(|a| (f.name.clone(), a.id.clone(), !a.is_read))) {
                    return self.native_feed_action("rss.set_read",&feed,Some(&id),Some(read),format!("item marked {}",if read{"read"}else{"unread"}));
                }
            },
            actions::Intent::MarkAll => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                return self.native_feed_action("rss.mark_feed_read",&name,None,None,format!("marked {name} as read"));
            },
            actions::Intent::Primary if !self.rss_focus_items => {
                if nf > 0 { self.rss_focus_items = true; if ni>0 && self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } }
            },
            actions::Intent::Primary => if self.rss_focus_items {
                let selected = self.item_list.borrow().selected();
                if let (Some(f), Some(i)) = (self.selected_feed(), selected) {
                    if let Some(a) = f.articles.get(i) {
                        let (feed, id, label) = (f.name.clone(), a.id.clone(), short(&a.title, 40));
                        return self.native_feed_action("rss.grab",&feed,Some(&id),None,format!("added {label}"));
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
        let tier = layout::Tier::at(area.width);
        let cols = if tier == layout::Tier::Narrow {
            if self.rss_focus_items {
                [Rect::default(), area]
            } else {
                [area, Rect::default()]
            }
        } else {
            let split = Flex::horizontal()
                .constraints([Constraint::Percentage(34.0), Constraint::Fill])
                .split(area);
            [split[0], split[1]]
        };
        if !cols[0].is_empty() {
            let title = format!(" RSS › Feeds ({}) ", self.feeds.len());
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .padding(Sides::all(0))
                .border_style(if self.rss_focus_items {
                    th.border()
                } else {
                    th.border_focused()
                })
                .title(&title);
            let inner = block.inner(cols[0]);
            block.render(cols[0], frame);
            if self.feeds.is_empty() {
                Paragraph::new("No feeds yet.\n\n(n) New feed: add an RSS URL")
                    .style(th.dimmed())
                    .render(pad(inner), frame);
            } else {
                let elapsed = self.marquee_elapsed();
                let width = inner.width.saturating_sub(2) as usize;
                self.view_rows(
                    MouseList::Feeds,
                    inner,
                    self.feeds.len(),
                    |i| {
                        let f = &self.feeds[i];
                        layout::fingerprint(&(
                            &f.name,
                            f.articles.len(),
                            self.feed_counts(f),
                            f.has_error,
                            f.is_loading,
                        ))
                    },
                    |i| {
                        let f = &self.feeds[i];
                        let (unread,total)=self.feed_counts(f);
                        let count=format!(" {}{}",total.map_or_else(||format!("{unread} unread"),|n|format!("{unread}/{n}")),if f.has_error{" !"}else{""});
                        Line::from_spans([
                            Span::styled(
                                readable_cell(
                                    &f.name,
                                    width.saturating_sub(dw(&count)),
                                    hovered_for(&self.feed_list, i, elapsed),
                                ),
                                if unread > 0 {
                                    th.text().bold()
                                } else {
                                    th.text()
                                },
                            ),
                            Span::styled(count, th.dimmed()),
                        ])
                    },
                    frame,
                );
            }
        }
        if cols[1].is_empty() {
            return;
        }
        let feed = self.selected_feed();
        let title = feed
            .map(|f| {
                format!(
                    " RSS › {} ",
                    if f.title.is_empty() {
                        &f.name
                    } else {
                        &f.title
                    }
                )
            })
            .unwrap_or_else(|| " RSS › Items ".into());
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .padding(Sides::all(0))
            .border_style(if self.rss_focus_items {
                th.border_focused()
            } else {
                th.border()
            })
            .title(&title);
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        let Some(feed) = feed else {
            return;
        };
        if feed.articles.is_empty() {
            let pending=self.store.hello.is_some() && !self.store.rss_items.contains_key(&feed.uid);
            Paragraph::new(if feed.is_loading || pending {
                "Loading…"
            } else {
                "No items.\n(r) Refresh feeds"
            })
            .style(th.dimmed())
            .render(pad(inner), frame);
            return;
        }
        let sizes = feed.articles.iter().any(|a| a.size.is_some());
        let width = inner.width.saturating_sub(2) as usize;
        let elapsed = self.marquee_elapsed();
        self.view_rows(
            MouseList::Items,
            inner,
            feed.articles.len(),
            |i| layout::fingerprint(&(&feed.articles[i], sizes, tier.extended())),
            |i| {
                let article = &feed.articles[i];
                let key = crate::release::parse(&article.title)
                    .episode
                    .unwrap_or_default();
                let date = if tier.extended() {
                    format!(
                        "  {}",
                        if article.ts > 0 {
                            ymd(article.ts)
                        } else {
                            article.date.chars().take(10).collect()
                        }
                    )
                } else {
                    String::new()
                };
                let size = if sizes {
                    format!(" {:>9}", article.size.map(human_bytes).unwrap_or_default())
                } else {
                    String::new()
                };
                let key = if key.is_empty() {
                    String::new()
                } else {
                    format!("{key} ")
                };
                let name_width = width.saturating_sub(9 + dw(&key) + dw(&size) + dw(&date));
                let style = if article.is_read {
                    th.dimmed()
                } else {
                    th.text()
                };
                Line::from_spans([
                    Span::styled(
                        if article.is_read {
                            "✓ Read   "
                        } else {
                            "● Unread "
                        },
                        style,
                    ),
                    Span::styled(key, style),
                    Span::styled(
                        readable_cell(
                            &article.title,
                            name_width,
                            hovered_for(&self.item_list, i, elapsed),
                        ),
                        style,
                    ),
                    Span::styled(format!("{size}{date}"), th.dimmed()),
                ])
            },
            frame,
        );
    }
}
