//! Existing mouse behavior.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseList { Torrents, Results, Browse, Feeds, Items, Shows, Episodes, Files, Providers, Settings }

impl TuiApp {
    pub(super) fn mouse_list_at(&self, x: u16, y: u16) -> Option<(MouseList, Rect)> {
        if self.log_open||self.feedback.rects.get().iter().flatten().any(|a|a.contains(x,y)){return None;}
        let b = self.last_bounds.get();
        if self.screen==Screen::Search && self.poster_open {return None;}
        if b.width == 0 || self.show_help || self.prompt.is_some() || self.settings.is_some() { return None; }
        if let Some(m) = &self.providers {
            let area = providers_area(b, m.plugins.len());
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
            return rows[0].contains(x, y).then_some((MouseList::Providers, rows[0]));
        }
        if self.details.is_some() {
            return self.rects.borrow().files.filter(|area|area.contains(x,y)).map(|area|(MouseList::Files,area));
        }
        if self.files.is_some() {
            let area = files_area(b);
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(1)]).split(inner);
            return rows[1].contains(x, y).then_some((MouseList::Files, rows[1]));
        }
        {
            let geometry=self.rects.borrow();
            if geometry.lists.iter().any(Option::is_some) {
                return [MouseList::Results,MouseList::Browse,MouseList::Feeds,MouseList::Items,MouseList::Shows,MouseList::Episodes].into_iter()
                    .find_map(|list|geometry.lists[list as usize].filter(|a|a.contains(x,y)).map(|a|(list,a)));
            }
        }
        let root = Flex::vertical().constraints([Constraint::Fixed(layout::header_height(b.height)), Constraint::Fill, Constraint::Fixed(0), Constraint::Fixed(2)]).split(b);
        match self.screen {
            Screen::Torrents => {
                if let Some(area)=self.rects.borrow().table {return area.contains(x,y).then_some((MouseList::Torrents,area));}
                let cols = torrent_columns(root[1]);
                let inner = Block::bordered().inner(cols[0]);
                let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
                rows[1].contains(x, y).then_some((MouseList::Torrents, rows[1]))
            }
            Screen::Search => {
                let rows = Flex::vertical().constraints([Constraint::Fixed(3), Constraint::Fill]).split(root[1]);
                let cols = detail_columns(rows[1]);
                let inner = Block::bordered().inner(cols[0]);
                if self.search_browse { inner.contains(x, y).then_some((MouseList::Browse, inner)) }
                else {
                    let list_rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
                    list_rows[1].contains(x, y).then_some((MouseList::Results, list_rows[1]))
                }
            }
            Screen::Rss => {
                let cols = Flex::horizontal().constraints([Constraint::Percentage(34.0), Constraint::Fill]).split(root[1]);
                let feeds = Block::bordered().inner(cols[0]);
                let items = Block::bordered().inner(cols[1]);
                if feeds.contains(x, y) { Some((MouseList::Feeds, feeds)) }
                else { items.contains(x, y).then_some((MouseList::Items, items)) }
            }
            Screen::Shows => {
                let [shows, releases] = self.shows_rects.get();
                if shows.contains(x, y) { Some((MouseList::Shows, shows)) }
                else { releases.contains(x, y).then_some((MouseList::Episodes, releases)) }
            }
        }
    }

    pub(super) fn mouse_list_state(&self, list: MouseList) -> &RefCell<ListState> {
        match list {
            MouseList::Torrents => &self.tor_list,
            MouseList::Results => &self.search_list,
            MouseList::Browse => &self.browse_list,
            MouseList::Feeds => &self.feed_list,
            MouseList::Items => &self.item_list,
            MouseList::Shows => &self.show_list,
            MouseList::Episodes => &self.ep_list,
            MouseList::Files => &self.files.as_ref().expect("files modal").list,
            MouseList::Providers => &self.providers.as_ref().expect("providers modal").list,
            MouseList::Settings => &self.settings.as_ref().expect("settings modal").list,
        }
    }

    pub(super) fn clear_pointer_hover(&self) {
        for state in [&self.tor_list, &self.search_list, &self.browse_list, &self.feed_list, &self.item_list, &self.show_list, &self.ep_list] {
            state.borrow_mut().hovered = None;
        }
        if let Some(modal) = &self.files { modal.list.borrow_mut().hovered = None; }
        if let Some(modal) = &self.providers { modal.list.borrow_mut().hovered = None; }
    }

    pub(super) fn pointer_row(&self) -> Option<(MouseList, usize)> {
        if let Some(modal) = &self.files { return modal.list.borrow().hovered.map(|i| (MouseList::Files, i)); }
        if let Some(modal) = &self.providers { return modal.list.borrow().hovered.map(|i| (MouseList::Providers, i)); }
        [MouseList::Torrents, MouseList::Results, MouseList::Browse, MouseList::Feeds, MouseList::Items, MouseList::Shows, MouseList::Episodes]
            .into_iter().find_map(|list| self.mouse_list_state(list).borrow().hovered.map(|i| (list, i)))
    }

    pub(super) fn mouse_list_len(&self, list: MouseList) -> usize {
        match list {
            MouseList::Torrents => self.torrents.len(),
            MouseList::Results => self.search_results.len(),
            MouseList::Browse => self.browse_entries.len(),
            MouseList::Feeds => self.feeds.len(),
            MouseList::Items => self.selected_feed().map_or(0, |f| f.articles.len()),
            MouseList::Shows => self.shows.shows.len() + 1,
            MouseList::Episodes => self.episode_count(),
            MouseList::Files => self.files.as_ref().map_or(0, |m| m.files.len()),
            MouseList::Providers => self.providers.as_ref().map_or(0, |m| m.plugins.len()),
            MouseList::Settings => self.settings.as_ref().map_or(0, |_|settings_fields::SETTINGS_ROWS),
        }
    }

    /// A double click must activate the same object, even if a refresh replaced its row.
    pub(super) fn mouse_item_identity(&self, list: MouseList, index: usize) -> Option<String> {
        match list {
            MouseList::Torrents => self.torrents.get(index).map(|torrent| torrent.hash.to_string()),
            MouseList::Results => self.search_results.get(index).map(|result| result.file_url.clone()),
            MouseList::Browse => self.browse_entries.get(index).map(|entry| format!("{:?}", entry.source)),
            MouseList::Feeds => self.feeds.get(index).map(|feed| if feed.uid.is_empty() { feed.name.clone() } else { feed.uid.clone() }),
            MouseList::Items => self.selected_feed().and_then(|feed| feed.articles.get(index).map(|article|
                format!("{:?}", (if feed.uid.is_empty() { &feed.name } else { &feed.uid }, &article.id)))),
            MouseList::Shows => if index == 0 { Some("all-unread".into()) } else { self.show_at(index).map(|show| show.imdb.clone()) },
            MouseList::Episodes => self.episode_at(index).map(|(show, episode)| format!("{:?}", (&show.imdb, &episode.key))),
            MouseList::Files => self.files.as_ref().and_then(|modal| modal.files.get(index).map(|file| format!("{:?}", (&modal.hash, file.index)))),
            MouseList::Providers => self.providers.as_ref().and_then(|modal| modal.plugins.get(index).map(|provider| provider.name.clone())),
            MouseList::Settings => Some(index.to_string()),
        }
    }

    pub(super) fn focus_mouse_list(&mut self, list: MouseList) {
        if matches!(list, MouseList::Results | MouseList::Browse) {
            self.search_focus_input = false;
            self.search_input.set_focused(false);
        }
        if matches!(list, MouseList::Feeds | MouseList::Items) { self.rss_focus_items = list == MouseList::Items; }
        if matches!(list, MouseList::Shows | MouseList::Episodes) { self.shows_focus_eps = list == MouseList::Episodes; }
    }

    /// Hover previews without changing the keyboard target; clicks select and double clicks activate.
    pub(super) fn on_mouse(&mut self, m: ftui::MouseEvent) -> Cmd<Msg> {
        use ftui::{MouseButton, MouseEventKind};
        if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) { self.clear_pointer_hover(); }
        if self.settings.is_some() {
            let area = settings_area(self.last_bounds.get());
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Rect::new(inner.x, inner.y, inner.width, inner.height.min(settings_fields::SETTINGS_ROWS as u16));
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                if rows.contains(m.x, m.y) {
                    let offset=self.settings.as_ref().unwrap().list.borrow().offset;
                    self.settings.as_ref().unwrap().list.borrow_mut().select(Some((offset+usize::from(m.y - rows.y)).min(settings_fields::SETTINGS_ROWS-1)));
                    return self.on_settings_key(KeyEvent::new(KeyCode::Right));
                }
                if inner.contains(m.x, m.y) && m.y == inner.y.saturating_add(settings_fields::SETTINGS_ROWS as u16) {
                    return self.on_settings_key(KeyEvent::new(KeyCode::Enter));
                }
            }
            if m.kind == MouseEventKind::ScrollDown { return self.on_settings_key(KeyEvent::new(KeyCode::Down)); }
            if m.kind == MouseEventKind::ScrollUp { return self.on_settings_key(KeyEvent::new(KeyCode::Up)); }
            return Cmd::none();
        }
        if self.log_open {return match m.kind{MouseEventKind::ScrollDown=>self.on_log_action(actions::Intent::Down),MouseEventKind::ScrollUp=>self.on_log_action(actions::Intent::Up),_=>Cmd::none()};}
        if self.show_help {
            match m.kind {
                MouseEventKind::ScrollDown => self.help_scroll = self.help_scroll.saturating_add(3).min(self.help_max_scroll()),
                MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(3),
                MouseEventKind::Down(MouseButton::Left) => self.show_help = false,
                _ => {},
            }
            return Cmd::none();
        }
        if self.poster_open && self.prompt.is_none(){return Cmd::none();}
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.show_help { self.show_help = false; return Cmd::none(); }
            let b = self.last_bounds.get();
            if self.providers.is_some() && !providers_area(b, self.providers.as_ref().map_or(0, |p| p.plugins.len())).contains(m.x, m.y) {
                self.providers = None;
                return Cmd::none();
            }
            if self.details.is_none() && self.files.is_some() && !files_area(b).contains(m.x, m.y) {
                self.files = None;
                return Cmd::none();
            }
        }
        if self.details.is_some() && self.files.is_none() {return Cmd::none();}
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
            && self.details.is_none() && self.files.is_none() && self.providers.is_none() && self.prompt.is_none() {
            let rects = self.rects.borrow();
            let help = rects.help.contains(m.x, m.y);
            let tab = rects.tabs.iter().position(|area| area.contains(m.x, m.y));
            let query = rects.query.is_some_and(|area| area.contains(m.x, m.y));
            drop(rects);
            if help {
                return actions::dispatch_scope(self, actions::Scope::Global, KeyEvent::new(KeyCode::Char('?')));
            }
            if let Some(index) = tab {
                self.last_click = None;
                return self.set_screen(Screen::from_index(index));
            }
            if self.screen == Screen::Search && query {
                self.search_focus_input = true;
                self.search_input.set_focused(true);
                return Cmd::none();
            }
        }
        if matches!(self.screen,Screen::Torrents|Screen::Search)&&self.files.is_none()&&self.providers.is_none()&&self.prompt.is_none()
            &&matches!(m.kind,MouseEventKind::Down(MouseButton::Left)) {
            let column=self.rects.borrow().headers.iter().find(|(_,area)|area.contains(m.x,m.y)).map(|(column,_)|*column);
            if let Some(column)=column {self.last_click=None;return if self.screen==Screen::Torrents {self.sort_torrent_column(column)} else {self.sort_search_column(column)};}
        }
        let Some((list, area)) = self.mouse_list_at(m.x, m.y) else { return Cmd::none() };
        if matches!(m.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) {
            self.focus_mouse_list(list);
            return self.on_key(KeyEvent::new(if m.kind == MouseEventKind::ScrollDown { KeyCode::Down } else { KeyCode::Up }));
        }
        if !matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) { return Cmd::none(); }
        let state = self.mouse_list_state(list);
        let index=if list==MouseList::Torrents&&self.rects.borrow().table.is_some(){
            self.rects.borrow().rows.iter().find(|(_,rect)|rect.contains(m.x,m.y))
                .and_then(|(hash,_)|self.torrents.iter().position(|t|t.hash.as_str()==hash))
        }else{mouse_row_index(area,m.x,m.y,state.borrow().offset,self.mouse_list_len(list))};
        let Some(index)=index else{return Cmd::none()};
        if m.kind == MouseEventKind::Moved {
            state.borrow_mut().hovered = Some(index);
            return Cmd::none();
        }
        let was_selected = state.borrow().selected() == Some(index);
        if !was_selected {
            state.borrow_mut().select(Some(index));
            if list == MouseList::Feeds { self.item_list.borrow_mut().select(None); }
            if list == MouseList::Shows { self.select_show_parent(index); }
        }
        self.focus_mouse_list(list);
        let Some(identity) = self.mouse_item_identity(list, index) else { self.last_click = None; return Cmd::none() };
        let now = std::time::Instant::now();
        let double = self.last_click.as_ref().is_some_and(|(l, previous, at)| *l == list && *previous == identity && now.duration_since(*at) < Duration::from_millis(450));
        self.last_click = if double { None } else { Some((list, identity, now)) };
        if double {
            if list == MouseList::Feeds {
                self.rss_focus_items = true;
                if self.mouse_list_len(MouseList::Items) > 0 { self.item_list.borrow_mut().select(Some(0)); }
                return Cmd::none();
            }
            if list == MouseList::Shows {
                self.shows_focus_eps = true;
                if self.mouse_list_len(MouseList::Episodes) > 0 { self.ep_list.borrow_mut().select(Some(0)); }
                return Cmd::none();
            }
            let key = if matches!(list, MouseList::Files | MouseList::Providers) { KeyCode::Char(' ') } else { KeyCode::Enter };
            return self.on_key(KeyEvent::new(key));
        }
        Cmd::none()
    }
}


pub(super) fn mouse_row_index(area: Rect, x: u16, y: u16, offset: usize, len: usize) -> Option<usize> {
    area.contains(x, y).then(|| offset + usize::from(y - area.y)).filter(|&i| i < len)
}
