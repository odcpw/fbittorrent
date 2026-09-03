//! The fbittorrent terminal UI (FrankenTUI).

mod poster;
mod theme;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ftui::layout::{Constraint, Flex, Rect, Sides};
use ftui::runtime::{Every, StopSignal, SubId, Subscription};
use ftui::text::wrap::WrapMode;
use ftui::text::{Line, Span, Text};
use ftui::widgets::block::Block;
use ftui::widgets::borders::BorderType;
use ftui::widgets::input::TextInput;
use ftui::widgets::list::{List, ListItem, ListState};
use ftui::widgets::paragraph::Paragraph;
use ftui::widgets::progress::MiniBar;
use ftui::widgets::{StatefulWidget, StatusItem, StatusLine, Tab, Tabs, TabsState, Widget};
use ftui::{App, Cmd, Event, Frame, KeyCode, KeyEvent, KeyEventKind, Model, Modifiers, Style};

use crate::config::Config;
use crate::imdb::{self, Title};
use crate::browse;
use crate::qbt::{human_bytes, human_eta, human_rate, ymd, AddOptions, Client, RssFeed, SearchPlugin, SearchResult, Torrent, TorrentFile, TransferInfo};
use poster::{KittyPlacer, Poster};
use theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen { Torrents, Search, Rss }

impl Screen {
    fn index(self) -> usize { match self { Screen::Torrents => 0, Screen::Search => 1, Screen::Rss => 2 } }
    fn from_index(i: usize) -> Self { match i % 3 { 0 => Screen::Torrents, 1 => Screen::Search, _ => Screen::Rss } }
}

pub enum ImdbState {
    Loading,
    Missing,
    Ready(Box<Title>, Option<Poster>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
enum PromptKind {
    AddMagnet,
    AddFeed,
    ConfirmDeleteTorrent { hash: String, files: bool },
    ConfirmRemoveFeed { name: String },
}

struct Prompt {
    kind: PromptKind,
    label: String,
    input: TextInput,
    yes_no: bool,
    /// Per-add overrides for the magnet prompt: (sequential, first/last, allow upload).
    add_flags: (bool, bool, bool),
}

impl Prompt {
    /// Title with the live flag state, so the toggles are discoverable while typing.
    fn title(&self) -> String {
        if self.kind != PromptKind::AddMagnet { return self.label.clone(); }
        let (s, f, u) = self.add_flags;
        format!("{}  ·  F2 sequential:{}  F3 first/last:{}  F4 upload:{} ", self.label,
            if s { "on" } else { "off" }, if f { "on" } else { "off" }, if u { "on" } else { "off" })
    }
}

struct ProvidersModal {
    plugins: Vec<SearchPlugin>,
    list: RefCell<ListState>,
    loading: bool,
    error: String,
}

struct FilesModal {
    hash: String,
    name: String,
    files: Vec<TorrentFile>,
    list: RefCell<ListState>,
    loading: bool,
    error: String,
}

pub enum Msg {
    Raw(Event),
    Poll,
    Torrents(Result<(Vec<Torrent>, TransferInfo), String>),
    ActionDone(Result<String, String>),
    Imdb(String, Result<Option<(Title, Option<Poster>)>, String>),
    SearchBatch { id: SubId, results: Vec<SearchResult>, done: bool, error: Option<String> },
    Rss(Result<Vec<RssFeed>, String>),
    Browse(String, Result<Vec<SearchResult>, String>),
    /// Fast timer: look the highlighted title up once the cursor has rested on it.
    Hover,
    Files(String, Result<Vec<TorrentFile>, String>),
    Plugins(Result<Vec<SearchPlugin>, String>),
    Ratings(Result<Arc<imdb::RatingsIndex>, String>),
    /// Resolved (release name, IMDb id) pairs for the ratings column.
    Ids(Vec<(String, String)>),
}

impl From<Event> for Msg {
    fn from(e: Event) -> Self { Msg::Raw(e) }
}

pub struct TuiApp {
    cfg: Config,
    theme: Theme,
    screen: Screen,
    tabs: RefCell<TabsState>,
    show_help: bool,
    online: bool,
    backend_error: String,
    torrents: Vec<Torrent>,
    transfer: TransferInfo,
    tor_list: RefCell<ListState>,
    poll_inflight: bool,
    tick: usize,
    // search
    search_input: TextInput,
    search_focus_input: bool,
    search_results: Vec<SearchResult>,
    search_list: RefCell<ListState>,
    search_sub: Option<(SubId, String)>,
    search_running: bool,
    search_seq: u64,
    search_browse: bool,
    browse_entries: Vec<browse::Entry>,
    browse_list: RefCell<ListState>,
    browse_loading: Option<String>,
    files: Option<FilesModal>,
    providers: Option<ProvidersModal>,
    plugins: Vec<SearchPlugin>,
    ratings: Option<Arc<imdb::RatingsIndex>>,
    name_ids: HashMap<String, String>,
    ids_inflight: bool,
    status_at: std::time::Instant,
    theme_mtime: Option<std::time::SystemTime>,
    hover_key: Option<String>,
    hover_since: std::time::Instant,
    kitty: Option<RefCell<KittyPlacer>>,
    /// Set by view() when a poster was drawn this frame; used to clear stale placements.
    poster_drawn: std::cell::Cell<bool>,
    // rss
    feeds: Vec<RssFeed>,
    feed_list: RefCell<ListState>,
    item_list: RefCell<ListState>,
    rss_focus_items: bool,
    rss_loaded: bool,
    // imdb
    imdb: HashMap<String, ImdbState>,
    prompt: Option<Prompt>,
    status: String,
    status_is_error: bool,
}

const SPINNER: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];

pub fn run(cfg: Config, screen: Screen) -> Result<()> {
    let theme = Theme::load();
    let mut search_input = TextInput::new()
        .with_placeholder("search torrents, Enter to run")
        .with_style(theme.text())
        .with_cursor_style(Style::new().bg(theme.accent).fg(theme.selection));
    search_input.set_focused(screen == Screen::Search);
    let app = TuiApp {
        cfg,
        theme,
        screen,
        tabs: RefCell::new(TabsState { active: screen.index(), offset: 0 }),
        show_help: false,
        online: true,
        backend_error: String::new(),
        torrents: vec![],
        transfer: TransferInfo::default(),
        tor_list: RefCell::new(ListState::default()),
        poll_inflight: false,
        tick: 0,
        search_input,
        search_focus_input: screen == Screen::Search,
        search_results: vec![],
        search_list: RefCell::new(ListState::default()),
        search_sub: None,
        search_running: false,
        search_seq: 0,
        search_browse: false,
        browse_entries: browse::catalogue(),
        browse_list: RefCell::new(ListState::default()),
        browse_loading: None,
        files: None,
        providers: None,
        plugins: vec![],
        ratings: None,
        name_ids: HashMap::new(),
        ids_inflight: false,
        status_at: std::time::Instant::now(),
        theme_mtime: std::fs::metadata(theme::colors_path()).and_then(|m| m.modified()).ok(),
        hover_key: None,
        hover_since: std::time::Instant::now(),
        kitty: if poster::kitty_supported() { Some(RefCell::new(KittyPlacer::default())) } else { None },
        poster_drawn: std::cell::Cell::new(false),
        feeds: vec![],
        feed_list: RefCell::new(ListState::default()),
        item_list: RefCell::new(ListState::default()),
        rss_focus_items: false,
        rss_loaded: false,
        imdb: HashMap::new(),
        prompt: None,
        status: String::new(),
        status_is_error: false,
    };
    App::fullscreen(app).with_mouse().run()?;
    Ok(())
}

// ---------------------------------------------------------------- background work

struct SearchSub { id: SubId, query: String, cfg: Config }

impl Subscription<Msg> for SearchSub {
    fn id(&self) -> SubId { self.id }
    fn run(&self, sender: Sender<Msg>, stop: StopSignal) {
        let id = self.id;
        let fail = |e: String| Msg::SearchBatch { id, results: vec![], done: true, error: Some(e) };
        let c = Client::from_config(&self.cfg);
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
                    if sender.send(Msg::SearchBatch { id, results: r.results, done, error: None }).is_err() || done {
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

impl TuiApp {
    fn poll(&mut self) -> Cmd<Msg> {
        if self.poll_inflight { return Cmd::none(); }
        self.poll_inflight = true;
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login().and_then(|_| Ok((c.torrents()?, c.transfer_info()?)));
            Msg::Torrents(r.map_err(|e| e.to_string()))
        })
    }

    fn action<F>(&self, label: impl Into<String>, f: F) -> Cmd<Msg>
    where F: FnOnce(&Client) -> anyhow::Result<()> + Send + 'static {
        let cfg = self.cfg.clone();
        let label = label.into();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            match c.login().and_then(|_| f(&c)) {
                Ok(()) => Msg::ActionDone(Ok(label)),
                Err(e) => Msg::ActionDone(Err(e.to_string())),
            }
        })
    }

    fn add_opts(&self) -> AddOptions {
        AddOptions {
            sequential: self.cfg.sequential,
            first_last: self.cfg.first_last,
            seed: self.cfg.seed_by_default,
            stopped: false,
            save_path: self.cfg.save_path.clone(),
            category: self.cfg.category.clone(),
        }
    }

    fn add_url(&self, url: String, label: String) -> Cmd<Msg> {
        let opts = self.add_opts();
        self.action(format!("added {label}"), move |c| c.add_url(&url, &opts))
    }

    fn load_rss(&self) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            Msg::Rss(c.login().and_then(|_| c.rss_feeds()).map_err(|e| e.to_string()))
        })
    }

    fn lookup_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
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

    fn open_files(&mut self, t: &Torrent) -> Cmd<Msg> {
        self.files = Some(FilesModal { hash: t.hash.clone(), name: t.name.clone(), files: vec![], list: RefCell::new(ListState::default()), loading: true, error: String::new() });
        self.reload_files(t.hash.clone())
    }
    fn reload_files(&self, hash: String) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login().and_then(|_| c.files(&hash)).map_err(|e| e.to_string());
            Msg::Files(hash, r)
        })
    }
    fn set_file_prio(&self, hash: String, ids: Vec<i64>, prio: i64) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        let h = hash.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login().and_then(|_| c.set_file_priority(&h, &ids, prio)).and_then(|_| c.files(&h)).map_err(|e| e.to_string());
            Msg::Files(hash, r)
        })
    }
    fn load_plugins(&self) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            Msg::Plugins(c.login().and_then(|_| c.search_plugins()).map_err(|e| e.to_string()))
        })
    }
    fn set_plugins(&self, names: Vec<String>, enable: bool) -> Cmd<Msg> {
        let cfg = self.cfg.clone();
        Cmd::task(move || {
            let c = Client::from_config(&cfg);
            let r = c.login()
                .and_then(|_| { for n in &names { c.search_set_plugin(n, enable)?; } Ok(()) })
                .and_then(|_| c.search_plugins())
                .map_err(|e| e.to_string());
            Msg::Plugins(r)
        })
    }
    fn open_providers(&mut self) -> Cmd<Msg> {
        self.providers = Some(ProvidersModal { plugins: self.plugins.clone(), list: RefCell::new(ListState::default()), loading: self.plugins.is_empty(), error: String::new() });
        if let Some(m) = &self.providers { if !m.plugins.is_empty() { m.list.borrow_mut().select(Some(0)); } }
        self.load_plugins()
    }
    fn on_providers_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let Some(m) = self.providers.as_ref() else { return Cmd::none() };
        let n = m.plugins.len();
        let cur = m.list.borrow().selected().and_then(|i| m.plugins.get(i)).cloned();
        let all: Vec<String> = m.plugins.iter().map(|p| p.name.clone()).collect();
        match k.code {
            KeyCode::Escape | KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char('s') => { self.providers = None; }
            KeyCode::Down | KeyCode::Char('j') => { if n > 0 { m.list.borrow_mut().select_next(n); } }
            KeyCode::Up | KeyCode::Char('k') => m.list.borrow_mut().select_previous(),
            KeyCode::Char(' ') => if let Some(p) = cur { return self.set_plugins(vec![p.name], !p.enabled); },
            KeyCode::Char('a') => return self.set_plugins(all, true),
            KeyCode::Char('n') => return self.set_plugins(all, false),
            KeyCode::Char('o') => if let Some(p) = cur { if !p.url.is_empty() { let _ = std::process::Command::new("xdg-open").arg(&p.url).spawn(); } },
            KeyCode::Char('r') => return self.load_plugins(),
            _ => {}
        }
        Cmd::none()
    }

    fn load_browse(&mut self, entry: browse::Entry) -> Cmd<Msg> {
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

    /// Title under the cursor on the current screen: (lookup key, IMDb id if known).
    fn hovered_title(&self) -> Option<(String, Option<String>)> {
        if self.files.is_some() || self.prompt.is_some() || self.show_help { return None; }
        match self.screen {
            Screen::Torrents => self.selected_torrent().map(|t| (t.name.clone(), None)),
            Screen::Search if !self.search_browse && !self.search_focus_input =>
                self.selected_result().map(|r| (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()))),
            Screen::Rss if self.rss_focus_items => {
                let i = self.item_list.borrow().selected()?;
                self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| (a.title.clone(), None))
            }
            _ => None,
        }
    }

    /// Debounced auto-lookup: fire once the cursor has rested ~350 ms on a title.
    fn on_hover(&mut self) -> Cmd<Msg> {
        let Some((key, id)) = self.hovered_title() else { self.hover_key = None; return Cmd::none() };
        if self.hover_key.as_deref() != Some(&key) {
            self.hover_key = Some(key);
            self.hover_since = std::time::Instant::now();
            return Cmd::none();
        }
        if self.hover_since.elapsed() < Duration::from_millis(350) || self.imdb.contains_key(&key) { return Cmd::none(); }
        self.lookup_imdb(key, id)
    }

    fn set_status(&mut self, s: impl Into<String>, err: bool) {
        self.status = s.into();
        self.status_is_error = err;
        self.status_at = std::time::Instant::now();
    }

    fn load_ratings(&self) -> Cmd<Msg> {
        Cmd::task(|| {
            if imdb::ratings_stale() { let _ = imdb::refresh_ratings(); }
            Msg::Ratings(imdb::RatingsIndex::load().map(Arc::new).map_err(|e| e.to_string()))
        })
    }

    /// Rating for a result: from the in-memory index when the source gave an IMDb id,
    /// otherwise from a finished name lookup.
    fn rating_for(&self, r: &SearchResult) -> Option<f32> {
        let id = if !r.imdb.is_empty() { Some(r.imdb.as_str()) } else { self.name_ids.get(&r.file_name).map(|s| s.as_str()) };
        if let (Some(id), Some(idx)) = (id, &self.ratings) {
            if let Some((v, _)) = idx.get(id) { return Some(v); }
        }
        match self.imdb.get(&r.file_name) { Some(ImdbState::Ready(t, _)) => t.rating, _ => None }
    }

    /// Resolve IMDb ids for the best results that came without one (top of the list only:
    /// one network call each, cached on disk by release name).
    fn resolve_ids(&mut self) -> Cmd<Msg> {
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

    /// Re-read Omarchy colors when the theme changed under us.
    fn maybe_reload_theme(&mut self) {
        let m = std::fs::metadata(theme::colors_path()).and_then(|m| m.modified()).ok();
        if m != self.theme_mtime {
            self.theme_mtime = m;
            self.theme = Theme::load();
            let value = self.search_input.value().to_string();
            let focused = self.search_input.focused();
            let mut input = TextInput::new()
                .with_placeholder("search torrents, Enter to run")
                .with_style(self.theme.text())
                .with_cursor_style(Style::new().bg(self.theme.accent).fg(self.theme.selection))
                .with_value(value);
            input.set_focused(focused);
            self.search_input = input;
        }
    }

    /// Wheel and tab-strip clicks; everything else stays keyboard-driven.
    fn on_mouse(&mut self, m: ftui::MouseEvent) -> Cmd<Msg> {
        use ftui::{MouseButton, MouseEventKind};
        match m.kind {
            MouseEventKind::ScrollDown => self.on_key(KeyEvent::new(KeyCode::Down)),
            MouseEventKind::ScrollUp => self.on_key(KeyEvent::new(KeyCode::Up)),
            MouseEventKind::Down(MouseButton::Left) if m.y == 0 && self.files.is_none() && self.providers.is_none() => {
                let s = if m.x < 12 { Screen::Torrents } else if m.x < 22 { Screen::Search } else if m.x < 29 { Screen::Rss } else { return Cmd::none() };
                self.set_screen(s)
            }
            _ => Cmd::none(),
        }
    }

    /// Open the IMDb page (or start the lookup) for a release name.
    fn open_imdb(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
        if let Some(ImdbState::Ready(t, _)) = self.imdb.get(&name) {
            let _ = std::process::Command::new("xdg-open").arg(t.url()).spawn();
            self.set_status("opened IMDb in browser", false);
            Cmd::none()
        } else {
            self.lookup_imdb(name, id)
        }
    }

    /// Open the Wikipedia article behind the IMDb card, if there is one.
    fn open_wiki(&mut self, name: String, id: Option<String>) -> Cmd<Msg> {
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

    fn selected_torrent(&self) -> Option<&Torrent> {
        self.tor_list.borrow().selected().and_then(|i| self.torrents.get(i))
    }
    fn selected_result(&self) -> Option<&SearchResult> {
        self.search_list.borrow().selected().and_then(|i| self.search_results.get(i))
    }
    fn selected_feed(&self) -> Option<&RssFeed> {
        self.feed_list.borrow().selected().and_then(|i| self.feeds.get(i))
    }

    fn set_screen(&mut self, s: Screen) -> Cmd<Msg> {
        self.screen = s;
        self.tabs.borrow_mut().select(s.index(), 3);
        // Landing on Search with nothing to browse means the user wants to type.
        if s == Screen::Search && self.search_results.is_empty() { self.search_focus_input = true; }
        self.search_input.set_focused(s == Screen::Search && self.search_focus_input);
        if s == Screen::Rss && !self.rss_loaded { self.rss_loaded = true; return self.load_rss(); }
        Cmd::none()
    }

    fn open_prompt(&mut self, kind: PromptKind, label: &str, yes_no: bool) {
        let mut input = TextInput::new()
            .with_style(self.theme.text())
            .with_cursor_style(Style::new().bg(self.theme.accent).fg(self.theme.selection));
        input.set_focused(!yes_no);
        self.search_input.set_focused(false);
        let add_flags = (self.cfg.sequential, self.cfg.first_last, self.cfg.seed_by_default);
        self.prompt = Some(Prompt { kind, label: label.to_string(), input, yes_no, add_flags });
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        self.search_input.set_focused(self.screen == Screen::Search && self.search_focus_input);
    }

    // ------------------------------------------------------------ key handling

    fn on_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        if k.kind == KeyEventKind::Release { return Cmd::none(); }
        if self.show_help {
            self.show_help = false;
            return Cmd::none();
        }
        if self.prompt.is_some() { return self.on_prompt_key(k); }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(Modifiers::CTRL) { return Cmd::quit(); }
        if self.files.is_some() { return self.on_files_key(k); }
        if self.providers.is_some() { return self.on_providers_key(k); }

        // The search box eats printable keys while focused.
        if self.screen == Screen::Search && self.search_focus_input {
            match k.code {
                KeyCode::Escape => { self.search_focus_input = false; self.search_input.set_focused(false); return Cmd::none(); }
                KeyCode::Enter => {
                    if self.search_input.value().trim().is_empty() {
                        self.search_focus_input = false; self.search_input.set_focused(false);
                        self.search_browse = true;
                        if self.browse_list.borrow().selected().is_none() { self.browse_list.borrow_mut().select(Some(0)); }
                        return Cmd::none();
                    }
                    return self.start_search();
                }
                KeyCode::Tab | KeyCode::BackTab => {}
                KeyCode::Down if !self.search_results.is_empty() => {
                    self.search_focus_input = false; self.search_input.set_focused(false);
                    if self.search_list.borrow().selected().is_none() { self.search_list.borrow_mut().select(Some(0)); }
                    return Cmd::none();
                }
                _ => { self.search_input.handle_event(&Event::Key(k)); return Cmd::none(); }
            }
        }

        match k.code {
            KeyCode::Char('q') => return Cmd::quit(),
            KeyCode::Char('Q') => {
                let _ = crate::backend::stop(&self.cfg);
                return Cmd::quit();
            }
            KeyCode::Char('?') => { self.show_help = true; return Cmd::none(); }
            KeyCode::Char('1') => return self.set_screen(Screen::Torrents),
            KeyCode::Char('2') => return self.set_screen(Screen::Search),
            KeyCode::Char('3') => return self.set_screen(Screen::Rss),
            KeyCode::Tab => return self.set_screen(Screen::from_index(self.screen.index() + 1)),
            KeyCode::BackTab => return self.set_screen(Screen::from_index(self.screen.index() + 2)),
            KeyCode::Char('a') => { self.open_prompt(PromptKind::AddMagnet, "Add magnet / URL", false); return Cmd::none(); }
            KeyCode::Char('c') => {
                let opts = self.add_opts();
                return self.action("added from clipboard", move |c| {
                    let out = std::process::Command::new("wl-paste").arg("--no-newline").output()?;
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !(text.starts_with("magnet:") || text.starts_with("http")) { anyhow::bail!("clipboard holds no magnet link or URL"); }
                    c.add_url(&text, &opts)
                });
            }
            KeyCode::Char('r') if self.screen != Screen::Rss => {
                if !self.online {
                    // Bring the backend back if it went away under us.
                    let cfg = self.cfg.clone();
                    let up = Cmd::task(move || {
                        let c = Client::from_config(&cfg);
                        match crate::backend::ensure_running(&cfg, &c) {
                            Ok(_) => Msg::ActionDone(Ok("qBittorrent started".into())),
                            Err(e) => Msg::ActionDone(Err(e.to_string())),
                        }
                    });
                    return up;
                }
                return self.poll();
            }
            _ => {}
        }
        match self.screen {
            Screen::Torrents => self.on_torrents_key(k),
            Screen::Search => self.on_search_key(k),
            Screen::Rss => self.on_rss_key(k),
        }
    }

    fn on_prompt_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let Some(p) = self.prompt.as_mut() else { return Cmd::none() };
        if p.yes_no {
            let kind = p.kind.clone();
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.close_prompt();
                    return match kind {
                        PromptKind::ConfirmDeleteTorrent { hash, files } =>
                            self.action(if files { "deleted with files" } else { "removed" }, move |c| c.delete(&[hash], files)),
                        PromptKind::ConfirmRemoveFeed { name } => {
                            let cmd = self.action(format!("removed feed {name}"), move |c| c.rss_remove(&name));
                            Cmd::batch(vec![cmd, self.load_rss()])
                        }
                        _ => Cmd::none(),
                    };
                }
                _ => { self.close_prompt(); return Cmd::none(); }
            }
        }
        match k.code {
            KeyCode::Escape => { self.close_prompt(); Cmd::none() }
            KeyCode::F(2) if p.kind == PromptKind::AddMagnet => { p.add_flags.0 = !p.add_flags.0; Cmd::none() }
            KeyCode::F(3) if p.kind == PromptKind::AddMagnet => { p.add_flags.1 = !p.add_flags.1; Cmd::none() }
            KeyCode::F(4) if p.kind == PromptKind::AddMagnet => { p.add_flags.2 = !p.add_flags.2; Cmd::none() }
            KeyCode::Enter => {
                let value = p.input.value().trim().to_string();
                let kind = p.kind.clone();
                let (seq, fl, seed) = p.add_flags;
                self.close_prompt();
                if value.is_empty() { return Cmd::none(); }
                match kind {
                    PromptKind::AddMagnet => {
                        let label = crate::magnet_name(&value).unwrap_or_else(|| value.chars().take(40).collect());
                        let mut opts = self.add_opts();
                        opts.sequential = seq; opts.first_last = fl; opts.seed = seed;
                        let flags = format!("{}{}{}", if seq { "S" } else { "" }, if fl { "F" } else { "" }, if seed { "U" } else { "" });
                        self.action(format!("added {label} [{flags}]"), move |c| c.add_url(&value, &opts))
                    }
                    PromptKind::AddFeed => {
                        let name = crate::host_of(&value);
                        let cmd = self.action("feed added", move |c| c.rss_add_feed(&value, name.as_deref()));
                        Cmd::batch(vec![cmd, self.load_rss()])
                    }
                    _ => Cmd::none(),
                }
            }
            _ => { p.input.handle_event(&Event::Key(k)); Cmd::none() }
        }
    }

    fn on_torrents_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let n = self.torrents.len();
        match k.code {
            KeyCode::Down | KeyCode::Char('j') => { if n > 0 { self.tor_list.borrow_mut().select_next(n); } }
            KeyCode::Up | KeyCode::Char('k') => self.tor_list.borrow_mut().select_previous(),
            KeyCode::Home | KeyCode::Char('g') => self.tor_list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            KeyCode::End | KeyCode::Char('G') => self.tor_list.borrow_mut().select(n.checked_sub(1)),
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                if let Some(t) = self.selected_torrent().cloned() {
                    let stopped = t.is_stopped();
                    return self.action(if stopped { "started" } else { "stopped" }, move |c| if stopped { c.start(&[t.hash]) } else { c.stop(&[t.hash]) });
                }
            }
            KeyCode::Char('s') => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled sequential", move |c| c.toggle_sequential(&[t.hash]));
            },
            KeyCode::Char('f') => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled first/last", move |c| c.toggle_first_last(&[t.hash]));
            },
            KeyCode::Char('u') => if let Some(t) = self.selected_torrent().cloned() {
                let allow = !t.seed_allowed();
                return self.action(if allow { "seeding allowed" } else { "upload off" },
                    move |c| if allow { c.allow_seeding(&[t.hash]) } else { c.deny_seeding(&[t.hash]) });
            },
            KeyCode::Char('d') | KeyCode::Delete => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: false },
                    &format!("Remove '{}' (keep files)? y/n", short(&t.name, 50)), true);
            },
            KeyCode::Char('D') => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: true },
                    &format!("DELETE '{}' AND ITS FILES? y/n", short(&t.name, 50)), true);
            },
            KeyCode::Char('o') => if let Some(t) = self.selected_torrent() {
                let _ = std::process::Command::new("xdg-open").arg(crate::qbt::open_target(t)).spawn();
            },
            KeyCode::Char('i') => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.open_imdb(name, None);
            },
            KeyCode::Char('w') => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.open_wiki(name, None);
            },
            KeyCode::Enter => if let Some(t) = self.selected_torrent().cloned() {
                return self.open_files(&t);
            },
            _ => {}
        }
        Cmd::none()
    }

    fn on_files_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let Some(m) = self.files.as_ref() else { return Cmd::none() };
        let n = m.files.len();
        let hash = m.hash.clone();
        let sel = m.list.borrow().selected();
        let cur = sel.and_then(|i| m.files.get(i)).cloned();
        let all_ids: Vec<i64> = m.files.iter().map(|f| f.index).collect();
        match k.code {
            KeyCode::Escape | KeyCode::Char('q') | KeyCode::Enter => { self.files = None; }
            KeyCode::Down | KeyCode::Char('j') => { if n > 0 { m.list.borrow_mut().select_next(n); } }
            KeyCode::Up | KeyCode::Char('k') => m.list.borrow_mut().select_previous(),
            KeyCode::Home | KeyCode::Char('g') => m.list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            KeyCode::End | KeyCode::Char('G') => m.list.borrow_mut().select(n.checked_sub(1)),
            KeyCode::Char(' ') => if let Some(f) = cur {
                let prio = if f.priority == 0 { 1 } else { 0 };
                return self.set_file_prio(hash, vec![f.index], prio);
            },
            KeyCode::Char('1') => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 1); },
            KeyCode::Char('2') => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 6); },
            KeyCode::Char('3') => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 7); },
            KeyCode::Char('a') => return self.set_file_prio(hash, all_ids, 1),
            KeyCode::Char('n') => return self.set_file_prio(hash, all_ids, 0),
            KeyCode::Char('r') => return self.reload_files(hash),
            _ => {}
        }
        Cmd::none()
    }

    fn start_search(&mut self) -> Cmd<Msg> {
        let q = self.search_input.value().trim().to_string();
        if q.is_empty() { return Cmd::none(); }
        self.search_seq += 1;
        self.search_results.clear();
        self.search_list.borrow_mut().select(None);
        self.search_running = true;
        self.search_sub = Some((1000 + self.search_seq, q.clone()));
        self.set_status(format!("searching \"{q}\"…"), false);
        Cmd::none()
    }

    fn on_search_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        if self.search_browse {
            let n = self.browse_entries.len();
            match k.code {
                KeyCode::Escape | KeyCode::Char('b') => { self.search_browse = false; }
                KeyCode::Down | KeyCode::Char('j') => self.browse_list.borrow_mut().select_next(n),
                KeyCode::Up | KeyCode::Char('k') => self.browse_list.borrow_mut().select_previous(),
                KeyCode::Char('/') | KeyCode::Char('e') => { self.search_browse = false; self.search_focus_input = true; self.search_input.set_focused(true); }
                KeyCode::Enter => {
                    let sel = self.browse_list.borrow().selected();
                    if let Some(e) = sel.and_then(|i| self.browse_entries.get(i)).cloned() { return self.load_browse(e); }
                }
                _ => {}
            }
            return Cmd::none();
        }
        let n = self.search_results.len();
        match k.code {
            KeyCode::Char('s') => return self.open_providers(),
            KeyCode::Char('b') => {
                self.search_browse = true;
                if self.browse_list.borrow().selected().is_none() { self.browse_list.borrow_mut().select(Some(0)); }
            }
            KeyCode::Char('/') | KeyCode::Char('e') => { self.search_focus_input = true; self.search_input.set_focused(true); }
            KeyCode::Escape => { self.search_sub = None; self.search_running = false; }
            KeyCode::Down | KeyCode::Char('j') => { if n > 0 { self.search_list.borrow_mut().select_next(n); } }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.search_list.borrow().selected() == Some(0) { self.search_focus_input = true; self.search_input.set_focused(true); }
                else { self.search_list.borrow_mut().select_previous(); }
            }
            KeyCode::Home | KeyCode::Char('g') => self.search_list.borrow_mut().select(if n > 0 { Some(0) } else { None }),
            KeyCode::End | KeyCode::Char('G') => self.search_list.borrow_mut().select(n.checked_sub(1)),
            KeyCode::Enter => if let Some(r) = self.selected_result().cloned() {
                return self.add_url(r.file_url.clone(), short(&r.file_name, 40));
            },
            KeyCode::Char('i') => if let Some(r) = self.selected_result() {
                let (name, id) = (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()));
                return self.open_imdb(name, id);
            },
            KeyCode::Char('w') => if let Some(r) = self.selected_result() {
                let (name, id) = (r.file_name.clone(), (!r.imdb.is_empty()).then(|| r.imdb.clone()));
                return self.open_wiki(name, id);
            },
            KeyCode::Char('O') => if let Some(r) = self.selected_result() {
                if !r.descr_link.is_empty() { let _ = std::process::Command::new("xdg-open").arg(&r.descr_link).spawn(); }
            },
            _ => {}
        }
        Cmd::none()
    }

    fn on_rss_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let nf = self.feeds.len();
        let ni = self.selected_feed().map(|f| f.articles.len()).unwrap_or(0);
        match k.code {
            KeyCode::Left | KeyCode::Char('h') => self.rss_focus_items = false,
            KeyCode::Right | KeyCode::Char('l') => { if ni > 0 { self.rss_focus_items = true; if self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } } }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.rss_focus_items { if ni > 0 { self.item_list.borrow_mut().select_next(ni); } }
                else if nf > 0 { self.feed_list.borrow_mut().select_next(nf); self.item_list.borrow_mut().select(None); }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.rss_focus_items { self.item_list.borrow_mut().select_previous(); }
                else { self.feed_list.borrow_mut().select_previous(); self.item_list.borrow_mut().select(None); }
            }
            KeyCode::Char('n') => { self.open_prompt(PromptKind::AddFeed, "Add RSS feed URL", false); }
            KeyCode::Char('x') => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                self.open_prompt(PromptKind::ConfirmRemoveFeed { name: name.clone() }, &format!("Remove feed '{name}'? y/n"), true);
            },
            KeyCode::Char('r') => {
                let names: Vec<String> = self.feeds.iter().map(|f| f.name.clone()).collect();
                let cmd = self.action("refreshing feeds", move |c| { for n in &names { c.rss_refresh(n)?; } Ok(()) });
                self.set_status("refreshing feeds…", false);
                return Cmd::batch(vec![cmd, self.load_rss()]);
            }
            KeyCode::Char('R') => return self.load_rss(),
            KeyCode::Char('m') => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                let cmd = self.action(format!("marked {name} as read"), move |c| c.rss_mark_read(&name, None));
                return Cmd::batch(vec![cmd, self.load_rss()]);
            },
            KeyCode::Enter => if self.rss_focus_items {
                if let (Some(f), Some(i)) = (self.selected_feed(), self.item_list.borrow().selected()) {
                    if let Some(a) = f.articles.get(i) {
                        let url = if a.torrent_url.is_empty() { a.link.clone() } else { a.torrent_url.clone() };
                        let (feed, id, label) = (f.name.clone(), a.id.clone(), short(&a.title, 40));
                        let opts = self.add_opts();
                        let cmd = self.action(format!("added {label}"), move |c| { c.add_url(&url, &opts)?; c.rss_mark_read(&feed, Some(&id)) });
                        return Cmd::batch(vec![cmd, self.load_rss()]);
                    }
                }
            },
            KeyCode::Char('i') => if self.rss_focus_items {
                let idx = self.item_list.borrow().selected();
                let name = idx.and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| a.title.clone()));
                if let Some(name) = name { return self.lookup_imdb(name, None); }
            },
            _ => {}
        }
        Cmd::none()
    }
}

// ---------------------------------------------------------------- model

impl Model for TuiApp {
    type Message = Msg;

    fn init(&mut self) -> Cmd<Msg> {
        let mut cmds = vec![self.poll(), self.load_plugins(), self.load_ratings()];
        if self.screen == Screen::Rss { self.rss_loaded = true; cmds.push(self.load_rss()); }
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
            Msg::Poll => { self.tick += 1; self.maybe_reload_theme(); self.poll() }
            Msg::Hover => {
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
                if let Some(e) = error { self.set_status(format!("search failed: {e}"), true); }
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
                    self.set_status(format!("{n} result{}", if n == 1 { "" } else { "s" }), false);
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
            Msg::Rss(r) => {
                match r {
                    Ok(feeds) => {
                        let sel = self.selected_feed().map(|f| f.name.clone());
                        self.feeds = feeds;
                        let mut st = self.feed_list.borrow_mut();
                        match sel.and_then(|n| self.feeds.iter().position(|f| f.name == n)) {
                            Some(i) => st.select(Some(i)),
                            None => st.select(if self.feeds.is_empty() { None } else { Some(0) }),
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
        self.poster_drawn.set(false);
        let root = Flex::vertical()
            .constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(if self.prompt.is_some() { 3 } else { 0 }), Constraint::Fixed(1)])
            .split(frame.bounds());
        self.view_tabs(root[0], frame);
        match self.screen {
            Screen::Torrents => self.view_torrents(root[1], frame),
            Screen::Search => self.view_search(root[1], frame),
            Screen::Rss => self.view_rss(root[1], frame),
        }
        if let Some(p) = &self.prompt {
            let title = p.title();
            let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title).padding(Sides::horizontal(1));
            let inner = block.inner(root[2]);
            block.render(root[2], frame);
            if p.yes_no {
                Paragraph::new("y / n").style(th.accented()).render(inner, frame);
            } else {
                p.input.render(inner, frame);
            }
        }
        self.view_status(root[3], frame);
        if self.files.is_some() { self.view_files(frame); }
        if self.providers.is_some() { self.view_providers(frame); }
        if self.show_help { self.view_help(frame); }
        // Graphics-layer posters sit above text, so anything drawn over the card
        // (modals, other screens) needs the placement gone.
        if let Some(k) = &self.kitty {
            let overlay = self.files.is_some() || self.providers.is_some() || self.show_help || self.prompt.is_some();
            if !self.poster_drawn.get() || overlay { k.borrow_mut().clear(); }
        }
    }
}

// ---------------------------------------------------------------- views

impl TuiApp {
    fn view_tabs(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Fill, Constraint::FitContent]).split(area);
        let tabs = Tabs::new([Tab::new(" 1 Torrents "), Tab::new(" 2 Search "), Tab::new(" 3 RSS ")])
            .style(th.dimmed())
            .active_style(th.selected())
            .separator("");
        StatefulWidget::render(&tabs, cols[0], frame, &mut self.tabs.borrow_mut());

        let right = if self.online {
            let mut spans = vec![
                Span::styled("↓ ", th.dimmed()), Span::styled(human_rate(self.transfer.dl_info_speed), th.text()),
                Span::styled("  ↑ ", th.dimmed()), Span::styled(human_rate(self.transfer.up_info_speed), th.text()),
                Span::styled(format!("  {} ", th.name), th.dimmed()),
            ];
            if self.poll_inflight { spans.insert(0, Span::styled(format!("{} ", SPINNER[self.tick % 4]), th.dimmed())); }
            Line::from_spans(spans)
        } else {
            Line::from_spans([Span::styled("● offline ", th.err())])
        };
        let w = right.width() as u16;
        let r = Rect::new(area.x + area.width.saturating_sub(w + 1), area.y, w.min(area.width), 1);
        Paragraph::new(Text::from_line(right)).render(r, frame);
    }

    fn view_status(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let mut sl = StatusLine::new().style(th.dimmed());
        let hints: &[(&str, &str)] = if self.prompt.is_some() {
            &[("Enter", "confirm"), ("Esc", "cancel")]
        } else if self.files.is_some() {
            &[("Space", "toggle"), ("1/2/3", "normal/high/max"), ("a/n", "all/none"), ("Esc", "close")]
        } else if self.providers.is_some() {
            &[("Space", "toggle"), ("a/n", "all/none"), ("o", "open site"), ("Esc", "close")]
        } else { match self.screen {
            Screen::Torrents => &[("j/k", "move"), ("Enter", "files"), ("p", "pause"), ("u", "seed on/off"), ("d/D", "remove/+files"), ("s/f", "seq/first-last"), ("o", "open"), ("a/c", "add/clip"), ("?", "help")],
            Screen::Search if self.search_browse => &[("j/k", "move"), ("Enter", "load / open site"), ("Esc", "back"), ("/", "search instead")],
            Screen::Search if self.search_focus_input => &[("Enter", "search · empty = browse providers"), ("↓", "results"), ("Esc", "leave box"), ("?", "help")],
            Screen::Search => &[("Enter", "add"), ("s", "choose providers"), ("b", "browse lists"), ("O", "source page"), ("/", "edit query"), ("?", "help")],
            Screen::Rss => &[("h/l", "feeds/items"), ("Enter", "add item"), ("n", "new feed"), ("x", "remove feed"), ("r", "refresh"), ("m", "mark read"), ("?", "help")],
        }};
        for (k, a) in hints { sl = sl.left(StatusItem::key_hint(k, a)); }
        if !self.status.is_empty() {
            sl = sl.right(StatusItem::text(&self.status));
        }
        sl.render(area, frame);
        if !self.status.is_empty() {
            // Recolor the trailing status so errors stand out.
            let style = if self.status_is_error { th.err() } else { th.ok() };
            let w = (self.status.chars().count() as u16).min(area.width);
            let r = Rect::new(area.x + area.width - w, area.y, w, 1);
            Paragraph::new(self.status.clone()).style(style).render(r, frame);
        }
    }

    fn view_torrents(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Percentage(58.0), Constraint::Fill]).split(area);
        let title = format!(" Torrents ({}) ", self.torrents.len());
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);

        if !self.online {
            let msg = format!("qBittorrent is not reachable.\n{}\n\npress r to retry", self.backend_error);
            Paragraph::new(msg).style(th.err()).wrap(WrapMode::Word).render(pad(inner), frame);
        } else if self.torrents.is_empty() {
            Paragraph::new("No torrents.\n\na  add a magnet link or URL\nc  add from clipboard\n2  search")
                .style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(torrent_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let items: Vec<ListItem> = self.torrents.iter().map(|t| li(torrent_line(t, inner.width.saturating_sub(2), th))).collect();
            let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
            StatefulWidget::render(&list, rows[1], frame, &mut self.tor_list.borrow_mut());
        }

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" Details ");
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        if let Some(t) = self.selected_torrent() {
            self.view_torrent_details(t, pad(inner), frame);
        }
    }

    fn view_torrent_details(&self, t: &Torrent, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let kv = |k: &str, v: String| Line::from_spans([Span::styled(format!("{k:<10}"), th.dimmed()), Span::styled(v, th.text())]);
        let flags = format!("{}sequential  {}first/last",
            if t.seq_dl { "✓ " } else { "✗ " }, if t.f_l_piece_prio { "✓ " } else { "✗ " });
        let upload = if t.seed_allowed() { "seeding allowed  (u to turn off)".to_string() }
            else { "off · stops when complete  (u to seed)".to_string() };
        let mut lines = vec![
            Line::styled(t.name.clone(), th.title()),
            Line::raw(""),
            kv("state", state_label(t)),
            kv("progress", format!("{:.1}%  {} of {}", t.progress * 100.0, human_bytes(t.completed), human_bytes(t.size))),
            kv("speed", format!("↓ {}   ↑ {}", human_rate(t.dlspeed), human_rate(t.upspeed))),
            kv("eta", human_eta(t.eta)),
            kv("peers", format!("{} seeds, {} leechers", t.num_seeds, t.num_leechs)),
            kv("ratio", format!("{:.2}", t.ratio)),
            kv("flags", flags),
            kv("upload", upload),
            kv("path", t.save_path.clone()),
            kv("added", ago(t.added_on)),
        ];
        let head_h = lines.len() as u16;
        let mut text = Text::from_lines(lines.drain(..));
        text.push_line(Line::raw(""));
        let rows = Flex::vertical().constraints([Constraint::Fixed(head_h + 1), Constraint::Fill]).split(area);
        Paragraph::new(text).wrap(WrapMode::None).render(rows[0], frame);
        self.view_imdb_card(&t.name, rows[1], frame);
    }

    fn view_imdb_card(&self, key: &str, area: Rect, frame: &mut Frame) {
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

    fn view_search(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let rows = Flex::vertical().constraints([Constraint::Fixed(3), Constraint::Fill]).split(area);
        let title = if self.search_running { format!(" Search {} ", SPINNER[self.tick % 4]) } else { " Search ".to_string() };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input { th.border_focused() } else { th.border() })
            .title(&title).padding(Sides::horizontal(1));
        let inner = block.inner(rows[0]);
        block.render(rows[0], frame);
        self.search_input.render(inner, frame);

        let cols = Flex::horizontal().constraints([Constraint::Percentage(58.0), Constraint::Fill]).split(rows[1]);
        if self.search_browse {
            self.view_browse(cols[0], cols[1], frame);
            return;
        }
        let enabled = self.plugins.iter().filter(|p| p.enabled).count();
        let title = if self.plugins.is_empty() { format!(" Results ({}) ", self.search_results.len()) }
            else { format!(" Results ({}) · {}/{} providers ", self.search_results.len(), enabled, self.plugins.len()) };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input { th.border() } else { th.border_focused() }).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);
        if self.search_results.is_empty() {
            let msg = if self.search_running { "searching…" } else { "Type a query and press Enter to search 17 providers.\nPress Enter on an empty box (or b) to browse trending lists." };
            Paragraph::new(msg).style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(result_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let items: Vec<ListItem> = self.search_results.iter().map(|r| li(result_line(r, self.rating_for(r), inner.width.saturating_sub(2), th))).collect();
            let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
            StatefulWidget::render(&list, rows[1], frame, &mut self.search_list.borrow_mut());
        }

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" Details ");
        let inner = block.inner(cols[1]);
        block.render(cols[1], frame);
        if let Some(r) = self.selected_result() {
            let kv = |k: &str, v: String| Line::from_spans([Span::styled(format!("{k:<10}"), th.dimmed()), Span::styled(v, th.text())]);
            let lines = vec![
                Line::styled(r.file_name.clone(), th.title()),
                Line::raw(""),
                kv("size", human_bytes(r.file_size)),
                kv("peers", format!("{} seeds, {} leechers", r.seeders, r.leechers)),
                kv("uploaded", if r.pub_date > 0 { ymd(r.pub_date) } else { "unknown".into() }),
                kv("source", format!("{}  {}", r.engine, r.site_url)),
                Line::raw(""),
            ];
            let inner = pad(inner);
            let rows = Flex::vertical().constraints([Constraint::Fixed(7), Constraint::Fill]).split(inner);
            Paragraph::new(Text::from_lines(lines)).wrap(WrapMode::Word).render(rows[0], frame);
            self.view_imdb_card(&r.file_name, rows[1], frame);
        }
    }

    fn view_browse(&self, left: Rect, right: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Browse providers ");
        let inner = block.inner(left);
        block.render(left, frame);
        let w = inner.width.saturating_sub(2) as usize;
        let mut last = "";
        let items: Vec<ListItem> = self.browse_entries.iter().map(|e| {
            let prov = if e.provider == last { String::new() } else { e.provider.to_string() };
            last = e.provider;
            let is_site = matches!(e.source, browse::Source::Site(_));
            let glyph = if is_site { "󰖟 " } else { "󰈙 " };
            let label = format!("{glyph}{}", e.label);
            li(Line::from_spans([
                Span::styled(cell(&prov, 16), th.accented()),
                Span::styled(cell(&label, w.saturating_sub(16)), if is_site { th.dimmed() } else { th.text() }),
            ]))
        }).collect();
        let list = List::new(items).highlight_style(th.selected()).highlight_symbol("▌");
        StatefulWidget::render(&list, inner, frame, &mut self.browse_list.borrow_mut());

        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" About ");
        let inner = block.inner(right);
        block.render(right, frame);
        let sel = self.browse_list.borrow().selected().and_then(|i| self.browse_entries.get(i));
        let text = match sel.map(|e| e.provider) {
            Some("1337x") => "1337x sits behind a browser challenge, so its lists cannot be loaded here.\nEach entry opens the page in your browser.",
            Some("YTS") => "Latest YTS releases (1080p picked per film) via the YTS JSON API.\nThe main domain does not resolve from this network; mirrors are tried, otherwise use Open site.",
            Some("EZTV") => "The 100 most recent TV episodes on EZTV, sorted by seeders.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            Some("The Pirate Bay") => "Top-100 lists from The Pirate Bay's public JSON feed.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            _ => "",
        };
        let mut lines = vec![Line::styled(text.to_string(), th.text()), Line::raw("")];
        if let Some(l) = &self.browse_loading { lines.push(Line::styled(format!("{} loading {l}…", SPINNER[self.tick % 4]), th.warn())); }
        lines.push(Line::raw(""));
        lines.push(Line::from_spans([Span::styled("Enter", th.key()), Span::styled(" load list or open site   ", th.dimmed()), Span::styled("Esc", th.key()), Span::styled(" back to results", th.dimmed())]));
        Paragraph::new(Text::from_lines(lines)).wrap(WrapMode::Word).render(pad(inner), frame);
    }

    fn view_providers(&self, frame: &mut Frame) {
        let th = &self.theme;
        let Some(m) = &self.providers else { return };
        let b = frame.bounds();
        let w = 84.min(b.width);
        let h = (m.plugins.len() as u16 + 4).clamp(8, b.height);
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let bg = Style::new().bg(th.selection);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), bg)));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Search providers ").padding(Sides::horizontal(1));
        let inner = block.inner(area);
        block.render(area, frame);
        if m.loading {
            Paragraph::new(format!("{} loading providers…", SPINNER[self.tick % 4])).style(bg.fg(th.dim)).render(inner, frame);
            return;
        }
        if !m.error.is_empty() {
            Paragraph::new(m.error.clone()).style(bg.fg(th.red)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        if m.plugins.is_empty() {
            Paragraph::new("No search plugins installed. Run: fbittorrent search --install").style(bg.fg(th.dim)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let iw = inner.width.saturating_sub(2) as usize;
        let items: Vec<ListItem> = m.plugins.iter().map(|p| {
            let mark = if p.enabled { "[x] " } else { "[ ] " };
            let st = if p.enabled { bg.fg(th.fg) } else { bg.fg(th.dim) };
            let site = p.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
            li(Line::from_spans([
                Span::styled(mark, st),
                Span::styled(cell(&p.full_name, 22), st),
                Span::styled(cell(&site, iw.saturating_sub(4 + 22 + 1)), bg.fg(th.dim)),
            ]))
        }).collect();
        let list = List::new(items).style(bg).highlight_style(th.selected().bg(th.accent).fg(th.selection)).highlight_symbol("▌");
        StatefulWidget::render(&list, rows[0], frame, &mut m.list.borrow_mut());
        let on = m.plugins.iter().filter(|p| p.enabled).count();
        let foot = format!("{on} of {} enabled · Space toggle · a all · n none · o open site · Esc close", m.plugins.len());
        Paragraph::new(foot).style(bg.fg(th.dim)).wrap(WrapMode::None).render(rows[1], frame);
    }

    fn view_files(&self, frame: &mut Frame) {
        let th = &self.theme;
        let Some(m) = &self.files else { return };
        let b = frame.bounds();
        let w = (b.width * 9 / 10).max(40.min(b.width));
        let h = (b.height * 8 / 10).max(8.min(b.height));
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let title = format!(" Files · {} ", short(&m.name, (w as usize).saturating_sub(12)));
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title).padding(Sides::horizontal(1));
        let inner = block.inner(area);
        block.render(area, frame);
        let bg = Style::new().bg(th.selection);
        if m.loading {
            Paragraph::new(format!("{} loading file list…", SPINNER[self.tick % 4])).style(bg.fg(th.dim)).render(inner, frame);
            return;
        }
        if !m.error.is_empty() {
            Paragraph::new(m.error.clone()).style(bg.fg(th.red)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        if m.files.is_empty() {
            Paragraph::new("No file list yet. Metadata has not arrived (start the torrent and wait a moment), then press r.").style(bg.fg(th.dim)).wrap(WrapMode::Word).render(inner, frame);
            return;
        }
        let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let iw = inner.width.saturating_sub(2) as usize;
        let tail_w = 1 + 9 + 1 + 6 + 1 + 7;
        let name_w = iw.saturating_sub(4 + tail_w);
        let header = Line::from_spans([Span::styled(format!("      {}", cell("FILE", name_w)), bg.fg(th.dim).bold()), Span::styled(format!(" {:>9} {:>6} {:>7}", "SIZE", "DONE", "PRIO"), bg.fg(th.dim).bold())]);
        Paragraph::new(Text::from_line(header)).wrap(WrapMode::None).render(rows[0], frame);
        let items: Vec<ListItem> = m.files.iter().map(|f| {
            let (mark, prio, pst) = match f.priority {
                0 => ("[ ]", "skip", bg.fg(th.dim)),
                6 => ("[x]", "high", bg.fg(th.yellow)),
                7 => ("[x]", "max", bg.fg(th.green)),
                _ => ("[x]", "normal", bg.fg(th.fg)),
            };
            let name_style = if f.priority == 0 { bg.fg(th.dim) } else { bg.fg(th.fg) };
            li(Line::from_spans([
                Span::styled(format!("{mark} "), name_style),
                Span::styled(cell(&f.name, name_w), name_style),
                Span::styled(format!(" {:>9} {:>5.0}%", human_bytes(f.size), f.progress * 100.0), bg.fg(th.dim)),
                Span::styled(format!(" {prio:>7}"), pst),
            ]))
        }).collect();
        let selected: i64 = m.files.iter().filter(|f| f.priority > 0).map(|f| f.size).sum();
        let list = List::new(items).style(bg).highlight_style(th.selected().bg(th.accent).fg(th.selection)).highlight_symbol("▌");
        StatefulWidget::render(&list, rows[1], frame, &mut m.list.borrow_mut());
        let foot = format!("{} of {} files selected · {}     Space toggle · 1 normal · 2 high · 3 max · a all · n none · Esc close",
            m.files.iter().filter(|f| f.priority > 0).count(), m.files.len(), human_bytes(selected));
        Paragraph::new(foot).style(bg.fg(th.dim)).wrap(WrapMode::None).render(rows[2], frame);
    }

    fn view_rss(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Percentage(34.0), Constraint::Fill]).split(area);
        let title = format!(" Feeds ({}) ", self.feeds.len());
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.rss_focus_items { th.border() } else { th.border_focused() }).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);
        if self.feeds.is_empty() {
            Paragraph::new("No feeds yet.\n\nn  add a feed URL").style(th.dimmed()).render(pad(inner), frame);
        } else {
            let items: Vec<ListItem> = self.feeds.iter().map(|f| {
                let unread = f.articles.iter().filter(|a| !a.is_read).count();
                let w = inner.width.saturating_sub(2) as usize;
                let count = format!(" {unread}/{}", f.articles.len());
                let name = cell(&f.name, w.saturating_sub(count.len() + 1));
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
            Paragraph::new(if f.is_loading { "loading…" } else { "No items. Press r to refresh." }).style(th.dimmed()).render(pad(inner), frame);
            return;
        }
        let w = inner.width.saturating_sub(2) as usize;
        let items: Vec<ListItem> = f.articles.iter().map(|a| {
            let date = if a.ts > 0 { ymd(a.ts) } else { a.date.chars().take(11).collect() };
            let size = a.size.map(human_bytes).unwrap_or_default();
            let tail = format!(" {size:>9}  {date}");
            let title = cell(&a.title, w.saturating_sub(tail.len() + 2));
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

    fn view_help(&self, frame: &mut Frame) {
        let th = &self.theme;
        let b = frame.bounds();
        let w = 84.min(b.width);
        let h = 22.min(b.height);
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Keys ").padding(Sides::all(1))
            .style(Style::new().bg(th.selection));
        let inner = block.inner(area);
        block.render(area, frame);
        let text = "\
GLOBAL      1/2/3, Tab  switch screen        q quit   Q quit + stop qBittorrent
            a add magnet/URL (F2/F3/F4 toggle sequential / first-last / upload)
            c add from clipboard   r refresh   ? this help

TORRENTS    j/k move   Enter choose files + priorities   p pause/resume
            d remove   D delete with files   u allow/forbid upload (seeding)
            s sequential   f first/last piece   o open (file, or folder)
            i IMDb (look up, then open)   w Wikipedia

SEARCH      type + Enter run   Enter on empty box / b = browse trending lists
            s choose providers (checkboxes, persisted in qBittorrent)
            ↓/Esc leave the box   / edit query   Enter add selected
            i IMDb   w Wikipedia   O open the result's source page

FILES       Space toggle   1 normal   2 high   3 max   a all   n none

RSS         h/l feeds ⇄ items   Enter add item   n new feed   x remove feed
            r refresh feeds   m mark feed read   i IMDb for item

MOUSE       wheel scrolls the focused list   click on the tab strip switches screen

Every add: sequential + first/last on, upload OFF (32 KiB/s cap, stops when complete).
Flags column: S sequential, F first/last, U upload allowed.";
        Paragraph::new(text).style(Style::new().fg(th.fg).bg(th.selection)).wrap(WrapMode::None).render(inner, frame);
    }
}

// ---------------------------------------------------------------- formatting helpers

fn pad(r: Rect) -> Rect { r.inner(Sides::horizontal(1)) }

/// List row with a blank 2-column marker, so unselected rows sit at the same
/// x as the highlighted one (the widget only indents the selected row otherwise).
fn li<'t>(content: impl Into<Text<'t>>) -> ListItem<'static> {
    ListItem::new(content).marker(" ")
}

/// Display width in terminal cells.
fn dw(s: &str) -> usize { unicode_width::UnicodeWidthStr::width(s) }

/// Drop control and zero-width characters and collapse whitespace so that
/// measured width matches what the terminal paints.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if c.is_control() || w == 0 { continue; }
        if c.is_whitespace() {
            if !last_space { out.push(' '); }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// Fit a string into `max` cells: keep the head and the tail, ellipsis in between.
fn short(s: &str, max: usize) -> String {
    let s = clean(s);
    if max == 0 { return String::new(); }
    if dw(&s) <= max { return s; }
    if max <= 1 { return "…".into(); }
    let keep = max - 1;
    let head_w = keep * 2 / 3;
    let tail_w = keep - head_w;
    let mut head = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > head_w { break; }
        head.push(c); w += cw;
    }
    let mut tail_chars = Vec::new();
    let mut w = 0;
    for c in s.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > tail_w { break; }
        tail_chars.push(c); w += cw;
    }
    let tail: String = tail_chars.into_iter().rev().collect();
    let out = format!("{head}…{tail}");
    // Pad if wide chars left us a cell short so columns stay aligned.
    let pad = max.saturating_sub(dw(&out));
    format!("{out}{}", " ".repeat(pad))
}

/// Left-align `s` in exactly `w` cells.
fn cell(s: &str, w: usize) -> String {
    let t = short(s, w);
    format!("{t}{}", " ".repeat(w.saturating_sub(dw(&t))))
}

fn human_count(n: u32) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) } else if n >= 1000 { format!("{}k", n / 1000) } else { n.to_string() }
}

fn ago(unix: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let d = (now - unix).max(0);
    if d < 60 { "just now".into() }
    else if d < 3600 { format!("{}m ago", d / 60) }
    else if d < 86400 { format!("{}h ago", d / 3600) }
    else { format!("{}d ago", d / 86400) }
}

fn state_label(t: &Torrent) -> String {
    match t.state.as_str() {
        "downloading" | "forcedDL" => "downloading",
        "metaDL" | "forcedMetaDL" => "fetching metadata",
        "stalledDL" => "stalled",
        "queuedDL" => "queued",
        "checkingDL" | "checkingUP" | "checkingResumeData" => "checking",
        "uploading" | "forcedUP" => "seeding",
        "stalledUP" => "seeding (idle)",
        "queuedUP" => "queued (seed)",
        "stoppedDL" | "pausedDL" => "stopped",
        "stoppedUP" | "pausedUP" => "done",
        "error" => "error",
        "missingFiles" => "missing files",
        s => s,
    }.to_string()
}

fn torrent_line(t: &Torrent, width: u16, th: &Theme) -> Line<'static> {
    let bar_w: u16 = 10;
    let right = format!(" {:>6.1}% {:>11} {:>7} {}{}{}", t.progress * 100.0, if t.is_active() { human_rate(t.dlspeed) } else { human_bytes(t.size) },
        if t.is_active() { human_eta(t.eta) } else { "".into() }, if t.seq_dl { "S" } else { "·" }, if t.f_l_piece_prio { "F" } else { "·" },
        if t.seed_allowed() { "U" } else { "·" });
    let name_w = (width as usize).saturating_sub(bar_w as usize + 1 + right.len());
    let name = cell(&t.name, name_w);
    let padn = 0;
    let bar = MiniBar::new(t.progress, bar_w).show_percent(false).filled_char('█').empty_char('░').render_string();
    let bar_style = if t.is_active() { Style::new().fg(th.accent) } else if t.is_stopped() { th.dimmed() } else { th.ok() };
    let name_style = if t.is_stopped() { th.dimmed() } else if t.state == "error" || t.state == "missingFiles" { th.err() } else { th.text() };
    Line::from_spans([
        Span::styled(name, name_style),
        Span::raw(" ".repeat(padn + 1)),
        Span::styled(bar, bar_style),
        Span::styled(right, th.dimmed()),
    ])
}

const MAX_RESULTS: usize = 100;

const RESULT_TAIL_W: usize = 5 + 1 + 6 + 1 + 6 + 1 + 9 + 1 + 10 + 2 + 14;

/// Collapse the same torrent reported by several providers (same info hash),
/// keeping the copy with the most seeders.
fn dedupe_results(rs: Vec<SearchResult>) -> Vec<SearchResult> {
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

fn result_line(r: &SearchResult, rating: Option<f32>, width: u16, th: &Theme) -> Line<'static> {
    let stars = match rating { Some(v) => format!("{v:>5.1}"), None => "     ".into() };
    let star_style = match rating { Some(v) if v >= 7.5 => th.ok(), Some(v) if v >= 6.0 => th.warn(), Some(_) => th.dimmed(), None => th.dimmed() };
    let seeds = format!("{:>6}", if r.seeders < 0 { "?".to_string() } else { r.seeders.to_string() });
    let leech = format!(" {:>6}", if r.leechers < 0 { "?".to_string() } else { r.leechers.to_string() });
    let size = format!(" {:>9}", human_bytes(r.file_size));
    let date = format!(" {:>10}", ymd(r.pub_date));
    let engine = format!("  {}", cell(&r.engine, 14));
    let name_w = (width as usize).saturating_sub(RESULT_TAIL_W + 1);
    let name = cell(&r.file_name, name_w);
    let seed_style = if r.seeders >= 20 { th.ok() } else if r.seeders > 0 { th.warn() } else { th.dimmed() };
    Line::from_spans([
        Span::styled(name, th.text()),
        Span::raw(" "),
        Span::styled(stars, star_style),
        Span::raw(" "),
        Span::styled(seeds, seed_style),
        Span::styled(leech, th.dimmed()),
        Span::styled(size, th.text()),
        Span::styled(date, th.dimmed()),
        Span::styled(engine, th.dimmed()),
    ])
}

fn result_header(width: u16, th: &Theme) -> Line<'static> {
    let name_w = (width as usize).saturating_sub(RESULT_TAIL_W + 1);
    let tail = format!(" {:>5} {:>6} {:>6} {:>9} {:>10}  {:<14}", "IMDB", "SEEDS", "PEERS", "SIZE", "UPLOADED", "SOURCE");
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::styled(tail, th.dimmed().bold())])
}

fn torrent_header(width: u16, th: &Theme) -> Line<'static> {
    let right = format!(" {:>7} {:>11} {:>7} {}", "DONE", "SPEED", "ETA", "SFU");
    let name_w = (width as usize).saturating_sub(10 + 1 + right.len());
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::raw(" ".repeat(11)), Span::styled(right, th.dimmed().bold())])
}
