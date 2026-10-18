//! The fbittorrent terminal UI (FrankenTUI).

mod poster;
mod theme;

#[cfg(test)]
#[path = "tests/golden.rs"]
mod golden;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
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
use ftui::widgets::{StatefulWidget, StatusLine, Tab, Tabs, TabsState, Widget};
use ftui::{App, Cmd, Event, Frame, KeyCode, KeyEvent, KeyEventKind, Model, Modifiers, Style};

use crate::config::Config;
use crate::imdb::{self, Title};
use crate::browse;
use crate::shows::{self, Episode, Show, Shows};
use crate::qbt::{human_bytes, human_eta, human_rate, ymd, AddOptions, Client, RssFeed, SearchPlugin, SearchResult, Torrent, TorrentFile, TransferInfo};
use poster::{KittyPlacer, Poster};
use theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen { Torrents, Search, Shows, Rss }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseList { Torrents, Results, Browse, Feeds, Items, Shows, Episodes, Files, Providers }

impl Screen {
    fn index(self) -> usize { match self { Screen::Torrents => 0, Screen::Search => 1, Screen::Shows => 2, Screen::Rss => 3 } }
    fn from_index(i: usize) -> Self { match i % 4 { 0 => Screen::Torrents, 1 => Screen::Search, 2 => Screen::Shows, _ => Screen::Rss } }
}

/// A row identity survives source reordering and the same episode key in another show.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShowReleaseTarget { imdb: String, key: String }

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
    FollowShow,
    ConfirmUnfollow { imdb: String, title: String },
}

struct Prompt {
    kind: PromptKind,
    label: String,
    input: TextInput,
    yes_no: bool,
    /// Per-add overrides for the magnet prompt: (sequential, first/last, allow upload).
    add_flags: (bool, bool, bool),
    native_streaming: bool,
}

impl Prompt {
    /// Title with the live flag state, so the toggles are discoverable while typing.
    fn title(&self) -> String {
        if self.kind != PromptKind::AddMagnet { return self.label.clone(); }
        let (s, f, u) = self.add_flags;
        if self.native_streaming {
            return format!("{} · streaming automatic · F4 seed:{} ", self.label, if u { "on" } else { "off" });
        }
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

struct SettingsModal {
    cfg: Config,
    list: RefCell<ListState>,
    connections: Vec<Option<String>>,
    error: String,
}

pub enum Msg {
    Raw(Event),
    Poll,
    Torrents(Result<(Vec<Torrent>, TransferInfo), String>),
    ActionDone(Result<String, String>),
    Imdb(String, Result<Option<(Title, Option<Poster>)>, String>),
    SearchBatch { id: SubId, results: Vec<SearchResult>, done: bool, error: Option<String> },
    Rss { revision: u64, result: Result<Vec<RssFeed>, String> },
    RssAction { revision: u64, label: String, result: Result<Vec<RssFeed>, String>, error: Option<String> },
    RssReload,
    ShowGrab { imdb: String, episode: Episode, result: Result<(), String> },
    Browse(String, Result<Vec<SearchResult>, String>),
    /// Fast timer: look the highlighted title up once the cursor has rested on it.
    Hover,
    Files(String, Result<Vec<TorrentFile>, String>),
    Plugins(Result<Vec<SearchPlugin>, String>),
    Ratings(Result<Arc<imdb::RatingsIndex>, String>),
    /// Resolved (release name, IMDb id) pairs for the ratings column.
    Ids(Vec<(String, String)>),
    /// Episode list for a followed show (by IMDb id).
    Episodes(String, Result<shows::EpisodeCheck, String>),
    /// A show the user asked to follow, resolved on IMDb.
    Followed(Result<Title, String>),
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
    help_scroll: u16,
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
    settings: Option<SettingsModal>,
    plugins: Vec<SearchPlugin>,
    ratings: Option<Arc<imdb::RatingsIndex>>,
    name_ids: HashMap<String, String>,
    ids_inflight: bool,
    status_at: std::time::Instant,
    theme_mtime: Option<std::time::SystemTime>,
    hover_key: Option<String>,
    hover_since: std::time::Instant,
    pointer_motion_on_start: bool,
    marquee_key: Option<String>,
    marquee_since: std::time::Instant,
    last_bounds: Cell<Rect>,
    tab_rects: Cell<[Rect; 4]>,
    last_click: Option<(MouseList, String, std::time::Instant)>,
    kitty: Option<RefCell<KittyPlacer>>,
    /// Set by view() when a poster was drawn this frame; used to clear stale placements.
    poster_drawn: std::cell::Cell<bool>,
    // rss
    feeds: Vec<RssFeed>,
    feed_list: RefCell<ListState>,
    item_list: RefCell<ListState>,
    rss_focus_items: bool,
    rss_loaded: bool,
    rss_initial_refresh_pending: bool,
    rss_revision: u64,
    rss_busy: bool,
    rss_loading: bool,
    rss_retry_until: Option<std::time::Instant>,
    show_grabbing: HashSet<String>,
    // shows
    shows: Shows,
    show_list: RefCell<ListState>,
    ep_list: RefCell<ListState>,
    shows_focus_eps: bool,
    shows_refreshing: HashSet<String>,
    inbox_rows: Vec<ShowReleaseTarget>,
    /// Explicit undo restores the exact release last marked read in the inbox.
    inbox_last_read: Option<ShowReleaseTarget>,
    shows_rects: Cell<[Rect; 2]>,
    // imdb
    imdb: HashMap<String, ImdbState>,
    prompt: Option<Prompt>,
    status: String,
    status_is_error: bool,
}

const SPINNER: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];

pub fn run(cfg: Config, screen: Screen) -> Result<()> {
    let mut model = TuiApp::new(cfg, screen, Shows::load()?);
    model.pointer_motion_on_start = true;
    App::fullscreen(model).with_mouse().run()?;
    Ok(())
}

impl TuiApp {
    fn new(cfg: Config, screen: Screen, shows: Shows) -> Self {
        let theme = Theme::load();
        let mut search_input = TextInput::new()
            .with_placeholder("search torrents, Enter to run")
            .with_style(theme.text())
            .with_cursor_style(Style::new().bg(theme.accent).fg(theme.selection));
        search_input.set_focused(screen == Screen::Search);
        let mut app = TuiApp {
            cfg,
            theme,
            screen,
            tabs: RefCell::new(TabsState { active: screen.index(), offset: 0 }),
            show_help: false,
            help_scroll: 0,
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
            settings: None,
            plugins: vec![],
            ratings: None,
            name_ids: HashMap::new(),
            ids_inflight: false,
            status_at: std::time::Instant::now(),
            theme_mtime: std::fs::metadata(theme::colors_path()).and_then(|m| m.modified()).ok(),
            hover_key: None,
            hover_since: std::time::Instant::now(),
            pointer_motion_on_start: false,
            marquee_key: None,
            marquee_since: std::time::Instant::now(),
            last_bounds: Cell::new(Rect::new(0, 0, 0, 0)),
            tab_rects: Cell::new([Rect::new(0, 0, 0, 0); 4]),
            last_click: None,
            kitty: if poster::kitty_supported() { Some(RefCell::new(KittyPlacer::default())) } else { None },
            poster_drawn: std::cell::Cell::new(false),
            feeds: vec![],
            feed_list: RefCell::new(ListState::default()),
            item_list: RefCell::new(ListState::default()),
            rss_focus_items: false,
            rss_loaded: false,
            rss_initial_refresh_pending: true,
            rss_revision: 0,
            rss_busy: false,
            rss_loading: false,
            rss_retry_until: None,
            show_grabbing: HashSet::new(),
            shows,
            show_list: RefCell::new(ListState::default()),
            ep_list: RefCell::new(ListState::default()),
            shows_focus_eps: true,
            shows_refreshing: HashSet::new(),
            inbox_rows: vec![],
            inbox_last_read: None,
            shows_rects: Cell::new([Rect::new(0, 0, 0, 0); 2]),
            imdb: HashMap::new(),
            prompt: None,
            status: String::new(),
            status_is_error: false,
        };
        app.show_list.borrow_mut().select(Some(0));
        app.rebuild_inbox();
        app
    }
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
            sequential: self.cfg.native() || self.cfg.sequential,
            first_last: self.cfg.native() || self.cfg.first_last,
            seed: self.cfg.seed_by_default,
            stopped: false,
            save_path: self.cfg.save_path.clone(),
            category: self.cfg.category.clone(),
        }
    }

    fn add_search_result(&self, result: SearchResult) -> Cmd<Msg> {
        let opts = self.add_opts();
        let label = short(&result.file_name, 40);
        self.action(format!("added {label}"), move |c| c.add_search_result(&result, &opts))
    }

    fn load_rss(&mut self) -> Cmd<Msg> {
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
    fn rss_action<F>(&mut self, label: impl Into<String>, action: F) -> Cmd<Msg>
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

    fn refresh_rss(&mut self) -> Cmd<Msg> {
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

    // ---- shows

    fn shows_inbox(&self) -> bool { self.show_list.borrow().selected() == Some(0) }

    fn show_at(&self, index: usize) -> Option<&Show> {
        index.checked_sub(1).and_then(|i| self.shows.shows.get(i))
    }

    fn selected_show(&self) -> Option<&Show> {
        self.show_list.borrow().selected().and_then(|i| self.show_at(i))
    }

    fn episode_at(&self, index: usize) -> Option<(&Show, &Episode)> {
        if self.shows_inbox() {
            let target = self.inbox_rows.get(index)?;
            let show = self.shows.get(&target.imdb)?;
            Some((show, show.episode(&target.key)?))
        } else {
            let show = self.selected_show()?;
            show.episodes.get(index).map(|episode| (show, episode))
        }
    }

    fn selected_episode(&self) -> Option<(&Show, &Episode)> {
        self.ep_list.borrow().selected().and_then(|i| self.episode_at(i))
    }

    fn selected_release_target(&self) -> Option<ShowReleaseTarget> {
        if self.shows_inbox() {
            self.ep_list.borrow().selected().and_then(|i| self.inbox_rows.get(i)).cloned()
        } else {
            self.selected_episode().map(|(show, episode)| ShowReleaseTarget { imdb: show.imdb.clone(), key: episode.key.clone() })
        }
    }

    fn episode_count(&self) -> usize {
        if self.shows_inbox() { self.inbox_rows.len() } else { self.selected_show().map_or(0, |s| s.episodes.len()) }
    }

    fn rebuild_inbox(&mut self) {
        let selected = self.shows_inbox().then(|| self.selected_release_target()).flatten();
        let old_index = self.ep_list.borrow().selected().unwrap_or(0);
        let mut releases: Vec<_> = self.shows.shows.iter().flat_map(|show| show.episodes.iter()
            .filter(move |episode| show.is_new(&episode.key))
            .map(move |episode| (show, episode))).collect();
        releases.sort_by(|(a_show, a), (b_show, b)| b.released.cmp(&a.released)
            .then_with(|| a_show.title.cmp(&b_show.title)).then_with(|| b.key.cmp(&a.key)).then_with(|| a_show.imdb.cmp(&b_show.imdb)));
        self.inbox_rows = releases.into_iter().map(|(show, episode)| ShowReleaseTarget { imdb: show.imdb.clone(), key: episode.key.clone() }).collect();
        self.ep_list.borrow_mut().hovered = None;
        self.last_click = None;
        if self.shows_inbox() {
            let index = match selected {
                Some(target) => self.inbox_rows.iter().position(|row| *row == target)
                    .or_else(|| self.inbox_rows.len().checked_sub(1).map(|last| old_index.min(last))),
                None => (!self.inbox_rows.is_empty()).then_some(0),
            };
            self.ep_list.borrow_mut().select(index);
        }
    }

    fn select_show_parent(&mut self, index: usize) {
        self.shows_focus_eps = false;
        self.show_list.borrow_mut().select(Some(index));
        self.ep_list.borrow_mut().select(None);
        self.rebuild_inbox();
    }

    fn release_show(&self) -> Option<&Show> {
        if self.shows_inbox() { self.selected_episode().map(|(show, _)| show) } else { self.selected_show() }
    }
    fn save_shows(&mut self) -> bool {
        if let Err(e) = self.shows.save() { self.set_status(format!("saving shows: {e:#}"), true); false } else { true }
    }

    /// Fetch the episode list of one followed show in the background.
    fn refresh_show(&mut self, imdb: String) -> Cmd<Msg> {
        let Some(show) = self.shows.get(&imdb).cloned() else { return Cmd::none() };
        if !self.shows_refreshing.insert(imdb.clone()) { return Cmd::none(); }
        let quality = self.cfg.show_quality.clone();
        Cmd::task(move || {
            let r = shows::fetch_updates(&show, &quality).map_err(|e| format!("{e:#}"));
            Msg::Episodes(imdb, r)
        })
    }

    /// On launch and on `r`: refresh what is stale (or everything when forced).
    fn refresh_shows(&mut self, force: bool) -> Cmd<Msg> {
        let now = shows::now();
        let ids: Vec<String> = self.shows.shows.iter().filter(|s| force || s.is_stale(now)).map(|s| s.imdb.clone()).collect();
        if ids.is_empty() { return Cmd::none(); }
        if force { self.set_status(format!("checking {} show(s)…", ids.len()), false); }
        Cmd::batch(ids.into_iter().map(|id| self.refresh_show(id)).collect())
    }

    /// Follow the show behind a release name (or a known IMDb id); resolved off-thread.
    fn follow(&mut self, query: String, id: Option<String>) -> Cmd<Msg> {
        self.set_status(format!("looking up {}…", short(&query, 40)), false);
        Cmd::task(move || {
            let r = match id.filter(|i| i.starts_with("tt")) {
                Some(i) => imdb::lookup_id(&i).map_err(|e| e.to_string()).and_then(|t| t.ok_or_else(|| format!("IMDb knows no title {i}"))),
                None => shows::resolve_title(&query).map_err(|e| format!("{e:#}")),
            };
            Msg::Followed(r)
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
    fn on_hover(&mut self) -> Cmd<Msg> {
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

    fn marquee_target(&self) -> Option<String> {
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

    fn sync_marquee(&mut self) {
        let key = self.marquee_target();
        if key != self.marquee_key {
            self.marquee_key = key;
            self.marquee_since = std::time::Instant::now();
        }
    }

    fn marquee_elapsed(&self) -> Duration {
        let target = self.marquee_target();
        if target.is_some() && target == self.marquee_key { self.marquee_since.elapsed() } else { Duration::ZERO }
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

    fn mouse_list_at(&self, x: u16, y: u16) -> Option<(MouseList, Rect)> {
        let b = self.last_bounds.get();
        if b.width == 0 || self.show_help || self.prompt.is_some() || self.settings.is_some() { return None; }
        if let Some(m) = &self.providers {
            let area = providers_area(b, m.plugins.len());
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
            return rows[0].contains(x, y).then_some((MouseList::Providers, rows[0]));
        }
        if self.files.is_some() {
            let area = files_area(b);
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(1)]).split(inner);
            return rows[1].contains(x, y).then_some((MouseList::Files, rows[1]));
        }
        let root = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(0), Constraint::Fixed(2)]).split(b);
        match self.screen {
            Screen::Torrents => {
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

    fn mouse_list_state(&self, list: MouseList) -> &RefCell<ListState> {
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
        }
    }

    fn clear_pointer_hover(&self) {
        for state in [&self.tor_list, &self.search_list, &self.browse_list, &self.feed_list, &self.item_list, &self.show_list, &self.ep_list] {
            state.borrow_mut().hovered = None;
        }
        if let Some(modal) = &self.files { modal.list.borrow_mut().hovered = None; }
        if let Some(modal) = &self.providers { modal.list.borrow_mut().hovered = None; }
    }

    fn pointer_row(&self) -> Option<(MouseList, usize)> {
        if let Some(modal) = &self.files { return modal.list.borrow().hovered.map(|i| (MouseList::Files, i)); }
        if let Some(modal) = &self.providers { return modal.list.borrow().hovered.map(|i| (MouseList::Providers, i)); }
        [MouseList::Torrents, MouseList::Results, MouseList::Browse, MouseList::Feeds, MouseList::Items, MouseList::Shows, MouseList::Episodes]
            .into_iter().find_map(|list| self.mouse_list_state(list).borrow().hovered.map(|i| (list, i)))
    }

    fn mouse_list_len(&self, list: MouseList) -> usize {
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
        }
    }

    /// A double click must activate the same object, even if a refresh replaced its row.
    fn mouse_item_identity(&self, list: MouseList, index: usize) -> Option<String> {
        match list {
            MouseList::Torrents => self.torrents.get(index).map(|torrent| torrent.hash.clone()),
            MouseList::Results => self.search_results.get(index).map(|result| result.file_url.clone()),
            MouseList::Browse => self.browse_entries.get(index).map(|entry| format!("{:?}", entry.source)),
            MouseList::Feeds => self.feeds.get(index).map(|feed| if feed.uid.is_empty() { feed.name.clone() } else { feed.uid.clone() }),
            MouseList::Items => self.selected_feed().and_then(|feed| feed.articles.get(index).map(|article|
                format!("{:?}", (if feed.uid.is_empty() { &feed.name } else { &feed.uid }, &article.id)))),
            MouseList::Shows => if index == 0 { Some("all-unread".into()) } else { self.show_at(index).map(|show| show.imdb.clone()) },
            MouseList::Episodes => self.episode_at(index).map(|(show, episode)| format!("{:?}", (&show.imdb, &episode.key))),
            MouseList::Files => self.files.as_ref().and_then(|modal| modal.files.get(index).map(|file| format!("{:?}", (&modal.hash, file.index)))),
            MouseList::Providers => self.providers.as_ref().and_then(|modal| modal.plugins.get(index).map(|provider| provider.name.clone())),
        }
    }

    fn focus_mouse_list(&mut self, list: MouseList) {
        if matches!(list, MouseList::Results | MouseList::Browse) {
            self.search_focus_input = false;
            self.search_input.set_focused(false);
        }
        if matches!(list, MouseList::Feeds | MouseList::Items) { self.rss_focus_items = list == MouseList::Items; }
        if matches!(list, MouseList::Shows | MouseList::Episodes) { self.shows_focus_eps = list == MouseList::Episodes; }
    }

    /// Hover previews without changing the keyboard target; clicks select and double clicks activate.
    fn on_mouse(&mut self, m: ftui::MouseEvent) -> Cmd<Msg> {
        use ftui::{MouseButton, MouseEventKind};
        if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) { self.clear_pointer_hover(); }
        if self.settings.is_some() {
            let area = settings_area(self.last_bounds.get());
            let inner = Block::bordered().padding(Sides::horizontal(1)).inner(area);
            let rows = Rect::new(inner.x, inner.y, inner.width, inner.height.min(3));
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                if rows.contains(m.x, m.y) {
                    self.settings.as_ref().unwrap().list.borrow_mut().select(Some(usize::from(m.y - rows.y)));
                    return self.on_settings_key(KeyEvent::new(KeyCode::Right));
                }
                if inner.contains(m.x, m.y) && m.y == inner.y.saturating_add(3) {
                    return self.on_settings_key(KeyEvent::new(KeyCode::Enter));
                }
            }
            if m.kind == MouseEventKind::ScrollDown { return self.on_settings_key(KeyEvent::new(KeyCode::Down)); }
            if m.kind == MouseEventKind::ScrollUp { return self.on_settings_key(KeyEvent::new(KeyCode::Up)); }
            return Cmd::none();
        }
        if self.show_help {
            match m.kind {
                MouseEventKind::ScrollDown => self.help_scroll = self.help_scroll.saturating_add(3).min(self.help_max_scroll()),
                MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(3),
                MouseEventKind::Down(MouseButton::Left) => self.show_help = false,
                _ => {},
            }
            return Cmd::none();
        }
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.show_help { self.show_help = false; return Cmd::none(); }
            let b = self.last_bounds.get();
            if self.providers.is_some() && !providers_area(b, self.providers.as_ref().map_or(0, |p| p.plugins.len())).contains(m.x, m.y) {
                self.providers = None;
                return Cmd::none();
            }
            if self.files.is_some() && !files_area(b).contains(m.x, m.y) {
                self.files = None;
                return Cmd::none();
            }
        }
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) && m.y == 0
            && self.files.is_none() && self.providers.is_none() && self.prompt.is_none() && !self.show_help {
            let Some(index) = self.tab_rects.get().iter().position(|area| area.contains(m.x, m.y)) else { return Cmd::none() };
            let s = Screen::from_index(index);
            self.last_click = None;
            return self.set_screen(s);
        }
        if self.screen == Screen::Search && self.files.is_none() && self.providers.is_none()
            && self.prompt.is_none() && !self.show_help
            && matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) && (1..4).contains(&m.y) {
            self.search_focus_input = true;
            self.search_input.set_focused(true);
            return Cmd::none();
        }
        let Some((list, area)) = self.mouse_list_at(m.x, m.y) else { return Cmd::none() };
        if matches!(m.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) {
            self.focus_mouse_list(list);
            return self.on_key(KeyEvent::new(if m.kind == MouseEventKind::ScrollDown { KeyCode::Down } else { KeyCode::Up }));
        }
        if !matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)) { return Cmd::none(); }
        let state = self.mouse_list_state(list);
        let Some(index) = mouse_row_index(area, m.x, m.y, state.borrow().offset, self.mouse_list_len(list)) else { return Cmd::none() };
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
        self.clear_pointer_hover();
        self.screen = s;
        self.tabs.borrow_mut().select(s.index(), 4);
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
        let add_flags = (self.cfg.native() || self.cfg.sequential, self.cfg.native() || self.cfg.first_last, self.cfg.seed_by_default);
        self.prompt = Some(Prompt { kind, label: label.to_string(), input, yes_no, add_flags, native_streaming: self.cfg.native() });
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        self.search_input.set_focused(self.screen == Screen::Search && self.search_focus_input);
    }

    // ------------------------------------------------------------ key handling

    fn open_settings(&mut self) {
        let cfg = Config::load().unwrap_or_else(|_| self.cfg.clone());
        let mut connections = vec![Some("proton".into()), None];
        for name in crate::network_app::interfaces().unwrap_or_default() {
            if name != "lo" && !connections.contains(&Some(name.clone())) { connections.push(Some(name)); }
        }
        if !connections.contains(&cfg.network_interface) { connections.push(cfg.network_interface.clone()); }
        let mut list = ListState::default();
        list.select(Some(0));
        self.settings = Some(SettingsModal { cfg, connections, list: RefCell::new(list), error: String::new() });
    }

    fn on_settings_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        if key.code == KeyCode::Escape { self.settings = None; return Cmd::none(); }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(Modifiers::CTRL) { crate::network_app::request_close(); return Cmd::quit(); }
        if key.code == KeyCode::Enter {
            let values = self.settings.as_ref().unwrap().cfg.clone();
            let result = (|| -> Result<Config> {
                let mut latest = Config::load()?;
                let reopen = latest.peer_encryption != values.peer_encryption || latest.network_interface != values.network_interface;
                latest.max_active_downloads = values.max_active_downloads;
                latest.peer_encryption = values.peer_encryption;
                latest.network_interface = values.network_interface;
                latest.validate()?;
                if reopen {
                    latest.save()?;
                    crate::network_app::request_restart();
                } else {
                    Client::from_config(&self.cfg).set_queue_limit(latest.max_active_downloads)?;
                }
                Ok(latest)
            })();
            return match result {
                Ok(cfg) => {
                    self.cfg = cfg;
                    self.settings = None;
                    if crate::network_app::restart_requested() { Cmd::quit() }
                    else { self.set_status("download settings saved", false); self.poll() }
                }
                Err(error) => { self.settings.as_mut().unwrap().error = format!("{error:#}"); Cmd::none() }
            };
        }
        let modal = self.settings.as_mut().unwrap();
        let selected = modal.list.borrow().selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => modal.list.borrow_mut().select(Some((selected + 1) % 3)),
            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => modal.list.borrow_mut().select(Some((selected + 2) % 3)),
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Char(' ') => {
                let forward = !matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
                match selected {
                    0 => modal.cfg.max_active_downloads = if forward { modal.cfg.max_active_downloads.saturating_add(1) } else { modal.cfg.max_active_downloads.saturating_sub(1).max(1) },
                    1 => {
                        let policies = ["prefer", "require", "off"];
                        let index = policies.iter().position(|p| *p == modal.cfg.peer_encryption).unwrap_or(0);
                        modal.cfg.peer_encryption = policies[(index + if forward { 1 } else { 2 }) % 3].into();
                    }
                    _ => {
                        let index = modal.connections.iter().position(|v| *v == modal.cfg.network_interface).unwrap_or(0);
                        let count = modal.connections.len();
                        modal.cfg.network_interface = modal.connections[(index + if forward { 1 } else { count - 1 }) % count].clone();
                    }
                }
                modal.error.clear();
            }
            _ => {}
        }
        Cmd::none()
    }

    fn view_settings(&self, frame: &mut Frame) {
        let Some(modal) = &self.settings else { return };
        let area = settings_area(frame.bounds());
        let th = &self.theme;
        let bg = Style::new().bg(th.selection).fg(th.fg);
        Paragraph::new(Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), bg)))).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).padding(Sides::horizontal(1)).title(" Download settings ");
        let inner = block.inner(area);
        block.render(area, frame);
        let encryption = match modal.cfg.peer_encryption.as_str() { "require" => "Require encryption", "off" => "Off", _ => "Prefer encryption" };
        let vpn = match modal.cfg.network_interface.as_deref() { None => "Off · ordinary connection".into(), Some("proton") => "ProtonVPN · required".into(), Some(name) => format!("{name} · required") };
        let items = vec![li(Line::raw(format!("Simultaneous downloads   {}", modal.cfg.max_active_downloads))), li(Line::raw(format!("Peer encryption          {encryption}"))), li(Line::raw(format!("VPN connection           {vpn}")))];
        let rows = Rect::new(inner.x, inner.y, inner.width, inner.height.min(3));
        StatefulWidget::render(&List::new(items).style(bg).highlight_style(th.selected()).highlight_symbol("▌"), rows, frame, &mut modal.list.borrow_mut());
        let description = if !modal.error.is_empty() { modal.error.clone() } else {
            format!("↑/↓ choose · ←/→ change · Enter save · Esc cancel\n\nExtra downloads wait their turn. Encryption protects peer traffic; your IP remains visible.\n\nA required VPN blocks torrent transfers when disconnected. Feed and search websites use your usual connection.\n\n{}\nChanging encryption or VPN reopens the app's engine.", crate::network_app::state(&modal.cfg).message)
        };
        let remaining = inner.height.saturating_sub(3);
        Paragraph::new(description).style(if modal.error.is_empty() { bg.fg(th.dim) } else { bg.fg(th.red) }).wrap(WrapMode::Word).render(Rect::new(inner.x, inner.y.saturating_add(3), inner.width, remaining), frame);
    }

    fn on_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        if k.kind == KeyEventKind::Release { return Cmd::none(); }
        self.clear_pointer_hover();
        if self.settings.is_some() { return self.on_settings_key(k); }
        if self.show_help {
            let max_scroll = self.help_max_scroll();
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll = self.help_scroll.saturating_add(1).min(max_scroll),
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10).min(max_scroll),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => self.help_scroll = max_scroll,
                _ => self.show_help = false,
            }
            return Cmd::none();
        }
        if self.prompt.is_some() { return self.on_prompt_key(k); }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(Modifiers::CTRL) { crate::network_app::request_close(); return Cmd::quit(); }
        if self.files.is_some() { return self.on_files_key(k); }
        if self.providers.is_some() { return self.on_providers_key(k); }
        if k.code == KeyCode::F(6) && self.cfg.native() { self.open_settings(); return Cmd::none(); }

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
            KeyCode::Char('q') => { crate::network_app::request_close(); return Cmd::quit(); }
            KeyCode::Char('Q') => {
                crate::network_app::request_close();
                if !self.cfg.native() { let _ = crate::backend::stop(&self.cfg); }
                return Cmd::quit();
            }
            KeyCode::Char('?') => { self.show_help = true; self.help_scroll = 0; return Cmd::none(); }
            KeyCode::Char('1') => return self.set_screen(Screen::Torrents),
            KeyCode::Char('2') => return self.set_screen(Screen::Search),
            KeyCode::Char('3') => return self.set_screen(Screen::Shows),
            KeyCode::Char('4') => return self.set_screen(Screen::Rss),
            KeyCode::Tab => return self.set_screen(Screen::from_index(self.screen.index() + 1)),
            KeyCode::BackTab => return self.set_screen(Screen::from_index(self.screen.index() + 3)),
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
            KeyCode::Char('r') if !matches!(self.screen, Screen::Rss | Screen::Shows) => {
                if !self.online {
                    // Bring the backend back if it went away under us.
                    let cfg = self.cfg.clone();
                    let up = Cmd::task(move || {
                        let c = Client::from_config(&cfg);
                        match crate::backend::ensure_running(&cfg, &c) {
                            Ok(_) => Msg::ActionDone(Ok("torrent engine ready".into())),
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
            Screen::Shows => self.on_shows_key(k),
        }
    }

    fn on_prompt_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let Some(p) = self.prompt.as_mut() else { return Cmd::none() };
        if p.yes_no {
            let kind = p.kind.clone();
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') if k.modifiers.is_empty() || k.modifiers == Modifiers::SHIFT => {
                    self.close_prompt();
                    return match kind {
                        PromptKind::ConfirmDeleteTorrent { hash, files } =>
                            self.action(if files { "deleted with files" } else { "removed" }, move |c| c.delete(&[hash], files)),
                        PromptKind::ConfirmRemoveFeed { name } => {
                            self.rss_action(format!("removed feed {name}"), move |c| c.rss_remove(&name))
                        }
                        PromptKind::ConfirmUnfollow { imdb, title } => {
                            self.shows.unfollow(&imdb);
                            if self.inbox_last_read.as_ref().is_some_and(|target| target.imdb == imdb) { self.inbox_last_read = None; }
                            self.save_shows();
                            let n = self.shows.shows.len() + 1;
                            let mut st = self.show_list.borrow_mut();
                            match st.selected() { Some(i) if i < n => {}, _ => st.select(n.checked_sub(1)) }
                            drop(st);
                            self.ep_list.borrow_mut().select(None);
                            self.rebuild_inbox();
                            self.set_status(format!("unfollowed {title}"), false);
                            Cmd::none()
                        }
                        _ => Cmd::none(),
                    };
                }
                _ => { self.close_prompt(); return Cmd::none(); }
            }
        }
        match k.code {
            KeyCode::Escape => { self.close_prompt(); Cmd::none() }
            KeyCode::F(2) if p.kind == PromptKind::AddMagnet && !p.native_streaming => { p.add_flags.0 = !p.add_flags.0; Cmd::none() }
            KeyCode::F(3) if p.kind == PromptKind::AddMagnet && !p.native_streaming => { p.add_flags.1 = !p.add_flags.1; Cmd::none() }
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
                        self.rss_action("feed added", move |c| c.rss_add_feed(&value, name.as_deref()))
                    }
                    PromptKind::FollowShow => self.follow(value, None),
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
            KeyCode::Char('s') if !self.cfg.native() => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled sequential", move |c| c.toggle_sequential(&[t.hash]));
            },
            KeyCode::Char('f') if !self.cfg.native() => if let Some(t) = self.selected_torrent().cloned() {
                return self.action("toggled first/last", move |c| c.toggle_first_last(&[t.hash]));
            },
            KeyCode::Char('u') => if let Some(t) = self.selected_torrent().cloned() {
                let allow = !t.seed_allowed();
                return self.action(if allow { "seeding allowed" } else { "seeding off · upload capped" },
                    move |c| if allow { c.allow_seeding(&[t.hash]) } else { c.deny_seeding(&[t.hash]) });
            },
            KeyCode::Char('d') | KeyCode::Delete => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: false },
                    &format!("Remove '{}' (keep files)? y/N", short(&t.name, 50)), true);
            },
            KeyCode::Char('D') => if let Some(t) = self.selected_torrent().cloned() {
                self.open_prompt(PromptKind::ConfirmDeleteTorrent { hash: t.hash.clone(), files: true },
                    &format!("DELETE '{}' AND ITS FILES? y/N", short(&t.name, 50)), true);
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
            KeyCode::Char('F') => if let Some(t) = self.selected_torrent() {
                let name = t.name.clone();
                return self.follow(name, None);
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
            KeyCode::Char('2') if !self.cfg.native() => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 6); },
            KeyCode::Char('3') if !self.cfg.native() => if let Some(f) = cur { return self.set_file_prio(hash, vec![f.index], 7); },
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
                return self.add_search_result(r);
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
            KeyCode::Char('f') => if let Some(r) = self.selected_result() {
                let id = if !r.imdb.is_empty() { Some(r.imdb.clone()) } else { self.name_ids.get(&r.file_name).filter(|i| !i.is_empty()).cloned() };
                let name = r.file_name.clone();
                return self.follow(name, id);
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
                self.open_prompt(PromptKind::ConfirmRemoveFeed { name: name.clone() }, &format!("Remove feed '{name}'? y/N"), true);
            },
            KeyCode::Char('r') => return self.refresh_rss(),
            KeyCode::Char('R') => return self.load_rss(),
            KeyCode::Char('m') | KeyCode::Char(' ') => {
                if !self.rss_focus_items {
                    self.set_status("Enter opens items · Shift+M marks this feed's items read", false);
                } else if let Some((feed, id, read)) = self.selected_feed().and_then(|f| self.item_list.borrow().selected().and_then(|i| f.articles.get(i)).map(|a| (f.name.clone(), a.id.clone(), !self.cfg.native() || !a.is_read))) {
                    return self.rss_action(format!("item marked {}", if read { "read" } else { "unread" }), move |c| c.rss_set_read(&feed, &id, read));
                }
            },
            KeyCode::Char('M') => if let Some(f) = self.selected_feed() {
                let name = f.name.clone();
                return self.rss_action(format!("marked {name} as read"), move |c| c.rss_mark_read(&name, None));
            },
            KeyCode::Enter if !self.rss_focus_items => {
                if ni > 0 { self.rss_focus_items = true; if self.item_list.borrow().selected().is_none() { self.item_list.borrow_mut().select(Some(0)); } }
            },
            KeyCode::Enter => if self.rss_focus_items {
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
            KeyCode::Char('i') => if self.rss_focus_items {
                let idx = self.item_list.borrow().selected();
                let name = idx.and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| a.title.clone()));
                if let Some(name) = name { return self.lookup_imdb(name, None); }
            },
            KeyCode::Char('f') => if self.rss_focus_items {
                let idx = self.item_list.borrow().selected();
                let name = idx.and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)).map(|a| a.title.clone()));
                if let Some(name) = name { return self.follow(name, None); }
            },
            _ => {}
        }
        Cmd::none()
    }

    fn on_shows_key(&mut self, k: KeyEvent) -> Cmd<Msg> {
        let ns = self.shows.shows.len() + 1;
        let ne = self.episode_count();
        let navigation = match k.code {
            KeyCode::Left | KeyCode::Char('h') => { self.shows_focus_eps = false; true }
            KeyCode::Right | KeyCode::Char('l') => {
                self.shows_focus_eps = true;
                if ne > 0 && self.ep_list.borrow().selected().is_none() { self.ep_list.borrow_mut().select(Some(0)); }
                true
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Up | KeyCode::Char('k') | KeyCode::Home | KeyCode::Char('g') | KeyCode::End | KeyCode::Char('G') => {
                let state = if self.shows_focus_eps { &self.ep_list } else { &self.show_list };
                let n = if self.shows_focus_eps { ne } else { ns };
                let old = state.borrow().selected();
                match k.code {
                    KeyCode::Down | KeyCode::Char('j') => if n > 0 { state.borrow_mut().select_next(n); },
                    KeyCode::Up | KeyCode::Char('k') => state.borrow_mut().select_previous(),
                    KeyCode::Home | KeyCode::Char('g') => state.borrow_mut().select((n > 0).then_some(0)),
                    _ => state.borrow_mut().select(n.checked_sub(1)),
                }
                if !self.shows_focus_eps {
                    let selected = self.show_list.borrow().selected();
                    if selected != old { if let Some(index) = selected { self.select_show_parent(index); } }
                }
                true
            }
            _ => false,
        };
        if navigation { return Cmd::none(); }
        match k.code {
            KeyCode::Char('n') | KeyCode::Char('+') => { self.open_prompt(PromptKind::FollowShow, "Follow show: title or IMDb id (tt…)", false); }
            KeyCode::Char('x') => if let Some(s) = self.selected_show() {
                let (imdb, title) = (s.imdb.clone(), s.label());
                self.open_prompt(PromptKind::ConfirmUnfollow { imdb, title: title.clone() }, &format!("Unfollow '{title}'? y/N"), true);
            },
            KeyCode::Char('r') => return self.refresh_shows(true),
            KeyCode::Char('R') => if let Some(s) = self.release_show() { let id = s.imdb.clone(); return self.refresh_show(id); },
            KeyCode::Char('m') | KeyCode::Char(' ') => {
                if !self.shows_focus_eps {
                    self.set_status("Enter opens releases · Shift+M marks one selected show's releases read", false);
                } else if let Some(label) = self.mark_selected_release() {
                    if self.save_shows() { self.set_status(label, false); }
                }
            }
            KeyCode::Char('u') if self.shows_inbox() => {
                if let Some(label) = self.undo_inbox_read() {
                    if self.save_shows() { self.set_status(label, false); }
                }
            }
            KeyCode::Char('M') => {
                if let Some(s) = self.selected_show() {
                    let (imdb, title) = (s.imdb.clone(), s.title.clone());
                    let n = self.shows.get_mut(&imdb).map(|s| s.mark_all_seen()).unwrap_or(0);
                    self.rebuild_inbox();
                    self.save_shows();
                    self.set_status(format!("{title}: {n} release(s) marked read"), false);
                } else {
                    self.set_status("Shift+M applies to one show. Select a followed show on the left first.", false);
                }
            }
            KeyCode::Enter => {
                if !self.shows_focus_eps {
                    self.shows_focus_eps = true;
                    if ne > 0 && self.ep_list.borrow().selected().is_none() { self.ep_list.borrow_mut().select(Some(0)); }
                    return Cmd::none();
                }
                let picked = self.selected_episode().map(|(s, e)| (s.imdb.clone(), e.clone()));
                if let Some((imdb, e)) = picked {
                    let token = format!("{imdb}:{}", e.key);
                    if !self.show_grabbing.insert(token) { return Cmd::none(); }
                    let cfg = self.cfg.clone();
                    let opts = self.add_opts();
                    return Cmd::task(move || {
                        let c = Client::from_config(&cfg);
                        let result = c.login().and_then(|_| c.add_url(&e.magnet, &opts)).map_err(|err| err.to_string());
                        Msg::ShowGrab { imdb, episode: e, result }
                    });
                }
            }
            KeyCode::Char('i') => if let Some(s) = self.release_show() {
                let (name, id) = (s.label(), Some(s.imdb.clone()));
                return self.open_imdb(name, id);
            },
            KeyCode::Char('w') => if let Some(s) = self.release_show() {
                let (name, id) = (s.label(), Some(s.imdb.clone()));
                return self.open_wiki(name, id);
            },
            KeyCode::Char('O') => if let Some(s) = self.release_show() {
                let _ = std::process::Command::new("xdg-open").arg(format!("https://eztvx.to/search/{}", crate::qbt::urlencode(&s.title))).spawn();
            },
            _ => {}
        }
        Cmd::none()
    }

    fn mark_selected_release(&mut self) -> Option<String> {
        let target = self.selected_release_target()?;
        let inbox = self.shows_inbox();
        let show = self.shows.get_mut(&target.imdb)?;
        let read = if inbox { show.seen.insert(target.key.clone()); true } else { show.toggle_seen(&target.key) };
        let label = format!("{} {} marked {}{}", show.title, target.key, if read { "read" } else { "unread" }, if inbox { " · (U)ndo" } else { "" });
        if inbox { self.inbox_last_read = Some(target); }
        self.rebuild_inbox();
        Some(label)
    }

    fn undo_inbox_read(&mut self) -> Option<String> {
        let target = self.inbox_last_read.take()?;
        let show = self.shows.get_mut(&target.imdb)?;
        show.seen.remove(&target.key);
        let label = format!("{} {} restored to unread", show.title, target.key);
        self.rebuild_inbox();
        self.ep_list.borrow_mut().select(self.inbox_rows.iter().position(|row| *row == target));
        Some(label)
    }

}

// ---------------------------------------------------------------- model

impl Model for TuiApp {
    type Message = Msg;

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
                if self.cfg.native() && crate::engine::quit_requested() { return Cmd::quit(); }
                if self.cfg.native() && (crate::network_app::close_requested() || crate::network_app::check_transition(&self.cfg)) { return Cmd::quit(); }
                self.tick += 1; self.maybe_reload_theme(); self.poll()
            }
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

// ---------------------------------------------------------------- views

impl TuiApp {
    fn view_tabs(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Fill, Constraint::FitContent]).split(area);
        let new = self.inbox_rows.len();
        let shows_tab = format!(" 3 Shows ({new}) ");
        let labels = [" 1 Torrents ".to_string(), " 2 Search ".to_string(), shows_tab.clone(), " 4 RSS ".to_string()];
        let mut x = area.x;
        let regions = labels.map(|label| {
            // Tabs wraps each title with two visible bracket/space cells.
            let width = (dw(&label).saturating_add(2) as u16).min(cols[0].right().saturating_sub(x));
            let region = Rect::new(x, area.y, width, area.height);
            x = x.saturating_add(width);
            region
        });
        self.tab_rects.set(regions);
        let tabs = Tabs::new([Tab::new(" 1 Torrents "), Tab::new(" 2 Search "), Tab::new(shows_tab.as_str()), Tab::new(" 4 RSS ")])
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
        let tabs_right = regions.iter().map(Rect::right).max().unwrap_or(area.x);
        let right_x = (area.x + area.width.saturating_sub(w + 1)).max(tabs_right).min(area.right());
        let r = Rect::new(right_x, area.y, w.min(area.right().saturating_sub(right_x)), 1);
        Paragraph::new(Text::from_line(right)).render(r, frame);
    }

    fn view_status(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() { return; }
        let th = &self.theme;
        let (context, hints, last): (&str, Vec<&str>, &str) = if self.show_help {
            ("Help", vec!["(↑↓) Scroll", "(PgUp/PgDn) Page"], "(Esc) Close")
        } else if self.settings.is_some() {
            ("Settings", vec!["(↑↓) Choose", "(←→) Change", "(Enter) Save"], "(Esc) Cancel")
        } else if let Some(prompt) = &self.prompt {
            if prompt.yes_no {
                ("Confirm", vec!["(y) Remove"], "(Esc) Cancel")
            } else {
                let submit = match prompt.kind {
                    PromptKind::FollowShow => "(Enter) Follow",
                    PromptKind::AddFeed => "(Enter) Add feed",
                    _ => "(Enter) Add",
                };
                ("Input", vec![submit], "(Esc) Cancel")
            }
        } else if self.files.is_some() {
            let priority = if self.cfg.native() { "(1) Include" } else { "(1/2/3) Priority" };
            ("Files", vec!["(Space) Include/skip", priority, "(A)ll", "(N)one"], "(Esc) Close")
        } else if self.providers.is_some() {
            ("Providers", vec!["(Space) Toggle", "(A)ll", "(N)one", "(O)pen site"], "(Esc) Close")
        } else { match self.screen {
            Screen::Torrents => {
                let mut hints = if let Some(torrent) = self.selected_torrent() {
                    let mut hints = vec!["(Enter) Files", "(O)pen", if torrent.complete_no_seed() { "(U) Seed" }
                        else if torrent.is_stopped() { "(P) Resume" } else { "(P)ause" },
                        "(D) Remove", "(Shift+D) Delete files"];
                    if !torrent.complete_no_seed() {
                        hints.push(if torrent.seed_allowed() { "(U) Seed off" } else { "(U) Seed on" });
                    }
                    hints
                } else { Vec::new() };
                hints.extend(["(A)dd", "(C)lipboard"]);
                if self.cfg.native() { hints.push("(F6) Settings"); }
                else { hints.extend(["(S)equential", "(F) First/last"]); }
                ("Torrents", hints, "(?) Help")
            }
            Screen::Search if self.search_focus_input => {
                let enter = if self.search_input.value().trim().is_empty() { "(Enter) Browse" } else { "(Enter) Search" };
                ("Query", vec![enter, "(↓/Esc) Results"], "(Esc, ?) Help")
            }
            Screen::Search if self.search_browse => {
                let site = self.browse_list.borrow().selected().and_then(|i| self.browse_entries.get(i))
                    .is_some_and(|entry| matches!(entry.source, browse::Source::Site(_)));
                ("Browse", vec![if site { "(Enter) Open site" } else { "(Enter) Load list" }, "(Esc) Results", "(/) Query"], "(?) Help")
            }
            Screen::Search => {
                let mut hints = Vec::new();
                if self.selected_result().is_some() { hints.extend(["(Enter) Download", "(Shift+O) Source", "(F)ollow show"]); }
                hints.extend(["(/) Query", "(S) Providers", "(B)rowse lists"]);
                ("Results", hints, "(?) Help")
            }
            Screen::Rss if self.rss_focus_items => {
                let picked = self.selected_feed().and_then(|feed| self.item_list.borrow().selected().and_then(|i| feed.articles.get(i)));
                let mut hints = Vec::new();
                if let Some(article) = picked {
                    hints.extend(["(Enter) Download", if self.cfg.native() && article.is_read { "(M)ark unread" } else { "(M)ark read" }]);
                }
                if self.selected_feed().is_some() { hints.push("(Shift+M) All read"); }
                hints.extend(["(←) Feeds", "(R)efresh", "(F)ollow show"]);
                ("Items", hints, "(?) Help")
            }
            Screen::Rss => {
                let mut hints = Vec::new();
                if self.selected_feed().is_some() { hints.extend(["(Enter) Items", "(Shift+M) All read"]); }
                hints.extend(["(N)ew feed", "(R)efresh"]);
                if self.selected_feed().is_some() { hints.push("(X) Remove feed"); }
                ("Feeds", hints, "(?) Help")
            }
            Screen::Shows if self.shows_focus_eps => {
                let mut hints = Vec::new();
                if let Some((show, episode)) = self.selected_episode() {
                    hints.extend(["(Enter) Download", if show.is_new(&episode.key) { "(M)ark read" } else { "(M)ark unread" }]);
                }
                if self.shows_inbox() {
                    if self.inbox_last_read.is_some() { hints.push("(U)ndo"); }
                    hints.extend(["(←) Shows", "(R) Check all"]);
                    ("Unread", hints, "(?) Help")
                } else {
                    if self.selected_show().is_some() { hints.push("(Shift+M) All read"); }
                    hints.extend(["(←) Shows", "(Shift+R) Check show", "(R) Check all"]);
                    ("Releases", hints, "(?) Help")
                }
            }
            Screen::Shows => {
                let mut hints = Vec::new();
                hints.push("(Enter) Releases");
                if self.selected_show().is_some() { hints.push("(Shift+M) All read"); }
                hints.extend(["(N) Follow show", "(R) Check all"]);
                if self.selected_show().is_some() { hints.extend(["(Shift+R) Check show", "(X) Unfollow"]); }
                ("Shows", hints, "(?) Help")
            }
        }};
        StatusLine::new().style(th.dimmed()).render(area, frame);
        // Keep the exit/help affordance and pane identity visible. Feedback has
        // its own row so a success or error cannot conceal the available actions.
        let width = usize::from(area.width);
        let last_width = dw(last).min(width);
        let last_x = area.right().saturating_sub(last_width as u16);
        Paragraph::new(Text::from_line(shortcut_line(&[last], th))).wrap(WrapMode::None)
            .render(Rect::new(last_x, area.y, last_width as u16, 1), frame);
        let available = width.saturating_sub(last_width + 2);
        let context = format!("[{context}]");
        let context_width = dw(&context).min(available);
        Paragraph::new(context.clone()).style(th.title()).wrap(WrapMode::None)
            .render(Rect::new(area.x, area.y, context_width as u16, 1), frame);
        let labels_width = available.saturating_sub(context_width + 1);
        let mut shown = Vec::new();
        let mut used = 0;
        for hint in hints {
            let needed = dw(hint) + if shown.is_empty() { 0 } else { 2 };
            if used + needed <= labels_width { shown.push(hint); used += needed; }
        }
        let actions_x = area.x.saturating_add(context_width as u16).saturating_add(1);
        Paragraph::new(Text::from_line(shortcut_line(&shown, th))).wrap(WrapMode::None)
            .render(Rect::new(actions_x, area.y, labels_width as u16, 1), frame);
        if area.height > 1 {
            let feedback = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
            Paragraph::new(" ".repeat(width)).style(th.dimmed()).render(feedback, frame);
            if !self.status.is_empty() {
                Paragraph::new(short(&self.status, width)).style(if self.status_is_error { th.err() } else { th.ok() })
                    .wrap(WrapMode::None).render(feedback, frame);
            } else if self.screen == Screen::Search && self.search_focus_input {
                Paragraph::new("Type a query; Enter searches, Esc leaves the query box.")
                    .style(th.dimmed()).wrap(WrapMode::None).render(feedback, frame);
            } else if self.prompt.is_none() && self.files.is_none() && self.providers.is_none() && self.settings.is_none() && !self.show_help {
                Paragraph::new("(1–4) Screens  (Tab/Shift+Tab) Switch screen  (↑↓) Move")
                    .style(th.dimmed()).wrap(WrapMode::None).render(feedback, frame);
            }
        }
    }

    fn view_torrents(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = torrent_columns(area);
        let queued = self.torrents.iter().filter(|t| t.state == "queuedDL").count();
        let network = crate::network_app::state(&self.cfg);
        let title = if self.cfg.native() && !network.ready { format!(" {} ", network.message) }
            else if queued > 0 { format!(" Torrents ({}) · {queued} queued ", self.torrents.len()) }
            else { format!(" Torrents ({}) ", self.torrents.len()) };
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);

        if !self.online {
            let msg = format!("Torrent engine is not reachable.\n{}\n\n(R)etry", self.backend_error);
            Paragraph::new(msg).style(th.err()).wrap(WrapMode::Word).render(pad(inner), frame);
        } else if self.torrents.is_empty() {
            let text = if !network.ready { format!("{}\n\nConnect your VPN to start transfers.\n(F6) Download and VPN settings\n\nYour saved downloads stay here.", network.message) }
                else { "No torrents.\n\n(A)dd a magnet link or URL\n(C)lipboard: add a copied link\n(2) Search".into() };
            Paragraph::new(text)
                .style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(torrent_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let selected = readable_index(&self.tor_list);
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.torrents.iter().enumerate().map(|(i, t)|
                li(torrent_line(t, inner.width.saturating_sub(2), th, (selected == Some(i)).then_some(elapsed)))).collect();
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
        let flags = if self.cfg.native() { "streaming priority automatic".to_string() } else {
            format!("{}sequential  {}first/last", if t.seq_dl { "✓ " } else { "✗ " }, if t.f_l_piece_prio { "✓ " } else { "✗ " })
        };
        let upload = if t.seed_allowed() { "seeding allowed  (u to turn off)".to_string() }
            else { "32 KiB/s cap · stops when complete  (u to seed)".to_string() };
        let mut lines = vec![
            Line::styled(t.name.clone(), th.title()),
            Line::raw(""),
            kv("state", state_label(t)),
            kv("progress", if t.progress_known { format!("{:.1}%  {} of {}", t.progress * 100.0, human_bytes(t.completed), human_bytes(t.size)) } else { "checked when the VPN connects".into() }),
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

        let cols = detail_columns(rows[1]);
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
            let msg = if self.search_running { "Searching… Results appear as providers reply." }
                else { "Type a query, then (Enter) Search.\nEmpty query + Enter browses provider lists.\n(/) Edit query from results · (B)rowse lists" };
            Paragraph::new(msg).style(th.dimmed()).render(pad(inner), frame);
        } else {
            let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill]).split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            Paragraph::new(Text::from_line(result_header(inner.width.saturating_sub(2), th))).wrap(WrapMode::None).render(hdr, frame);
            let selected = readable_index(&self.search_list);
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.search_results.iter().enumerate().map(|(i, r)|
                li(result_line(r, self.rating_for(r), inner.width.saturating_sub(2), th, (selected == Some(i)).then_some(elapsed)))).collect();
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
        let selected = readable_index(&self.browse_list);
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = self.browse_entries.iter().enumerate().map(|(i, e)| {
            let prov = if e.provider == last { String::new() } else { e.provider.to_string() };
            last = e.provider;
            let is_site = matches!(e.source, browse::Source::Site(_));
            let glyph = if is_site { "󰖟 " } else { "󰈙 " };
            let label = format!("{glyph}{}", e.label);
            li(Line::from_spans([
                Span::styled(cell(&prov, 16), th.accented()),
                Span::styled(readable_cell(&label, w.saturating_sub(16), (selected == Some(i)).then_some(elapsed)), if is_site { th.dimmed() } else { th.text() }),
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
        let area = providers_area(frame.bounds(), m.plugins.len());
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
        let selected = readable_index(&m.list);
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = m.plugins.iter().enumerate().map(|(i, p)| {
            let mark = if p.enabled { "[x] " } else { "[ ] " };
            let st = if p.enabled { bg.fg(th.fg) } else { bg.fg(th.dim) };
            let site = p.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
            li(Line::from_spans([
                Span::styled(mark, st),
                Span::styled(readable_cell(&p.full_name, 22, (selected == Some(i)).then_some(elapsed)), st),
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
        let area = files_area(frame.bounds());
        let w = area.width;
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
        let selected_row = readable_index(&m.list);
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = m.files.iter().enumerate().map(|(i, f)| {
            let (mark, prio, pst) = match f.priority {
                0 => ("[ ]", "skip", bg.fg(th.dim)),
                6 => ("[x]", "high", bg.fg(th.yellow)),
                7 => ("[x]", "max", bg.fg(th.green)),
                _ => ("[x]", if self.cfg.native() { "include" } else { "normal" }, bg.fg(th.fg)),
            };
            let name_style = if f.priority == 0 { bg.fg(th.dim) } else { bg.fg(th.fg) };
            li(Line::from_spans([
                Span::styled(format!("{mark} "), name_style),
                Span::styled(readable_cell(&f.name, name_w, (selected_row == Some(i)).then_some(elapsed)), name_style),
                Span::styled(format!(" {:>9} {:>6}", human_bytes(f.size), if f.progress_known { format!("{:.0}%", f.progress * 100.0) } else { "—".into() }), bg.fg(th.dim)),
                Span::styled(format!(" {prio:>7}"), pst),
            ]))
        }).collect();
        let selected: i64 = m.files.iter().filter(|f| f.priority > 0).map(|f| f.size).sum();
        let list = List::new(items).style(bg).highlight_style(th.selected().bg(th.accent).fg(th.selection)).highlight_symbol("▌");
        StatefulWidget::render(&list, rows[1], frame, &mut m.list.borrow_mut());
        let controls = if self.cfg.native() { "Space include/skip · a/n all/none · Esc close" }
            else { "Space toggle · 1 normal · 2 high · 3 max · a/n all/none · Esc close" };
        let foot = format!("{}/{} · {} · {controls}",
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
            Paragraph::new("No feeds yet.\n\n(N)ew feed: add an RSS URL").style(th.dimmed()).render(pad(inner), frame);
        } else {
            let selected = readable_index(&self.feed_list);
            let elapsed = self.marquee_elapsed();
            let items: Vec<ListItem> = self.feeds.iter().enumerate().map(|(i, f)| {
                let unread = f.articles.iter().filter(|a| !a.is_read).count();
                let w = inner.width.saturating_sub(2) as usize;
                let count = format!(" {unread}/{}", f.articles.len());
                let name = readable_cell(&f.name, w.saturating_sub(count.len() + 1),
                    (selected == Some(i) && (!self.rss_focus_items || self.feed_list.borrow().hovered.is_some())).then_some(elapsed));
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
        let selected = readable_index(&self.item_list);
        let elapsed = self.marquee_elapsed();
        let items: Vec<ListItem> = f.articles.iter().enumerate().map(|(i, a)| {
            let date = if a.ts > 0 { ymd(a.ts) } else { a.date.chars().take(11).collect() };
            let size = a.size.map(human_bytes).unwrap_or_default();
            let tail = format!(" {size:>9}  {date}");
            let title = readable_cell(&a.title, w.saturating_sub(tail.len() + 2),
                (selected == Some(i) && (self.rss_focus_items || self.item_list.borrow().hovered.is_some())).then_some(elapsed));
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

    fn view_shows(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Percentage(if area.width < 100 { 28.0 } else { 32.0 }), Constraint::Fill]).split(area);
        let card_h = if cols[0].height >= 24 { 11 } else { 0 };
        let left = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(card_h)]).split(cols[0]);
        let unread = self.inbox_rows.len();
        let title = format!(" Shows ({}) ", self.shows.shows.len());
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.shows_focus_eps { th.border() } else { th.border_focused() }).title(&title);
        let inner = block.inner(left[0]);
        block.render(left[0], frame);
        let w = inner.width.saturating_sub(2) as usize;
        let selected = readable_index(&self.show_list);
        let elapsed = self.marquee_elapsed();
        let mut parents = vec![li(Line::from_spans([
            Span::styled(cell("All unread", w.saturating_sub(dw(&unread.to_string()) + 1)), th.text().bold()),
            Span::styled(format!(" {unread}"), th.accented()),
        ]))];
        parents.extend(self.shows.shows.iter().enumerate().map(|(i, show)| {
            let n = show.new_count();
            let busy = self.shows_refreshing.contains(&show.imdb);
            let warning = show.error.is_some() || (show.last_checked > 0 && !show.catalogue_complete);
            let tail = if busy { format!(" {}", SPINNER[self.tick % 4]) } else if warning { " !".into() } else if n > 0 { format!(" {n}") } else { String::new() };
            let name = readable_cell(&show.label(), w.saturating_sub(dw(&tail)), (selected == Some(i + 1) && (!self.shows_focus_eps || self.show_list.borrow().hovered.is_some())).then_some(elapsed));
            li(Line::from_spans([
                Span::styled(name, if n > 0 { th.text().bold() } else { th.text() }),
                Span::styled(tail, if warning && !busy { th.err() } else { th.accented() }),
            ]))
        }));
        StatefulWidget::render(&List::new(parents).highlight_style(th.selected()).highlight_symbol("▌"), inner, frame, &mut self.show_list.borrow_mut());
        if card_h > 0 {
            let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border()).title(" IMDb ");
            let inner = block.inner(left[1]);
            block.render(left[1], frame);
            if let Some(show) = self.release_show() { self.view_imdb_card(&show.label(), pad(inner), frame); }
        }

        let inbox = self.shows_inbox();
        let title = if inbox { format!(" All unread · {unread} releases · newest first ") }
            else if let Some(show) = self.selected_show() { format!(" {} · {} releases ", show.label(), show.episodes.len()) }
            else { " Releases ".into() };
        let block = Block::bordered().border_type(BorderType::Rounded)
            .border_style(if self.shows_focus_eps { th.border_focused() } else { th.border() }).title(&title);
        let right = block.inner(cols[1]);
        block.render(cols[1], frame);
        let summary = Rect::new(right.x, right.y, right.width, right.height.min(1));
        let releases_inner = Rect::new(right.x, right.y.saturating_add(summary.height), right.width, right.height.saturating_sub(summary.height));
        let warning = if inbox {
            let warnings: Vec<String> = self.shows.shows.iter().filter_map(|show| {
                if let Some(error) = &show.error { Some(format!("{}: {error}", show.title)) }
                else if show.last_checked > 0 && !show.catalogue_complete { Some(format!("{}: history incomplete", show.title)) }
                else { None }
            }).collect();
            (!warnings.is_empty()).then(|| format!("Source warnings · {} · (R) retry", warnings.join("; ")))
        } else { self.selected_show().and_then(|show| show.error.clone().or_else(|| (show.last_checked > 0 && !show.catalogue_complete).then(|| "History incomplete".into()))).map(|error| format!("Check: {error} · (Shift+R) retry")) };
        let summary_text = if inbox {
            let checking = self.shows_refreshing.len();
            let unchecked = self.shows.shows.iter().filter(|show| show.last_checked == 0).count();
            if checking > 0 { format!("Checking {checking} show(s)… · cached releases") }
            else if unchecked > 0 { format!("{unchecked} show(s) unchecked · (R) Check all") }
            else { format!("Cached releases · (R) Check all · (←) Browse shows") }
        } else if let Some(show) = self.selected_show() {
            if self.shows_refreshing.contains(&show.imdb) { "Checking EZTV… · cached releases".into() }
            else {
                let checked = if show.last_checked > 0 { format!("checked {}", ago(show.last_checked)) } else { "never checked".into() };
                format!("{checked} · prefers {}", show.quality.as_deref().unwrap_or(&self.cfg.show_quality))
            }
        } else { String::new() };
        Paragraph::new(summary_text).style(th.dimmed()).wrap(WrapMode::None).render(summary, frame);
        let rows = episode_rows(releases_inner, warning.is_some());
        self.shows_rects.set([inner, rows[1]]);
        if let Some(warning) = &warning { Paragraph::new(warning.clone()).style(th.err()).wrap(WrapMode::Word).render(rows[2], frame); }
        if self.episode_count() == 0 {
            let message = if self.shows.shows.is_empty() {
                "No shows followed yet.\n\n(N) Follow a title or IMDb id\n(F)ollow a Search or RSS item\n(Shift+F) Follow a torrent\n\nChecks find releases. Enter downloads your choice."
            } else if inbox {
                if !self.shows_refreshing.is_empty() { "Checking for releases…\n\nNo unread cached releases." }
                else if self.shows.shows.iter().any(|show| show.last_checked == 0) { "Not checked yet.\n\n(R) Check all shows." }
                else if warning.is_some() { "No unread cached releases.\n\nSource warnings remain below.\n(←) Browse a show's full release history." }
                else { "All caught up.\n\n(←) Browse a show's full release history.\n(R) Check for releases." }
            } else if self.selected_show().is_some_and(|show| show.last_checked == 0) { "Not checked yet.\n(Shift+R) Check this show" }
            else { "No cached releases for this show.\n(Shift+R) Check · (2) Search · (Shift+O) EZTV" };
            Paragraph::new(message).style(th.dimmed()).wrap(WrapMode::Word).render(pad(rows[1]), frame);
            return;
        }
        let w = rows[1].width.saturating_sub(2) as usize;
        // At 80 columns the show name and release key carry the task; metadata is optional.
        let metadata = if inbox && w < 80 { 0 } else { episode_metadata_width(w) };
        let name_w = w.saturating_sub(11 + metadata);
        let header = format!("   {:<7} {}{}", "RELEASE", cell(if inbox { "SHOW" } else { "TITLE" }, name_w), if metadata > 0 { episode_metadata(None, w) } else { String::new() });
        Paragraph::new(header).style(th.dimmed()).wrap(WrapMode::None).render(Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), rows[0].height), frame);
        let selected = readable_index(&self.ep_list);
        let have: HashMap<(String, String), &Torrent> = self.torrents.iter().filter_map(|torrent| {
            let parsed = crate::release::parse(&torrent.name);
            Some(((parsed.title.to_ascii_lowercase(), parsed.episode?), torrent))
        }).collect();
        let releases: Vec<(&Show, &Episode)> = if inbox {
            let catalogue: HashMap<(&str, &str), (&Show, &Episode)> = self.shows.shows.iter().flat_map(|show| show.episodes.iter()
                .map(move |episode| ((show.imdb.as_str(), episode.key.as_str()), (show, episode)))).collect();
            self.inbox_rows.iter().filter_map(|target| catalogue.get(&(target.imdb.as_str(), target.key.as_str())).copied()).collect()
        } else { self.selected_show().map(|show| show.episodes.iter().map(|episode| (show, episode)).collect()).unwrap_or_default() };
        let items: Vec<ListItem> = releases.into_iter().enumerate().map(|(index, (show, episode))| {
            let unread = show.is_new(&episode.key);
            let torrent = have.get(&(show.title.to_ascii_lowercase(), episode.key.clone()));
            let (mark, style) = match torrent {
                Some(t) if t.progress_known && t.progress >= 1.0 => ("✓", th.ok()),
                Some(t) if matches!(t.state.as_str(), "queuedDL" | "waitingVPN") || !t.progress_known => ("…", th.dimmed()),
                Some(t) if matches!(t.state.as_str(), "stoppedDL" | "pausedDL") => ("‖", th.dimmed()),
                Some(_) => ("↓", th.accented()),
                None if show.grabbed.contains_key(&episode.key) => ("+", th.dimmed()),
                None => (" ", th.dimmed()),
            };
            let name = if inbox {
                if w >= 80 { format!("{} · {}", show.label(), episode.title) } else { show.label() }
            } else { episode.title.clone() };
            let text_style = if unread { th.text().bold() } else { th.dimmed() };
            li(Line::from_spans([
                Span::styled(if unread { "●" } else { " " }, th.accented()),
                Span::styled(format!("{mark} "), style),
                Span::styled(format!("{:<7} ", episode.key), text_style),
                Span::styled(readable_cell(&name, name_w, (selected == Some(index) && (self.shows_focus_eps || self.ep_list.borrow().hovered.is_some())).then_some(elapsed)), text_style),
                Span::styled(if metadata > 0 { episode_metadata(Some(episode), w) } else { String::new() }, th.dimmed()),
            ]))
        }).collect();
        StatefulWidget::render(&List::new(items).highlight_style(th.selected()).highlight_symbol("▌"), rows[1], frame, &mut self.ep_list.borrow_mut());
    }

    fn help_text(&self) -> String {
        let lifetime = if self.cfg.native() { "(Q)uit and stop the engine · Shift+Q also quits" }
            else { "(Q)uit · Shift+Q quits and stops qBittorrent" };
        let add = if self.cfg.native() { "(A)dd URL/magnet · F4 toggles seeding in the add prompt" }
            else { "(A)dd URL/magnet · F2/F3/F4 sequential/first-last/upload" };
        let files = if self.cfg.native() { "FILES    (Space) Include/skip · (1) Include\n         (A)ll files · (N)one · (Esc/Enter) Close" }
            else { "FILES    (Space) Include/skip · (1/2/3) Normal/high/max\n         (A)ll files · (N)one · (Esc/Enter) Close" };
        let streaming = if self.cfg.native() { "Streaming priority is automatic; file selection supports include/skip." }
            else { "(S)equential · (F) First/last priority" };
        let rss_read = if self.cfg.native() { "(M)ark read/unread or Space: selected item only" }
            else { "(M)ark read or Space: selected item only (legacy engine)" };
        format!("\
GLOBAL   (1) Torrents · (2) Search · (3) Shows · (4) RSS
         (Tab/Shift+Tab) Next/previous screen
         (?) Help · (Esc) Leave input, close a dialog or return
         Letters in parentheses use lowercase; Shift is explicit.
         (F6) Download queue, encryption and VPN (native engine)
         {lifetime}
         {add}
         (C)lipboard · arrows or j/k move · g/Shift+G first/last

TORRENTS (Enter) Files · (O)pen downloaded file/folder
         (P)ause/resume · (U) Allow/stop seeding
         (D) Remove, keep files · (Shift+D) Delete with files
         (I) IMDb · (W)ikipedia · (Shift+F) Follow this show
         {streaming}

SEARCH   Query: type + (Enter) Search; empty query browses lists
         (Down/Esc) Leave query · (/) Edit query from results
         Results: (Enter) Download · (Shift+O) Source page
         (S) Providers · (B)rowse lists · (F)ollow show
         (I) IMDb · (W)ikipedia; press Esc before ? in the query

BROWSE   (Enter) Load selected list or open a browser-only site
         (Esc) Results · (/) Query

PROVIDERS (Space) Toggle · (A)ll · (N)one · (O)pen site
          (Esc/Enter) Close; choices are saved

{files}

RSS      (Left/Right) or h/l: focus Feeds/Items
         Feeds: (Enter) Items · (N)ew feed · (X) Remove feed
         Items: (Enter) Download
         {rss_read}
         (Shift+M) Mark all items in the selected feed read
         (R)efresh feeds · (Shift+R) Reload saved feeds
         (F)ollow show from item · (I) Look up title
         Selecting or hovering leaves read state unchanged.

SHOWS    Starts in All unread, across every followed show.
         (Left/Right) or h/l: focus Shows/Releases
         Shows: All unread or one show · (Enter) Releases
         (N) Follow title/IMDb id
         Releases: (Enter) Download episode or season pack
         Inbox: (M)ark read or Space removes the selected release
         (U)ndo restores the last inbox read action
         One show: (M)ark read/unread or Space toggles one release
         (Shift+M) Mark all releases of one show read
         Select a followed show first; unavailable in All unread.
         (R) Check all · (Shift+R) Check selected show
         (X) Unfollow · (Shift+O) EZTV · (I) IMDb · (W)ikipedia
         Counts show unread releases, including old releases.
         Download history and read state are stored separately.
         Cached releases survive source removal and years between runs.
         Checking finds releases; downloading is always your choice.

MOUSE    Hover previews names; keyboard target stays selected.
         Click selects and focuses · wheel scrolls the pointed pane.
         Double-click activates the clicked row.
         A feed/show activates its child pane; an item downloads.
         Files/providers: double-click toggles the selected row.
         Click tabs or query box · long selected names scroll.

RELEASES ● unread · ↓ downloading · … queued/waiting · ‖ paused
         ✓ complete · + added earlier; separate from read state.

Native feed checks and transfers stop when the owning app closes.
Defaults: seeding off; (U) allows it per torrent.")
    }

    fn help_max_scroll(&self) -> u16 {
        let height = self.last_bounds.get().height.min(34).saturating_sub(5);
        (self.help_text().lines().count() as u16).saturating_sub(height)
    }

    fn view_help(&self, frame: &mut Frame) {
        let th = &self.theme;
        let b = frame.bounds();
        let w = 84.min(b.width);
        let h = 34.min(b.height);
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Keys ").padding(Sides::all(1))
            .style(Style::new().bg(th.selection));
        let inner = block.inner(area);
        block.render(area, frame);
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let text = self.help_text();
        let scroll = self.help_scroll.min(self.help_max_scroll());
        Paragraph::new(text).style(Style::new().fg(th.fg).bg(th.selection)).scroll((scroll, 0)).wrap(WrapMode::None).render(rows[0], frame);
        Paragraph::new("j/k ↑/↓ wheel scroll · PgUp/PgDn · Esc or click close")
            .style(Style::new().fg(th.accent).bg(th.selection)).wrap(WrapMode::None).render(rows[1], frame);
    }

}

// ---------------------------------------------------------------- formatting helpers

/// Style the parenthesized key separately from its action while retaining the
/// complete label as readable text in terminals with reduced styling.
fn shortcut_line(labels: &[&str], th: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        if index > 0 { spans.push(Span::raw("  ")); }
        if let Some(end) = label.find(')') {
            spans.push(Span::styled(label[..=end].to_string(), th.key()));
            spans.push(Span::styled(label[end + 1..].to_string(), th.dimmed()));
        } else {
            spans.push(Span::styled((*label).to_string(), th.dimmed()));
        }
    }
    Line::from_spans(spans)
}

fn pad(r: Rect) -> Rect { r.inner(Sides::horizontal(1)) }

fn detail_columns(area: Rect) -> [Rect; 2] {
    columns_with_details(area, 100)
}

fn torrent_columns(area: Rect) -> [Rect; 2] {
    columns_with_details(area, 140)
}

fn columns_with_details(area: Rect, minimum_width: u16) -> [Rect; 2] {
    if area.width < minimum_width {
        [area, Rect::new(area.x.saturating_add(area.width), area.y, 0, area.height)]
    } else {
        let cols = Flex::horizontal().constraints([Constraint::Percentage(58.0), Constraint::Fill]).split(area);
        [cols[0], cols[1]]
    }
}

fn episode_rows(inner: Rect, warning: bool) -> [Rect; 3] {
    let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(if warning { 2 } else { 0 })]).split(inner);
    [rows[0], rows[1], rows[2]]
}

fn episode_metadata_width(width: usize) -> usize {
    if width >= 80 { 1 + 5 + 2 + 9 + 2 + 5 + 2 + 10 }
    else if width >= 60 { 1 + 5 + 2 + 9 + 2 + 5 }
    else { 1 + 5 + 2 + 5 }
}

fn episode_metadata(episode: Option<&Episode>, width: usize) -> String {
    let (quality, size, seeds, date) = match episode {
        Some(e) => (e.quality.clone(), human_bytes(e.size), e.seeds.to_string(), if e.released > 0 { ymd(e.released) } else { String::new() }),
        None => ("QUAL".into(), "SIZE".into(), "SEEDS".into(), "DATE".into()),
    };
    if width >= 80 { format!(" {quality:>5}  {size:>9}  {seeds:>5}  {date:>10}") }
    else if width >= 60 { format!(" {quality:>5}  {size:>9}  {seeds:>5}") }
    else { format!(" {quality:>5}  {seeds:>5}") }
}

fn providers_area(b: Rect, count: usize) -> Rect {
    let w = 84.min(b.width);
    let h = (count.saturating_add(4).min(u16::MAX as usize) as u16).max(8).min(b.height);
    Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h)
}

fn settings_area(b: Rect) -> Rect {
    let width = b.width.min(78);
    let height = b.height.min(18);
    Rect::new(b.x + b.width.saturating_sub(width) / 2, b.y + b.height.saturating_sub(height) / 2, width, height)
}

fn files_area(b: Rect) -> Rect {
    let w = (b.width * 9 / 10).max(40.min(b.width));
    let h = (b.height * 8 / 10).max(8.min(b.height));
    Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h)
}

fn readable_index(state: &RefCell<ListState>) -> Option<usize> {
    let state = state.borrow();
    state.hovered.or(state.selected())
}

fn mouse_row_index(area: Rect, x: u16, y: u16, offset: usize, len: usize) -> Option<usize> {
    area.contains(x, y).then(|| offset + usize::from(y - area.y)).filter(|&i| i < len)
}

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

/// After a pause, slide a fixed-width window across the entire selected name.
fn readable_cell(s: &str, w: usize, selected_for: Option<Duration>) -> String {
    let Some(elapsed) = selected_for else { return cell(s, w) };
    let name = clean(s);
    if w < 2 || dw(&name) <= w || elapsed < Duration::from_millis(900) { return cell(&name, w); }
    let offset = ((elapsed - Duration::from_millis(900)).as_millis() / 160) as usize % (dw(&name) + 3);
    let looped = format!("{name}   {name}");
    let mut out = String::new();
    let mut pos = 0;
    let mut used = 0;
    for c in looped.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if pos + cw <= offset { pos += cw; continue; }
        if pos < offset { out.push(' '); used += 1; pos += cw; continue; }
        if used + cw > w { break; }
        out.push(c); used += cw; pos += cw;
    }
    out.push_str(&" ".repeat(w.saturating_sub(used)));
    out
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

fn torrent_line(t: &Torrent, width: u16, th: &Theme, selected_for: Option<Duration>) -> Line<'static> {
    let bar_w: u16 = 10;
    let percent = if t.progress_known { format!("{:>6.1}%", t.progress * 100.0) } else { "      —".into() };
    let right = format!(" {percent} {:>11} {:>7} {}{}{}", if t.is_active() { human_rate(t.dlspeed) } else { human_bytes(t.size) },
        if t.is_active() { human_eta(t.eta) } else { "".into() }, if t.seq_dl { "S" } else { "·" }, if t.f_l_piece_prio { "F" } else { "·" },
        if t.seed_allowed() { "U" } else { "·" });
    let name_w = (width as usize).saturating_sub(bar_w as usize + 1 + right.len());
    let name = readable_cell(&t.name, name_w, selected_for);
    let padn = 0;
    let bar = if t.is_queued() { "  Queued  ".into() }
        else if t.state == "waitingVPN" { " Wait VPN ".into() }
        else { MiniBar::new(t.progress, bar_w).show_percent(false).filled_char('█').empty_char('░').render_string() };
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

fn result_tail_width(width: u16) -> usize {
    if width >= 88 { RESULT_TAIL_W } else if width >= 60 { 5 + 1 + 6 + 1 + 9 } else { 6 + 1 + 9 }
}

fn result_line(r: &SearchResult, rating: Option<f32>, width: u16, th: &Theme, selected_for: Option<Duration>) -> Line<'static> {
    let stars = match rating { Some(v) => format!("{v:>5.1}"), None => "     ".into() };
    let star_style = match rating { Some(v) if v >= 7.5 => th.ok(), Some(v) if v >= 6.0 => th.warn(), Some(_) => th.dimmed(), None => th.dimmed() };
    let seeds = format!("{:>6}", if r.seeders < 0 { "?".to_string() } else { r.seeders.to_string() });
    let size = format!(" {:>9}", human_bytes(r.file_size));
    let name_w = (width as usize).saturating_sub(result_tail_width(width) + 1);
    let seed_style = if r.seeders >= 20 { th.ok() } else if r.seeders > 0 { th.warn() } else { th.dimmed() };
    let mut spans = vec![Span::styled(readable_cell(&r.file_name, name_w, selected_for), th.text()), Span::raw(" ")];
    if width >= 60 { spans.extend([Span::styled(stars, star_style), Span::raw(" ")]); }
    spans.push(Span::styled(seeds, seed_style));
    if width >= 88 { spans.push(Span::styled(format!(" {:>6}", if r.leechers < 0 { "?".to_string() } else { r.leechers.to_string() }), th.dimmed())); }
    spans.push(Span::styled(size, th.text()));
    if width >= 88 {
        spans.extend([Span::styled(format!(" {:>10}", ymd(r.pub_date)), th.dimmed()), Span::styled(format!("  {}", cell(&r.engine, 14)), th.dimmed())]);
    }
    Line::from_spans(spans)
}

fn result_header(width: u16, th: &Theme) -> Line<'static> {
    let name_w = (width as usize).saturating_sub(result_tail_width(width) + 1);
    let tail = if width >= 88 { format!(" {:>5} {:>6} {:>6} {:>9} {:>10}  {:<14}", "IMDB", "SEEDS", "PEERS", "SIZE", "UPLOADED", "SOURCE") }
        else if width >= 60 { format!(" {:>5} {:>6} {:>9}", "IMDB", "SEEDS", "SIZE") }
        else { format!(" {:>6} {:>9}", "SEEDS", "SIZE") };
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::styled(tail, th.dimmed().bold())])
}

fn torrent_header(width: u16, th: &Theme) -> Line<'static> {
    let right = format!(" {:>7} {:>11} {:>7} {}", "DONE", "SPEED", "ETA", "SFU");
    let name_w = (width as usize).saturating_sub(10 + 1 + right.len());
    Line::from_spans([Span::styled(cell("NAME", name_w), th.dimmed().bold()), Span::raw(" ".repeat(11)), Span::styled(right, th.dimmed().bold())])
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    fn destructive_keys(open: fn(&mut TuiApp), unfollow: bool) {
        for code in [KeyCode::Enter, KeyCode::Escape, KeyCode::Char('n'), KeyCode::Char(' '), KeyCode::Char('q'), KeyCode::Char('y'), KeyCode::Char('Y')] {
            let mut app = inbox_app();
            app.torrents = vec![Torrent { hash: "fixture-hash".into(), name: "Fixture".into(), ..Default::default() }];
            app.tor_list.borrow_mut().select(Some(0));
            app.feeds = vec![feed(vec![])];
            app.feed_list.borrow_mut().select(Some(0));
            open(&mut app);
            assert!(app.prompt.as_ref().unwrap().label.ends_with("y/N"));
            let text = render(&app, 120, 30);
            assert!(text.contains("(y) Remove") && text.contains("(Esc) Cancel") && text.contains("default: No"));
            let command = app.on_key(KeyEvent::new(code));
            assert!(app.prompt.is_none());
            let yes = matches!(code, KeyCode::Char('y') | KeyCode::Char('Y'));
            if unfollow {
                assert!(matches!(command, Cmd::None));
                assert_eq!(app.shows.shows.len(), if yes { 1 } else { 2 });
                assert_eq!(app.status.starts_with("unfollowed"), yes);
            } else {
                assert_eq!(matches!(command, Cmd::Task(..)), yes, "key {code:?}");
                assert_eq!(app.torrents.len(), 1);
                assert_eq!(app.feeds.len(), 1);
            }
        }
    }

    #[test]
    fn destructive_remove_requires_y() {
        destructive_keys(|app| { app.on_torrents_key(KeyEvent::new(KeyCode::Char('d'))); }, false);
    }

    #[test]
    fn destructive_delete_files_requires_y() {
        destructive_keys(|app| { app.on_torrents_key(KeyEvent::new(KeyCode::Char('D'))); }, false);
    }

    #[test]
    fn destructive_remove_feed_requires_y() {
        destructive_keys(|app| { app.screen = Screen::Rss; app.on_rss_key(KeyEvent::new(KeyCode::Char('x'))); }, false);
    }

    #[test]
    fn destructive_unfollow_requires_y() {
        // Unfollow persists synchronously. Run this unit case in a child with
        // private XDG roots rather than mutating process-wide env in parallel.
        if std::env::var_os("FBT_P013_UNFOLLOW_CHILD").is_none() {
            let root = std::env::temp_dir().join(format!("fbt-p013-unfollow-{}", std::process::id()));
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "tui::interaction_tests::destructive_unfollow_requires_y", "--nocapture"])
                .env("FBT_P013_UNFOLLOW_CHILD", "1");
            for name in ["XDG_RUNTIME_DIR", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
                let path = root.join(name);
                std::fs::create_dir_all(&path).unwrap();
                child.env(name, path);
            }
            let output = child.output().unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed;"), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        assert!(shows::path().starts_with(std::env::temp_dir()), "unfollow fixture requires temporary XDG_DATA_HOME");
        destructive_keys(|app| { app.screen = Screen::Shows; app.select_show_parent(1); app.on_shows_key(KeyEvent::new(KeyCode::Char('x'))); }, true);
    }

    fn app() -> TuiApp {
        let mut app = TuiApp::new(Config::default(), Screen::Torrents, Shows::default());
        app.kitty = None;
        app.rss_initial_refresh_pending = false;
        app
    }

    fn article(id: &str, title: &str) -> crate::qbt::RssArticle {
        crate::qbt::RssArticle { id: id.into(), title: title.into(), torrent_url: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(), ..Default::default() }
    }

    fn feed(articles: Vec<crate::qbt::RssArticle>) -> RssFeed {
        RssFeed { name: "Example feed".into(), title: "Example feed".into(), articles, ..Default::default() }
    }

    fn render(app: &TuiApp, width: u16, height: u16) -> String {
        let mut pool = ftui::GraphemePool::new();
        let mut frame = Frame::new(width, height, &mut pool);
        app.view(&mut frame);
        (0..height).map(|y| (0..width).map(|x| frame.buffer.get(x, y).and_then(|cell| cell.content.as_char()).unwrap_or(' ')).collect::<String>())
            .collect::<Vec<_>>().join("\n")
    }

    fn episode(key: &str, released: i64) -> Episode {
        Episode { key: key.into(), season: 1, episode: if key == "S01" { 0 } else { 1 }, title: format!("Fixture.{key}.1080p"), quality: "1080p".into(), magnet: format!("magnet:?xt=urn:btih:{released:040}"), seeds: 10, size: 100, released, candidates: 1 }
    }

    fn inbox_app() -> TuiApp {
        let mut shows = Shows::default();
        shows.shows = vec![
            Show { imdb: "tt111".into(), title: "Alpha Show".into(), episodes: vec![episode("S01E01", 10), episode("S01", 5)], last_checked: shows::now(), catalogue_complete: true, ..Default::default() },
            Show { imdb: "tt222".into(), title: "Beta Show".into(), episodes: vec![episode("S01E01", 20)], last_checked: shows::now(), catalogue_complete: true, ..Default::default() },
        ];
        let mut app = TuiApp::new(Config::default(), Screen::Shows, shows);
        app.kitty = None;
        app
    }

    #[test]
    fn shows_inbox_aggregates_unread_episodes_and_seasons_newest_first() {
        let mut app = inbox_app();
        assert!(app.shows_inbox() && app.shows_focus_eps);
        assert_eq!(app.inbox_rows, vec![
            ShowReleaseTarget { imdb: "tt222".into(), key: "S01E01".into() },
            ShowReleaseTarget { imdb: "tt111".into(), key: "S01E01".into() },
            ShowReleaseTarget { imdb: "tt111".into(), key: "S01".into() },
        ]);
        app.shows.shows[0].seen.insert("S01E01".into());
        app.shows.shows[1].grabbed.insert("S01E01".into(), "prior download".into());
        app.rebuild_inbox();
        assert_eq!(app.inbox_rows.len(), 2);
        assert_eq!(app.selected_episode().unwrap().0.imdb, "tt222");
        let text = render(&app, 80, 24);
        assert!(text.contains("Alpha Show") && text.contains("Beta Show") && text.contains("S01E01") && text.contains("S01"), "{text}");
    }

    #[test]
    fn inbox_mark_and_explicit_undo_use_show_and_episode_identity() {
        let mut app = inbox_app();
        app.mark_selected_release();
        assert!(app.shows.shows[1].seen.contains("S01E01"));
        assert!(!app.shows.shows[0].seen.contains("S01E01"));
        assert_eq!(app.selected_episode().unwrap().0.imdb, "tt111");
        let text = render(&app, 80, 24);
        assert!(text.lines().nth(22).unwrap().contains("(U)ndo"));
        app.undo_inbox_read();
        assert_eq!(app.selected_episode().unwrap().0.imdb, "tt222");
        assert!(!app.shows.shows[1].seen.contains("S01E01"));
        assert!(app.inbox_last_read.is_none());
        app.on_shows_key(KeyEvent::new(KeyCode::Char('M')));
        assert!(app.status.contains("one show"));
        assert_eq!(app.shows.new_total(), 3);
        app.select_show_parent(2);
        app.ep_list.borrow_mut().select(Some(0));
        app.mark_selected_release();
        assert_eq!(app.ep_list.borrow().selected(), Some(0));
        app.mark_selected_release();
        assert!(!app.shows.shows[1].seen.contains("S01E01"));
    }

    #[test]
    fn selecting_a_followed_show_opens_its_releases_before_download() {
        let mut app = inbox_app();
        // Following resolves to the parent, using the same selector as pointer navigation.
        app.select_show_parent(2);
        assert!(!app.shows_focus_eps);
        assert!(app.selected_episode().is_none());
        assert!(matches!(app.on_shows_key(KeyEvent::new(KeyCode::Enter)), Cmd::None));
        assert!(app.shows_focus_eps);
        assert_eq!(app.selected_release_target(), Some(ShowReleaseTarget { imdb: "tt222".into(), key: "S01E01".into() }));
        assert!(matches!(app.on_shows_key(KeyEvent::new(KeyCode::Enter)), Cmd::Task(..)));
        assert!(app.show_grabbing.contains("tt222:S01E01"));
        assert_eq!(app.shows.new_total(), 3);
    }

    #[test]
    fn inbox_refresh_and_download_dispatch_preserve_the_stable_release_target() {
        let mut app = inbox_app();
        app.ep_list.borrow_mut().select(Some(1));
        let selected = app.selected_release_target().unwrap();
        app.shows.shows[1].merge_episodes(vec![episode("S01E02", 30)]);
        app.rebuild_inbox();
        assert_eq!(app.selected_release_target(), Some(selected.clone()));
        assert_eq!(app.ep_list.borrow().selected(), Some(2));
        assert!(matches!(app.on_shows_key(KeyEvent::new(KeyCode::Enter)), Cmd::Task(..)));
        assert!(app.show_grabbing.contains("tt111:S01E01"));
        assert!(!app.show_grabbing.contains("tt222:S01E01"));
        assert!(matches!(app.on_shows_key(KeyEvent::new(KeyCode::Enter)), Cmd::None));
        app.update(Msg::ShowGrab { imdb: "tt111".into(), episode: episode("S01E01", 10), result: Err("fixture add failed".into()) });
        assert_eq!(app.selected_release_target(), Some(selected));
        assert!(!app.shows.shows[0].seen.contains("S01E01"));
        assert_eq!(app.shows.new_total(), 4);
    }

    #[test]
    fn shows_badge_order_and_direct_keys_are_visible_on_other_screens() {
        let mut app = inbox_app();
        for screen in [Screen::Torrents, Screen::Search, Screen::Rss] {
            app.set_screen(screen);
            let text = render(&app, 80, 24);
            let header = text.lines().next().unwrap();
            assert!(header.contains("3 Shows (3)"), "{header}");
            assert!(header.find("Shows").unwrap() < header.find("4 RSS").unwrap());
        }
        app.set_screen(Screen::Torrents);
        app.on_key(KeyEvent::new(KeyCode::Char('3')));
        assert_eq!(app.screen, Screen::Shows);
        app.on_key(KeyEvent::new(KeyCode::Tab));
        assert_eq!(app.screen, Screen::Rss);
        app.on_key(KeyEvent::new(KeyCode::BackTab));
        assert_eq!(app.screen, Screen::Shows);
        app.on_key(KeyEvent::new(KeyCode::Char('4')));
        assert_eq!(app.screen, Screen::Rss);
    }

    #[test]
    fn inbox_mouse_geometry_uses_rendered_rows_after_summary_warnings_and_scroll() {
        use ftui::{MouseButton, MouseEvent, MouseEventKind};
        let mut app = inbox_app();
        app.shows.shows[0].error = Some("source unavailable".into());
        app.shows_refreshing.insert("tt222".into());
        app.shows.shows[0].episodes = (1..=30).map(|i| episode(&format!("S01E{i:02}"), i)).collect();
        app.rebuild_inbox();
        app.ep_list.borrow_mut().select(Some(20));
        let text = render(&app, 80, 24);
        assert!(text.contains("Checking 1 show(s)") && text.contains("source unavailable"), "{text}");
        let area = app.shows_rects.get()[1];
        let offset = app.ep_list.borrow().offset;
        assert!(offset > 0);
        assert!(app.mouse_list_at(area.x, area.y - 1).is_none());
        assert!(app.mouse_list_at(area.x, area.bottom()).is_none());
        let target = app.episode_at(offset + 1).map(|(s, e)| ShowReleaseTarget { imdb: s.imdb.clone(), key: e.key.clone() }).unwrap();
        let selected = app.selected_release_target();
        app.on_mouse(MouseEvent::new(MouseEventKind::Moved, area.x + 2, area.y + 1));
        assert_eq!(app.selected_release_target(), selected);
        assert_eq!(app.hovered_title().unwrap().1.as_deref(), Some(target.imdb.as_str()));
        app.on_mouse(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), area.x + 2, area.y + 1));
        assert_eq!(app.selected_release_target(), Some(target));
        app.shows.shows.iter_mut().for_each(|s| { s.mark_all_seen(); });
        app.rebuild_inbox();
        let text = render(&app, 80, 24);
        assert!(text.contains("Checking for releases") && text.contains("Checking 1 show(s)") && text.contains("source unavailable"), "{text}");
        app.shows_refreshing.clear();
        assert!(render(&app, 80, 24).contains("Source warnings remain below"));
        app.shows.shows[0].error = None;
        assert!(render(&app, 80, 24).contains("All caught up"));
    }

    #[test]
    fn selected_name_reveals_hidden_middle_and_tail() {
        let name = "abcdefghij";
        assert_eq!(readable_cell(name, 5, None), "ab…ij");
        assert_eq!(readable_cell(name, 5, Some(Duration::from_millis(800))), "ab…ij");
        assert_eq!(readable_cell(name, 5, Some(Duration::from_millis(900 + 3 * 160))), "defgh");
        assert_eq!(readable_cell(name, 5, Some(Duration::from_millis(900 + 7 * 160))), "hij  ");
        for step in 0..16 {
            assert_eq!(dw(&readable_cell("字幕ファイル.mkv", 8, Some(Duration::from_millis(900 + step * 160)))), 8);
        }
    }

    #[test]
    fn mouse_row_uses_visible_scroll_offset() {
        let area = Rect::new(4, 6, 20, 5);
        assert_eq!(mouse_row_index(area, 5, 8, 12, 30), Some(14));
        assert_eq!(mouse_row_index(area, 5, 10, 12, 15), None);
        assert_eq!(mouse_row_index(area, 3, 8, 12, 30), None);
    }

    #[test]
    fn torrent_names_and_mouse_rows_remain_usable_across_details_cutoff() {
        use ftui::{MouseButton, MouseEvent, MouseEventKind};
        for width in [80, 86, 99, 100, 120, 139, 140, 160] {
            let mut app = app();
            app.online = true;
            app.torrents = ["Short.txt", "Episode.S01E02.1080p.mkv"].into_iter().enumerate()
                .map(|(i, name)| Torrent { hash: format!("fixture-{i}"), name: name.into(), state: "stoppedDL".into(), progress_known: true, up_limit: 32768, ..Default::default() }).collect();
            app.tor_list.borrow_mut().select(Some(0));
            let text = render(&app, width, 24);
            assert_eq!(text.contains(" Details "), width >= 140, "width {width}: {text}");
            assert!(text.contains("Short.txt"), "width {width}: {text}");
            let (y, row) = text.lines().enumerate().find(|(_, row)| row.contains("Episode.S01E02.1080p.mkv"))
                .unwrap_or_else(|| panic!("filename truncated at width {width}: {text}"));
            let x = row.chars().position(|ch| ch == 'E').unwrap() as u16;
            assert_eq!(app.mouse_list_at(x, y as u16).unwrap().0, MouseList::Torrents);
            assert_eq!(app.mouse_list_at(width - 3, y as u16).is_some(), width < 140,
                "mouse region disagrees with rendered list at width {width}");
            assert!(matches!(app.on_mouse(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), x, y as u16)), Cmd::None));
            assert_eq!(app.tor_list.borrow().selected(), Some(1));
        }
        let mut app = app();
        app.screen = Screen::Search;
        for width in [99, 100, 120, 139, 140, 160] {
            assert_eq!(render(&app, width, 24).contains(" Details "), width >= 100);
        }
    }

    #[test]
    fn clicking_visible_tab_titles_selects_the_same_screen() {
        use ftui::{MouseButton, MouseEvent, MouseEventKind};
        for width in [80, 120] {
            for (label, target) in [("Torrents", Screen::Torrents), ("Search", Screen::Search), ("RSS", Screen::Rss), ("Shows", Screen::Shows)] {
                let fresh = app();
                let rendered = render(&fresh, width, 24);
                let header = rendered.lines().next().unwrap();
                let start = header.find(label).unwrap();
                for offset in 0..label.len() {
                    let mut candidate = app();
                    render(&candidate, width, 24);
                    candidate.on_mouse(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), (start + offset) as u16, 0));
                    assert_eq!(candidate.screen, target, "{label} cell {offset} at width {width}");
                }
            }
        }
    }

    #[test]
    fn rss_mouse_hover_keeps_target_and_click_selects_without_reading_or_adding() {
        use ftui::{MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.screen = Screen::Rss;
        app.feeds = vec![feed(vec![article("one", "First item"), article("two", "Second item")]), RssFeed { name: "Another feed".into(), ..Default::default() }];
        app.feed_list.borrow_mut().select(Some(0));
        app.item_list.borrow_mut().select(Some(0));
        let text = render(&app, 120, 30);
        let (y, row) = text.lines().enumerate().find(|(_, row)| row.contains("Second item")).unwrap();
        let x = row.chars().position(|ch| ch == 'S').unwrap() as u16;
        app.on_mouse(MouseEvent::new(MouseEventKind::Moved, x, y as u16));
        assert!(!app.rss_focus_items);
        assert_eq!(app.item_list.borrow().selected(), Some(0));
        assert_eq!(app.item_list.borrow().hovered, Some(1));
        assert!(!app.rss_busy);
        assert!(matches!(app.on_mouse(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), x, y as u16)), Cmd::None));
        assert!(app.rss_focus_items);
        assert_eq!(app.item_list.borrow().selected(), Some(1));
        assert!(!app.rss_busy);
        assert!(app.feeds[0].articles.iter().all(|a| !a.is_read));
        assert!(matches!(app.on_mouse(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), x, y as u16)), Cmd::Task(..)));
        assert!(app.rss_busy);
    }

    #[test]
    fn refreshed_rss_row_with_a_different_id_requires_its_own_double_click() {
        use ftui::{MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.screen = Screen::Rss;
        app.feeds = vec![feed(vec![article("original", "Original item")])];
        app.feed_list.borrow_mut().select(Some(0));
        render(&app, 120, 30);
        let (_, area) = app.mouse_list_at(60, 3).unwrap();
        let click = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), area.x + 2, area.y);
        assert!(matches!(app.on_mouse(click), Cmd::None));
        let first_click = app.last_click.clone().unwrap();
        app.update(Msg::Rss { revision: 0, result: Ok(vec![feed(vec![article("replacement", "Replacement item"), article("original", "Original item")])]) });
        render(&app, 120, 30);
        // Force the first timestamp into the double-click window, independently of test scheduling.
        app.last_click = Some((first_click.0, first_click.1, std::time::Instant::now()));
        assert!(matches!(app.on_mouse(click), Cmd::None));
        assert!(!app.rss_busy);
        assert_eq!(app.item_list.borrow().selected(), Some(0));
        assert_eq!(app.feeds[0].articles[0].id, "replacement");
        assert!(app.feeds[0].articles.iter().all(|item| !item.is_read));
        assert!(matches!(app.on_mouse(click), Cmd::Task(..)));
        assert!(app.rss_busy);
    }

    #[test]
    fn parent_pane_read_key_never_marks_a_whole_feed_or_show() {
        let mut app = app();
        app.screen = Screen::Rss;
        app.feeds = vec![feed(vec![article("one", "Unread item")])];
        app.feed_list.borrow_mut().select(Some(0));
        assert!(matches!(app.on_rss_key(KeyEvent::new(KeyCode::Char('m'))), Cmd::None));
        assert!(app.status.contains("Shift+M"));
        assert!(!app.rss_busy);
        app.on_rss_key(KeyEvent::new(KeyCode::Enter));
        assert!(app.rss_focus_items);
        assert_eq!(app.item_list.borrow().selected(), Some(0));
        app.screen = Screen::Shows;
        app.shows.shows.push(Show { title: "Tracked Show".into(), ..Default::default() });
        app.show_list.borrow_mut().select(Some(1));
        app.shows_focus_eps = false;
        assert!(matches!(app.on_shows_key(KeyEvent::new(KeyCode::Char('m'))), Cmd::None));
        assert!(app.status.contains("Shift+M"));
        assert!(app.shows.shows[0].seen.is_empty());
    }

    #[test]
    fn read_actions_and_help_remain_visible_while_feedback_changes() {
        let mut app = app();
        app.screen = Screen::Rss;
        app.feeds = vec![feed(vec![article("one", "One item")])];
        app.feed_list.borrow_mut().select(Some(0));
        app.item_list.borrow_mut().select(Some(0));
        app.rss_focus_items = true;
        let text = render(&app, 80, 24);
        let action_row = text.lines().nth(22).unwrap();
        assert!(action_row.contains("(M)ark read"), "{action_row}");
        assert!(action_row.contains("(Shift+M) All read"), "{action_row}");
        assert!(action_row.contains("(?) Help"), "{action_row}");
        app.set_status("A long action result that explains what happened without covering any command in the row above", false);
        app.feeds[0].articles[0].is_read = true;
        let text = render(&app, 80, 24);
        let action_row = text.lines().nth(22).unwrap();
        assert!(action_row.contains("(M)ark unread"), "{action_row}");
        assert!(action_row.contains("(Shift+M) All read"), "{action_row}");
        assert!(action_row.contains("(?) Help"), "{action_row}");
        assert!(text.lines().nth(23).unwrap().contains("A long action result"));
    }

    #[test]
    fn settings_controls_and_small_terminal_keep_meaningful_labels() {
        let mut app = app();
        let mut list = ListState::default();
        list.select(Some(0));
        app.settings = Some(SettingsModal { cfg: Config::default(), list: RefCell::new(list), connections: vec![Some("proton".into()), None], error: String::new() });
        app.on_key(KeyEvent::new(KeyCode::Left));
        assert_eq!(app.settings.as_ref().unwrap().cfg.max_active_downloads, 2);
        app.on_key(KeyEvent::new(KeyCode::Down));
        app.on_key(KeyEvent::new(KeyCode::Right));
        assert_eq!(app.settings.as_ref().unwrap().cfg.peer_encryption, "require");
        app.on_key(KeyEvent::new(KeyCode::Down));
        app.on_key(KeyEvent::new(KeyCode::Right));
        assert_eq!(app.settings.as_ref().unwrap().cfg.network_interface.as_deref(), Some("proton"));
        let text = render(&app, 80, 24);
        assert!(text.contains("Download settings"));
        assert!(text.contains("ProtonVPN · required"));
        assert!(text.contains("Require encryption"));
        render(&app, 24, 8);
        render(&app, 8, 3);
        app.on_key(KeyEvent::new(KeyCode::Escape));
        assert!(app.settings.is_none());
    }

    #[test]
    fn queued_and_vpn_waiting_rows_do_not_claim_running_downloads() {
        let mut app = app();
        app.torrents = vec![Torrent { name: "Next episode".into(), state: "queuedDL".into(), progress_known: true, ..Default::default() }, Torrent { name: "Saved episode".into(), state: "waitingVPN".into(), progress_known: false, ..Default::default() }];
        let text = render(&app, 120, 30);
        assert!(text.contains("Queued"));
        assert!(text.contains("Wait VPN"));
        assert!(!app.torrents[0].is_active());
        assert!(!app.torrents[1].is_active());
    }

    #[test]
    fn rss_refresh_preserves_article_and_rejects_stale_results() {
        let mut app = app();
        app.feeds = vec![feed(vec![article("old", "Old item"), article("selected", "Selected item")])];
        app.feed_list.borrow_mut().select(Some(0));
        app.item_list.borrow_mut().select(Some(1));
        app.rss_revision = 7;
        app.update(Msg::Rss { revision: 7, result: Ok(vec![feed(vec![article("new", "New item"), article("old", "Old item"), article("selected", "Selected item")])]) });
        assert_eq!(app.item_list.borrow().selected(), Some(2));
        app.update(Msg::Rss { revision: 6, result: Ok(vec![]) });
        assert_eq!(app.feeds.len(), 1);
        assert_eq!(app.item_list.borrow().selected(), Some(2));
    }

    #[test]
    fn completed_search_keeps_partial_results_and_provider_warning() {
        let mut app = app();
        app.search_sub = Some((42, "fixture".into()));
        app.search_running = true;
        app.update(Msg::SearchBatch {
            id: 42,
            results: vec![SearchResult { file_name: "Partial result".into(), file_url: "magnet:?xt=urn:btih:abc".into(), ..Default::default() }],
            done: true,
            error: Some("one provider failed".into()),
        });
        assert_eq!(app.search_results.len(), 1);
        assert!(!app.search_running);
        assert!(app.status_is_error);
        assert!(app.status.contains("1 results"));
        assert!(app.status.contains("one provider failed"));
    }

    #[test]
    fn rss_initial_check_and_download_dispatch_only_once() {
        let mut app = app();
        app.rss_initial_refresh_pending = true;
        assert!(matches!(app.update(Msg::Rss { revision: 0, result: Ok(vec![feed(vec![article("item", "FeedItemAlpha")])]) }), Cmd::Task(..)));
        assert!(app.rss_busy);
        assert!(!app.rss_initial_refresh_pending);
        assert!(matches!(app.refresh_rss(), Cmd::None));
        let revision = app.rss_revision;
        app.update(Msg::RssAction { revision, label: "checked".into(), result: Ok(app.feeds.clone()), error: None });
        assert!(!app.rss_busy);
        app.on_rss_key(KeyEvent::new(KeyCode::Right));
        assert!(!app.feeds[0].articles[0].is_read);
        assert!(matches!(app.on_rss_key(KeyEvent::new(KeyCode::Enter)), Cmd::Task(..)));
        assert!(matches!(app.on_rss_key(KeyEvent::new(KeyCode::Enter)), Cmd::None));
        assert!(!app.feeds[0].articles[0].is_read);
    }

    #[test]
    fn native_controls_do_not_offer_unsupported_priorities_or_toggles() {
        let mut app = app();
        app.files = Some(FilesModal { hash: "hash".into(), name: "Torrent".into(), files: vec![TorrentFile { index: 0, name: "File".into(), priority: 1, ..Default::default() }], list: RefCell::new(ListState::default()), loading: false, error: String::new() });
        app.files.as_ref().unwrap().list.borrow_mut().select(Some(0));
        assert!(matches!(app.on_files_key(KeyEvent::new(KeyCode::Char('2'))), Cmd::None));
        assert!(matches!(app.on_files_key(KeyEvent::new(KeyCode::Char('3'))), Cmd::None));
        app.files = None;
        app.open_prompt(PromptKind::AddMagnet, "Add", false);
        let before = app.prompt.as_ref().unwrap().add_flags;
        app.on_prompt_key(KeyEvent::new(KeyCode::F(2)));
        app.on_prompt_key(KeyEvent::new(KeyCode::F(3)));
        assert_eq!(app.prompt.as_ref().unwrap().add_flags, before);
        assert!(app.prompt.as_ref().unwrap().title().contains("streaming automatic"));
        assert!(!app.help_text().contains("normal/high/max"));
    }

    #[test]
    fn screens_and_scrollable_help_keep_names_visible_at_supported_sizes() {
        let mut app = app();
        app.torrents = vec![Torrent { name: "TorrentAlpha.1080p.mkv".into(), hash: "hash".into(), ..Default::default() }];
        app.tor_list.borrow_mut().select(Some(0));
        app.search_results = vec![SearchResult { file_name: "SearchAlpha.1080p.mkv".into(), ..Default::default() }];
        app.search_focus_input = false;
        app.search_list.borrow_mut().select(Some(0));
        app.feeds = vec![feed(vec![article("item", "FeedItemAlpha.1080p.mkv")])];
        app.feed_list.borrow_mut().select(Some(0));
        app.shows.shows.push(Show { title: "Tracked Show".into(), imdb: "tt12345".into(), episodes: vec![Episode { key: "S01".into(), season: 1, episode: 0, title: "ReleaseAlpha.S01.1080p".into(), quality: "1080p".into(), magnet: "magnet:".into(), seeds: 10, size: 10, released: 0, candidates: 1 }], ..Default::default() });
        app.show_list.borrow_mut().select(Some(1));
        for (width, height) in [(80, 24), (120, 30), (160, 44)] {
            for (screen, expected) in [(Screen::Torrents, "TorrentAlpha"), (Screen::Search, "SearchAlpha"), (Screen::Rss, "FeedItemAlpha"), (Screen::Shows, "ReleaseAlpha")] {
                app.screen = screen;
                let text = render(&app, width, height);
                assert!(text.contains(expected), "{screen:?} {width}x{height}: {text}");
            }
            app.show_help = true;
            app.help_scroll = 0;
            assert!(render(&app, width, height).contains("GLOBAL"));
            app.on_key(KeyEvent::new(KeyCode::End));
            let text = render(&app, width, height);
            assert!(text.contains("Defaults:"), "help end {width}x{height}: {text}");
            assert!(text.contains("Esc or click close"));
            app.show_help = false;
        }
        app.screen = Screen::Shows;
        render(&app, 80, 24);
        let area = app.shows_rects.get()[1];
        assert_eq!(app.mouse_list_at(area.x, area.y).unwrap().0, MouseList::Episodes);
        app.ep_list.borrow_mut().offset = 10;
        assert_eq!(mouse_row_index(area, area.x, area.y + 2, app.ep_list.borrow().offset, 30), Some(12));
    }
}
