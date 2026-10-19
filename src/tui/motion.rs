//! Existing motion behavior.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MarqueeTarget { list: MouseList, index: usize, revision: u64 }


impl TuiApp {
    /// Title under the cursor on the current screen: (lookup key, IMDb id if known).
    pub(super) fn hovered_title(&self) -> Option<(String, Option<String>)> {
        if self.poster_open {return self.poster_target.clone();}
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

    /// Debounced auto-lookup: fire once the cursor has rested ~300 ms on a title.
    pub(super) fn on_hover(&mut self) -> Cmd<Msg> {
        self.sync_marquee();
        let Some((key, id)) = self.hovered_title() else { self.hover_key = None; return Cmd::none() };
        if self.hover_key.as_deref() != Some(&key) {
            self.hover_key = Some(key);
            self.hover_since = std::time::Instant::now();
            return Cmd::none();
        }
        if self.hover_since.elapsed() < Duration::from_millis(300) || self.imdb_state(&key).is_some() { return Cmd::none(); }
        self.lookup_imdb(key, id)
    }

    pub(super) fn marquee_target(&self) -> Option<MarqueeTarget> {
        use std::hash::{Hash, Hasher};
        if self.poster_open || self.cfg.reduce_motion || self.show_help || self.prompt.is_some() || self.settings.is_some() { return None; }
        let (list,index)=self.pointer_row()?;
        if self.mouse_list_state(list).borrow().selected()==Some(index) { return None; }
        // Borrow stable identifiers instead of allocating a formatted target on
        // every frame. Metadata replacement under a stationary pointer resets it.
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        match list {
            MouseList::Torrents=>{ let row=self.torrents.get(index)?; (&row.hash,&row.name).hash(&mut hash); }
            MouseList::Results=>{ let row=self.search_results.get(index)?; (&row.file_url,&row.file_name).hash(&mut hash); }
            MouseList::Browse=>{ let row=self.browse_entries.get(index)?; (row.provider,row.label).hash(&mut hash); }
            MouseList::Feeds=>{ self.feeds.get(index)?.name.hash(&mut hash); }
            MouseList::Items=>{ let row=self.selected_feed()?.articles.get(index)?; (&row.id,&row.title).hash(&mut hash); }
            MouseList::Shows=>{ if index>0 { let row=self.show_at(index)?; (&row.imdb,&row.title).hash(&mut hash); } }
            MouseList::Episodes=>{ let (show,row)=self.episode_at(index)?; (&show.imdb,&row.key,&row.title).hash(&mut hash); }
            MouseList::Files=>{ let modal=self.files.as_ref()?; let row=modal.files.get(index)?; (&modal.hash,row.index,&row.name).hash(&mut hash); }
            MouseList::Settings=>return None,
            MouseList::Providers=>{ let row=self.providers.as_ref()?.plugins.get(index)?; (&row.name,&row.full_name).hash(&mut hash); }
        }
        Some(MarqueeTarget { list,index,revision:hash.finish() })
    }

    pub(super) fn sync_marquee(&mut self) {
        let key = self.marquee_target();
        if key != self.marquee_key {
            self.marquee_key = key;
            self.marquee_since = std::time::Instant::now();
        }
    }

    pub(super) fn marquee_elapsed(&self) -> Option<Duration> {
        let target=self.marquee_target();
        (target.is_some() && target==self.marquee_key).then(||self.marquee_since.elapsed())
    }
}
