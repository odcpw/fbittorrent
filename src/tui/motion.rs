//! Existing motion behavior.
use super::*;

impl TuiApp {
    /// Title under the cursor on the current screen: (lookup key, IMDb id if known).
    pub(super) fn hovered_title(&self) -> Option<(String, Option<String>)> {
        if self.files.is_some() || self.providers.is_some() || self.settings.is_some() || self.prompt.is_some() || self.show_help { return None; }
        if let Some((list, index)) = self.pointer_row() {
            let preview = match list {
                MouseList::Torrents => self.torrents.get(index).map(|t| (t.name.clone(), None)),
                MouseList::Results => self.search_results.get(index).map(|r| (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()))),
                MouseList::Items => self.selected_feed().and_then(|f| f.articles.get(index)).map(|a| (a.title.clone(), None)),
                MouseList::Shows => self.show_at(index).map(|s| (s.label(), Some(s.imdb.clone()))),
                MouseList::Episodes => self.episode_at(index).map(|(s, _)| (s.label(), Some(s.imdb.clone()))),
                _ => None,
            };
            if preview.is_some() { return preview; }
        }
        match self.screen {
            Screen::Torrents => self.selected_torrent().map(|t| (t.name.clone(), None)),
            Screen::Search if !self.search_browse && !self.search_focus_input =>
                self.selected_result().map(|r| (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()))),
            Screen::Rss if self.rss_focus_items => {
                let i = self.item_list.borrow().selected()?;
                self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| (a.title.clone(), None))
            }
            Screen::Shows => self.release_show().map(|s| (s.label(), Some(s.imdb.clone()))),
            _ => None,
        }
    }

    /// Debounced auto-lookup: fire once the cursor has rested ~350 ms on a title.
    pub(super) fn on_hover(&mut self) -> Cmd<Msg> {
        self.sync_marquee();
        let Some((key, id)) = self.hovered_title() else { self.hover_key = None; return Cmd::none() };
        if self.hover_key.as_deref() != Some(&key) {
            self.hover_key = Some(key);
            self.hover_since = std::time::Instant::now();
            return Cmd::none();
        }
        if self.hover_since.elapsed() < Duration::from_millis(350) || self.imdb.contains_key(&key) { return Cmd::none(); }
        self.lookup_imdb(key, id)
    }

    pub(super) fn marquee_target(&self) -> Option<String> {
        if self.show_help || self.prompt.is_some() || self.settings.is_some() { return None; }
        if let Some((list, index)) = self.pointer_row() {
            let text = match list {
                MouseList::Torrents => self.torrents.get(index).map(|t| t.name.clone()),
                MouseList::Results => self.search_results.get(index).map(|r| r.file_name.clone()),
                MouseList::Browse => self.browse_entries.get(index).map(|e| e.label.to_owned()),
                MouseList::Feeds => self.feeds.get(index).map(|f| f.name.clone()),
                MouseList::Items => self.selected_feed().and_then(|f| f.articles.get(index)).map(|a| a.title.clone()),
                MouseList::Shows => if index == 0 { Some("All unread".into()) } else { self.show_at(index).map(Show::label) },
                MouseList::Episodes => self.episode_at(index).map(|(s, e)| if self.shows_inbox() { format!("{} · {} · {}", s.label(), e.key, e.title) } else { e.title.clone() }),
                MouseList::Files => self.files.as_ref().and_then(|m| m.files.get(index)).map(|f| f.name.clone()),
                MouseList::Providers => self.providers.as_ref().and_then(|m| m.plugins.get(index)).map(|p| p.full_name.clone()),
            };
            if let Some(text) = text { return Some(format!("hover:{list:?}:{index}:{text}")); }
        }
        if let Some(m) = &self.providers {
            return m.list.borrow().selected().and_then(|i| m.plugins.get(i))
                .map(|p| format!("provider:{}:{}", p.name, p.full_name));
        }
        if let Some(m) = &self.files {
            return m.list.borrow().selected().and_then(|i| m.files.get(i))
                .map(|f| format!("file:{}:{}:{}", m.hash, f.index, f.name));
        }
        match self.screen {
            Screen::Torrents => self.selected_torrent().map(|t| format!("torrent:{}:{}", t.hash, t.name)),
            Screen::Search if self.search_browse => self.browse_list.borrow().selected()
                .and_then(|i| self.browse_entries.get(i)).map(|e| format!("browse:{}:{}", e.provider, e.label)),
            Screen::Search => self.selected_result().map(|r| format!("result:{}:{}", r.file_url, r.file_name)),
            Screen::Rss if self.rss_focus_items => self.item_list.borrow().selected()
                .and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i).map(|a| format!("item:{}:{}", f.name, a.id)))),
            Screen::Rss => self.selected_feed().map(|f| format!("feed:{}", f.name)),
            Screen::Shows if self.shows_focus_eps => self.selected_episode().map(|(s, e)| format!("show:{}:{}", s.imdb, e.key)),
            Screen::Shows => self.selected_show().map(|s| format!("show:{}", s.imdb)),
        }
    }

    pub(super) fn sync_marquee(&mut self) {
        let key = self.marquee_target();
        if key != self.marquee_key {
            self.marquee_key = key;
            self.marquee_since = std::time::Instant::now();
        }
    }

    pub(super) fn marquee_elapsed(&self) -> Duration {
        let target = self.marquee_target();
        if target.is_some() && target == self.marquee_key { self.marquee_since.elapsed() } else { Duration::ZERO }
    }
}

