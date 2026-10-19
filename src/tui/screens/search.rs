//! Existing screens search behavior.
use super::*;

pub(super) struct SearchSub { pub(super) id: SubId, pub(super) query: String, pub(super) cfg: Config, pub(super) presence:Option<Arc<attach::Presence>> }

impl TuiApp {
    pub(super) fn selected_result(&self) -> Option<&SearchResult> {
        self.search_list.borrow().selected().and_then(|i| self.search_results.get(i))
    }

    pub(super) fn add_search_result(&self, result: SearchResult) -> Cmd<Msg> {
        if self.cfg.native() {return self.native_search_grab(&result);}
        let opts = self.add_opts();
        let label = short(&result.file_name, 40);
        self.action(format!("added {label}"), move |c| c.add_search_result(&result, &opts))
    }

    pub(super) fn load_browse(&mut self, entry: browse::Entry) -> Cmd<Msg> {
        if self.cfg.native() {return self.native_browse(entry);}
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
        self.search_wire.clear();
        self.search_list.borrow_mut().select(None);
        self.search_running = true;
        self.search_sub = Some((1000 + self.search_seq, q.clone()));
        self.set_status(format!("searching \"{q}\"…"), false);
        if self.cfg.native() {return self.native_search_start(q);}
        Cmd::none()
    }

    #[cfg(test)]
    pub(super) fn on_search_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Search), key)
    }

    pub(super) fn on_search_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        if intent==actions::Intent::Card {self.search_card=!self.search_card;return Cmd::none();}
        if intent==actions::Intent::Cancel && self.search_card {self.search_card=false;return Cmd::none();}
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
            actions::Intent::Cancel => { if self.cfg.native(){return self.native_search_cancel();} self.search_sub = None; self.search_running = false; }
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


}


impl Subscription<Msg> for SearchSub {
    fn id(&self) -> SubId { self.id }
    fn run(&self, sender: Sender<Msg>, stop: StopSignal) {
        let id = self.id;
        let fail = |e: String| Msg::SearchBatch { id, results: vec![], done: true, error: Some(e) };
        let c = Client::from_config(&self.cfg,self.presence.clone());
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

pub(super) const MAX_RESULTS: usize = 100;
