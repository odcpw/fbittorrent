//! The fbittorrent terminal UI (FrankenTUI).

mod poster;
mod theme;
pub(crate) mod signals;
mod prelude;
use prelude::*;
mod app;
pub use app::{TuiApp, Screen, Msg, ImdbState};
use app::*;
mod connection;
#[path = "screens/torrents.rs"]
mod screens_torrents;
#[path = "screens/search.rs"]
mod screens_search;
use screens_search::*;
#[path = "screens/rss.rs"]
mod screens_rss;
#[path = "screens/shows.rs"]
mod screens_shows;
#[path = "modals/files.rs"]
mod modals_files;
use modals_files::*;
#[path = "modals/providers.rs"]
mod modals_providers;
use modals_providers::*;
#[path = "modals/settings.rs"]
mod modals_settings;
use modals_settings::*;
#[path = "modals/prompt.rs"]
mod modals_prompt;
use modals_prompt::*;
mod imdb_card;
mod motion;
mod mouse;
use mouse::*;
mod layout;
use layout::*;
mod footer;
mod help;
mod text;
use text::*;
pub use connection::run;
pub(crate) use connection::SignalTermination;
mod actions;
mod store;
mod format;
mod palette;
#[cfg(test)]
#[path = "tests/interaction.rs"]
mod interaction_tests;
#[cfg(test)]
#[path = "tests/golden.rs"]
mod golden;

impl Model for TuiApp {
    type Message = Msg;

    fn on_shutdown(&mut self) -> Cmd<Msg> {
        signals::event("shutdown_once begin (model hook)");
        self.acknowledge_signal();
        Cmd::none()
    }

    fn init(&mut self) -> Cmd<Msg> {
        if self.pointer_motion_on_start {
            // FrankenTUI's default captures clicks and dragging (1002), but
            // ordinary hover needs 1003. Init runs after terminal setup and
            // before rendering; TerminalSession also resets 1003 on cleanup.
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            if let Err(error) = out.write_all(b"\x1b[?1003h").and_then(|_| out.flush()) {
                self.set_status(format!("mouse hover: {error}"), true);
            }
        }
        self.rss_loaded = true;
        let mut cmds = vec![self.poll(), self.load_plugins(), self.load_ratings(), self.load_rss()];
        cmds.push(self.refresh_shows(false));
        Cmd::batch(cmds)
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Msg>>> {
        let mut v: Vec<Box<dyn Subscription<Msg>>> = vec![
            Box::new(Every::with_id(1, Duration::from_millis(2000), || Msg::Poll)),
            Box::new(Every::with_id(2, Duration::from_millis(300), || Msg::Hover)),
        ];
        if let Some((id, q)) = &self.search_sub {
            v.push(Box::new(SearchSub { id: *id, query: q.clone(), cfg: self.cfg.clone() }));
        }
        if self.rss_retry_until.is_some_and(|until| std::time::Instant::now() < until) && self.feeds.iter().any(|f| f.is_loading) {
            v.push(Box::new(Every::with_id(3, Duration::from_secs(1), || Msg::RssReload)));
        }
        v
    }

    fn update(&mut self, msg: Msg) -> Cmd<Msg> {
        match msg {
            Msg::Raw(Event::Key(k)) => self.on_key(k),
            Msg::Raw(Event::Mouse(m)) => self.on_mouse(m),
            Msg::Raw(Event::Resize { .. }) => {
                if let Some(k) = &self.kitty { k.borrow_mut().clear(); }
                Cmd::none()
            }
            Msg::Raw(Event::Paste(p)) => {
                let ev = Event::Paste(p);
                if let Some(pr) = self.prompt.as_mut() { if !pr.yes_no { pr.input.handle_event(&ev); } }
                else if self.screen == Screen::Search && self.search_focus_input { self.search_input.handle_event(&ev); }
                Cmd::none()
            }
            Msg::Raw(_) => Cmd::none(),
            Msg::Poll => {
                if self.cfg.native() && crate::network_app::check_transition(&self.cfg, &self.lifecycle) { return Cmd::quit(); }
                self.tick += 1; self.maybe_reload_theme(); self.poll()
            }
            Msg::Hover => {
                if self.acknowledge_signal() { return Cmd::quit(); }
                let ttl = if self.status_is_error { 15 } else { 8 };
                if !self.status.is_empty() && !self.search_running && self.status_at.elapsed() > Duration::from_secs(ttl) { self.status.clear(); }
                self.on_hover()
            }
            Msg::Ids(pairs) => {
                self.ids_inflight = false;
                for (n, id) in pairs { self.name_ids.insert(n, id); }
                // More unresolved rows may have arrived meanwhile.
                self.resolve_ids()
            }
            Msg::Ratings(r) => {
                match r { Ok(idx) => self.ratings = Some(idx), Err(e) => self.set_status(format!("ratings index: {e}"), true) }
                Cmd::none()
            }
            Msg::Torrents(r) => {
                self.poll_inflight = false;
                match r {
                    Ok((ts, ti)) => {
                        self.online = true;
                        self.backend_error.clear();
                        let sel_hash = self.selected_torrent().map(|t| t.hash.clone());
                        self.torrents = ts;
                        let mut st = self.tor_list.borrow_mut();
                        let cur = st.selected().unwrap_or(0);
                        match sel_hash.and_then(|h| self.torrents.iter().position(|t| t.hash == h)) {
                            Some(i) => st.select(Some(i)),
                            None if !self.torrents.is_empty() => st.select(Some(cur.min(self.torrents.len() - 1))),
                            None => st.select(None),
                        }
                        self.transfer = ti;
                    }
                    Err(e) => { self.online = false; self.backend_error = e; }
                }
                Cmd::none()
            }
            Msg::ActionDone(r) => {
                match r { Ok(s) => self.set_status(s, false), Err(e) => self.set_status(e, true) }
                self.poll()
            }
            Msg::Imdb(key, r) => {
                let st = match r {
                    Ok(Some((t, p))) => ImdbState::Ready(Box::new(t), p),
                    Ok(None) => ImdbState::Missing,
                    Err(e) => ImdbState::Failed(e),
                };
                self.imdb.insert(key, st);
                Cmd::none()
            }
            Msg::SearchBatch { id, results, done, error } => {
                if self.search_sub.as_ref().map(|(i, _)| *i) != Some(id) { return Cmd::none(); }
                if let Some(e) = error.as_ref() { self.set_status(format!("search failed: {e}"), true); }
                if !results.is_empty() || done {
                    let sel_url = self.selected_result().map(|r| r.file_url.clone());
                    let mut rs = dedupe_results(results);
                    rs.sort_by(|a, b| b.seeders.cmp(&a.seeders));
                    // Best 100 by seeders is all anyone reads; dedupe first so the cap
                    // is not spent on the same torrent from five providers.
                    rs.truncate(MAX_RESULTS);
                    if !rs.is_empty() { self.search_results = rs; }
                    let mut st = self.search_list.borrow_mut();
                    match sel_url.and_then(|u| self.search_results.iter().position(|r| r.file_url == u)) {
                        Some(i) => st.select(Some(i)),
                        None if !self.search_results.is_empty() && !self.search_focus_input => st.select(Some(0)),
                        _ => {}
                    }
                    drop(st);
                    if !done { return self.resolve_ids(); }
                }
                if done {
                    self.search_running = false;
                    self.search_sub = None;
                    let n = self.search_results.len();
                    match error {
                        Some(error) => self.set_status(format!("{n} results · some providers failed: {error}"), true),
                        None => self.set_status(format!("{n} result{}", if n == 1 { "" } else { "s" }), false),
                    }
                    return self.resolve_ids();
                }
                Cmd::none()
            }
            Msg::Browse(label, r) => {
                self.browse_loading = None;
                match r {
                    Ok(rs) => {
                        self.search_sub = None; self.search_running = false;
                        let n = rs.len();
                        self.search_results = rs;
                        self.search_browse = false;
                        self.search_list.borrow_mut().select(if n > 0 { Some(0) } else { None });
                        self.set_status(format!("{label}: {n} entries"), false);
                    }
                    Err(e) => self.set_status(e, true),
                }
                Cmd::none()
            }
            Msg::Plugins(r) => {
                match r {
                    Ok(mut ps) => {
                        ps.sort_by_key(|p| p.full_name.to_lowercase());
                        self.plugins = ps.clone();
                        if let Some(m) = self.providers.as_mut() {
                            m.loading = false;
                            m.error.clear();
                            let n = ps.len();
                            m.plugins = ps;
                            let mut st = m.list.borrow_mut();
                            match st.selected() { Some(i) if i < n => {}, _ => st.select(if n > 0 { Some(0) } else { None }) }
                        }
                    }
                    Err(e) => { if let Some(m) = self.providers.as_mut() { m.loading = false; m.error = e; } }
                }
                Cmd::none()
            }
            Msg::Files(hash, r) => {
                if let Some(m) = self.files.as_mut() {
                    if m.hash == hash {
                        m.loading = false;
                        match r {
                            Ok(fs) => {
                                let n = fs.len();
                                m.files = fs;
                                let mut st = m.list.borrow_mut();
                                match st.selected() {
                                    Some(i) if i < n => {}
                                    _ => st.select(if n > 0 { Some(0) } else { None }),
                                }
                            }
                            Err(e) => m.error = e,
                        }
                    }
                }
                Cmd::none()
            }
            Msg::Episodes(imdb, r) => {
                let selected_key = self.selected_episode().filter(|(s, _)| s.imdb == imdb).map(|(_, e)| e.key.clone());
                self.shows_refreshing.remove(&imdb);
                let Some(s) = self.shows.get_mut(&imdb) else { return Cmd::none() };
                match r {
                    Ok(check) => {
                        let had_any = !s.episodes.is_empty();
                        let fresh = s.apply_check(check);
                        let title = s.title.clone();
                        if self.show_list.borrow().selected() != Some(0) {
                            if let Some(key) = selected_key { self.ep_list.borrow_mut().select(s.episodes.iter().position(|e| e.key == key)); }
                        }
                        self.rebuild_inbox();
                        self.save_shows();
                        if fresh > 0 && had_any { self.set_status(format!("{title}: {fresh} new episode(s)"), false); }
                        else if self.shows_refreshing.is_empty() {
                            let warnings = self.shows.shows.iter().filter(|s| s.error.is_some() || !s.catalogue_complete).count();
                            self.set_status(format!("shows checked · {} unread{}", self.shows.new_total(), if warnings > 0 { format!(" · {warnings} source warning(s)") } else { String::new() }), warnings > 0);
                        }
                    }
                    Err(e) => { s.error = Some(e.clone()); self.save_shows(); self.set_status(format!("{}: {e}", short(&imdb, 12)), true); }
                }
                Cmd::none()
            }
            Msg::Followed(r) => {
                match r {
                    Ok(t) => {
                        let added = self.shows.follow(&t);
                        let label = match t.year { Some(y) => format!("{} ({y})", t.name), None => t.name.clone() };
                        if let Some(i) = self.shows.shows.iter().position(|s| s.imdb == t.id) { self.select_show_parent(i + 1); }
                        self.ep_list.borrow_mut().select(None);
                        self.rebuild_inbox();
                        self.save_shows();
                        self.set_status(if added { format!("following {label}") } else { format!("already following {label}") }, false);
                        if added { return self.refresh_show(t.id); }
                    }
                    Err(e) => self.set_status(format!("follow: {e}"), true),
                }
                Cmd::none()
            }
            Msg::ShowGrab { imdb, episode, result } => {
                self.show_grabbing.remove(&format!("{imdb}:{}", episode.key));
                match result {
                    Ok(()) => {
                        if let Some(show) = self.shows.get_mut(&imdb) { show.mark_grabbed(&episode.key, &episode.magnet); }
                        self.rebuild_inbox();
                        self.save_shows();
                        let title = self.shows.get(&imdb).map(|s| s.title.as_str()).unwrap_or(&imdb);
                        self.set_status(format!("added {title} {}", episode.key), false);
                    }
                    Err(e) => self.set_status(format!("adding {}: {e}", episode.key), true),
                }
                self.poll()
            }
            Msg::RssReload => self.load_rss(),
            Msg::RssAction { revision, label, result, error } => {
                if revision != self.rss_revision { return Cmd::none(); }
                self.rss_busy = false;
                let succeeded = result.is_ok() && error.is_none();
                let reload = self.update(Msg::Rss { revision, result });
                if succeeded { self.set_status(label, false); }
                if let Some(error) = error { self.set_status(error, true); }
                Cmd::batch(vec![reload, self.poll()])
            }
            Msg::Rss { revision, result } => {
                if revision != self.rss_revision { return Cmd::none(); }
                self.rss_loading = false;
                match result {
                    Ok(feeds) => {
                        let sel = self.selected_feed().map(|f| f.name.clone());
                        let article = self.item_list.borrow().selected().and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i))).map(|a| a.id.clone());
                        self.feeds = feeds;
                        let mut st = self.feed_list.borrow_mut();
                        match sel.and_then(|n| self.feeds.iter().position(|f| f.name == n)) {
                            Some(i) => st.select(Some(i)),
                            None => st.select(if self.feeds.is_empty() { None } else { Some(0) }),
                        }
                        drop(st);
                        let new_index = article.and_then(|id| self.selected_feed().and_then(|f| f.articles.iter().position(|a| a.id == id)));
                        self.item_list.borrow_mut().select(new_index.or_else(|| self.selected_feed().filter(|f| !f.articles.is_empty()).map(|_| 0)));
                        if self.feeds.iter().any(|f| f.is_loading) {
                            if self.rss_retry_until.is_none() { self.rss_retry_until = Some(std::time::Instant::now() + Duration::from_secs(45)); }
                        } else { self.rss_retry_until = None; }
                        if self.rss_initial_refresh_pending {
                            self.rss_initial_refresh_pending = false;
                            return self.refresh_rss();
                        }
                    }
                    Err(e) => self.set_status(e, true),
                }
                Cmd::none()
            }
        }
    }

    fn view(&self, frame: &mut Frame) {
        let th = &self.theme;
        self.last_bounds.set(frame.bounds());
        self.poster_drawn.set(false);
        let root = Flex::vertical()
            .constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(if self.prompt.is_some() { 3 } else { 0 }), Constraint::Fixed(2)])
            .split(frame.bounds());
        self.view_tabs(root[0], frame);
        match self.screen {
            Screen::Torrents => self.view_torrents(root[1], frame),
            Screen::Search => self.view_search(root[1], frame),
            Screen::Rss => self.view_rss(root[1], frame),
            Screen::Shows => self.view_shows(root[1], frame),
        }
        if let Some(p) = &self.prompt {
            let title = p.title();
            let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title).padding(Sides::horizontal(1));
            let inner = block.inner(root[2]);
            block.render(root[2], frame);
            if p.yes_no {
                Paragraph::new("y / N (default: No)").style(th.accented()).render(inner, frame);
            } else {
                p.input.render(inner, frame);
            }
        }
        self.view_status(root[3], frame);
        if self.files.is_some() { self.view_files(frame); }
        if self.providers.is_some() { self.view_providers(frame); }
        if self.settings.is_some() { self.view_settings(frame); }
        if self.show_help { self.view_help(frame); }
        // Graphics-layer posters sit above text, so anything drawn over the card
        // (modals, other screens) needs the placement gone.
        if let Some(k) = &self.kitty {
            let overlay = self.files.is_some() || self.providers.is_some() || self.settings.is_some() || self.show_help || self.prompt.is_some();
            if !self.poster_drawn.get() || overlay { k.borrow_mut().clear(); }
        }
    }
}
