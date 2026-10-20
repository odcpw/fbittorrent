//! Existing screens search behavior.
use super::*;

impl TuiApp {
    pub(super) fn selected_result(&self) -> Option<&SearchResult> {
        self.search_list.borrow().selected().and_then(|i| self.search_results.get(i))
    }

    pub(super) fn add_search_result(&mut self, result: SearchResult) -> Cmd<Msg> {
        let Some(row) = self.wire_result(&result).cloned() else { return Cmd::none() };
        if !self.search_selectable(&result) { return Cmd::none(); }
        self.search_adding.insert(search_identity(&result));
        self.native_search_grab(vec![row])
    }

    /// Marks and add states use the displayed identity, which is also how a
    /// display row finds its daemon result.
    pub(super) fn search_mark_state(&self, result: &SearchResult) -> (bool, bool, bool) {
        let id = search_identity(result);
        let added = self.search_added.contains(&id) || self.wire_result(result).is_some_and(|row| row.added.is_some());
        (self.search_marked.contains(&id), !added && self.search_adding.contains(&id), added)
    }
    fn search_selectable(&self, result: &SearchResult) -> bool {
        let (_, adding, added) = self.search_mark_state(result);
        !adding && !added
    }
    fn mark_search_row(&mut self, index: usize, on: bool) {
        let Some(result) = self.search_results.get(index) else { return };
        let id = search_identity(result);
        if on && self.search_selectable(result) && self.search_marked.len() < BATCH_LIMIT { self.search_marked.insert(id); }
        else if !on { self.search_marked.remove(&id); }
    }
    fn report_marks(&mut self) {
        let n = self.search_marked.len();
        if n == 0 { self.set_status("selection cleared", false); }
        else { self.set_status(format!("{n} selected · Enter downloads · Esc clears"), false); }
    }
    /// Space: toggle the current result and move on, so repeated Space marks a run.
    pub(super) fn toggle_search_mark(&mut self) -> Cmd<Msg> {
        let Some(index) = self.search_list.borrow().selected() else { return Cmd::none() };
        let on = !self.search_results.get(index).is_some_and(|r| self.search_marked.contains(&search_identity(r)));
        self.mark_search_row(index, on);
        self.search_list.borrow_mut().select_next(self.search_results.len());
        self.report_marks();
        Cmd::none()
    }
    /// Shift+↓/↑: mark the current result and the one moved to.
    pub(super) fn extend_search_marks(&mut self, down: bool) -> Cmd<Msg> {
        let Some(index) = self.search_list.borrow().selected() else { return Cmd::none() };
        self.mark_search_row(index, true);
        let next = if down { (index + 1).min(self.search_results.len().saturating_sub(1)) } else { index.saturating_sub(1) };
        self.search_list.borrow_mut().select(Some(next));
        self.mark_search_row(next, true);
        self.report_marks();
        Cmd::none()
    }
    pub(super) fn clear_search_marks(&mut self) -> bool {
        if self.search_marked.is_empty() { return false; }
        self.search_marked.clear();
        self.report_marks();
        true
    }
    fn marked_search_rows(&self) -> Vec<(String, crate::api::SearchItem)> {
        self.search_results.iter()
            .filter(|r| self.search_marked.contains(&search_identity(r)) && self.search_selectable(r))
            .filter_map(|r| self.wire_result(r).map(|row| (search_identity(r), row.clone())))
            .collect()
    }
    fn confirm_search_batch(&mut self) -> Cmd<Msg> {
        let rows = self.marked_search_rows();
        if rows.is_empty() { self.search_marked.clear(); return Cmd::none(); }
        let bytes: u64 = rows.iter().filter_map(|(_, row)| row.size).sum();
        let folder = self.add_opts().save_path.unwrap_or_else(|| "the default download folder".into());
        let label = format!("Download {} results? All files · {} · no upload · to {folder}", rows.len(), human_bytes(bytes as i64));
        self.open_prompt(PromptKind::ConfirmSearchBatch { count: rows.len() }, &label, true);
        Cmd::none()
    }
    /// One confirmed choice adds every marked result: all files, the default
    /// folder and no upload. A single result still adds on Enter alone.
    pub(super) fn grab_marked_results(&mut self) -> Cmd<Msg> {
        let rows = self.marked_search_rows();
        self.search_marked.clear();
        if rows.is_empty() { return Cmd::none(); }
        self.set_status(format!("adding {} results…", rows.len()), false);
        let mut wire = Vec::with_capacity(rows.len());
        for (id, row) in rows { self.search_adding.insert(id); wire.push(row); }
        self.native_search_grab(wire)
    }

    pub(super) fn load_browse(&mut self, entry: browse::Entry) -> Cmd<Msg> {
        self.native_browse(entry)
    }

    pub(super) fn start_search(&mut self) -> Cmd<Msg> {
        let q = self.search_input.value().trim().to_string();
        if q.is_empty() { return Cmd::none(); }
        self.search_seq += 1;
        self.search_results.clear();
        self.search_wire.clear();
        self.search_marked.clear();
        self.search_list.borrow_mut().select(None);
        self.search_running = true;
        self.search_sub = Some((1000 + self.search_seq, q.clone()));
        self.set_status(format!("searching \"{q}\"…"), false);
        self.native_search_start(q)
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
            actions::Intent::Cancel => {
                if self.clear_search_marks() { return Cmd::none(); }
                return self.native_search_cancel();
            }
            actions::Intent::Down => { if n > 0 { self.search_list.borrow_mut().select_next(n); } }
            actions::Intent::Up => {
                if self.search_list.borrow().selected() == Some(0) { self.search_focus_input = true; self.search_input.set_focused(true); }
                else { self.search_list.borrow_mut().select_previous(); }
            }
            actions::Intent::First => self.search_list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            actions::Intent::Last => self.search_list.borrow_mut().select(n.checked_sub(1)),
            actions::Intent::Primary => {
                if !self.search_marked.is_empty() { return self.confirm_search_batch(); }
                if let Some(r) = self.selected_result().cloned() { return self.add_search_result(r); }
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
                // Provider-supplied: only http(s) pages reach xdg-open.
                if !r.descr_link.is_empty() && !crate::public_text::open_web(&r.descr_link) { self.set_status("source link is not a web page", true); }
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


pub(super) const MAX_RESULTS: usize = 100;
/// The API allows eight requests in flight; a batch stays well within the queue.
pub(super) const BATCH_LIMIT: usize = 50;

pub(super) fn search_identity(result: &SearchResult) -> String {
    format!("{}\u{0}{}", result.file_name, result.file_url)
}
