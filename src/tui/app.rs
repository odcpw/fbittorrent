//! Application state and construction.
use super::*;

impl From<Event> for Msg {
    fn from(e: Event) -> Self { Msg::Raw(e) }
}

impl Screen {
    pub(super) fn index(self) -> usize { match self { Screen::Torrents => 0, Screen::Search => 1, Screen::Shows => 2, Screen::Rss => 3 } }
    pub(super) fn from_index(i: usize) -> Self { match i % 4 { 0 => Screen::Torrents, 1 => Screen::Search, 2 => Screen::Shows, _ => Screen::Rss } }
}

pub struct TuiApp {
    pub(super) cfg: Config,
    pub(super) store: super::store::Store,
    pub(super) presence: Option<Arc<super::attach::Presence>>,
    pub(super) recovery: super::attach::Recovery,
    pub(super) failed_polls: u8,
    pub(super) daemon_stopped: bool,
    pub(super) quit_notice: bool,
    pub(super) theme: Theme,
    pub(super) footer_cache: RefCell<Option<super::footer::FooterCache>>,
    pub(super) screen: Screen,
    pub(super) removal_undo: Option<(String,u64)>,
    pub(super) release_sort: release_sort::ReleaseSort,
    pub(super) release_descending: bool,
    pub(super) tabs: RefCell<TabsState>,
    pub(super) show_help: bool,
    pub(super) help_scroll: u16,
    pub(super) online: bool,
    pub(super) backend_error: String,
    pub(super) activity_text: Option<String>,
    pub(super) torrents: Vec<Torrent>,
    pub(super) transfer: TransferInfo,
    pub(super) tor_list: RefCell<ListState>,
    pub(super) rects: RefCell<super::layout::Rects>,
    pub(super) torrent_table: RefCell<super::layout::TorrentTable>,
    pub(super) list_tables: RefCell<[super::layout::ListTable;10]>,
    pub(super) torrent_revision: u64,
    pub(super) torrent_sort: super::store::TorrentSort,
    pub(super) torrent_descending: bool,
    pub(super) poll_inflight: bool,
    pub(super) tick: usize,
    // search
    pub(super) search_input: TextInput,
    pub(super) search_focus_input: bool,
    pub(super) search_results: Vec<SearchResult>,
    pub(super) search_list: RefCell<ListState>,
    pub(super) search_sub: Option<(SubId, String)>,
    pub(super) search_running: bool,
    pub(super) native_search: Option<super::remote::search::Session>,
    pub(super) search_wire: Vec<crate::api::SearchItem>,
    pub(super) search_seq: u64,
    pub(super) search_browse: bool,
    pub(super) poster_open: bool,
    pub(super) search_sort: super::store::SearchSort,
    pub(super) search_descending: bool,
    pub(super) browse_entries: Vec<browse::Entry>,
    pub(super) browse_list: RefCell<ListState>,
    pub(super) browse_loading: Option<String>,
    pub(super) files: Option<FilesModal>,
    pub(super) details: Option<DetailsModal>,
    pub(super) providers: Option<ProvidersModal>,
    pub(super) settings: Option<SettingsModal>,
    pub(super) plugins: Vec<SearchPlugin>,
    pub(super) ratings: Option<Arc<imdb::RatingsIndex>>,
    pub(super) name_ids: HashMap<String, String>,
    pub(super) status_at: std::time::Instant,
    pub(super) theme_mtime: Option<std::time::SystemTime>,
    pub(super) hover_key: Option<String>,
    pub(super) hover_since: std::time::Instant,
    pub(super) pointer_motion_on_start: bool,
    pub(super) termination_signal: Option<i32>,
    pub(super) marquee_key: Option<super::motion::MarqueeTarget>,
    pub(super) marquee_since: std::time::Instant,
    pub(super) last_bounds: Cell<Rect>,
    pub(super) tab_rects: Cell<[Rect; 4]>,
    pub(super) last_click: Option<(MouseList, String, std::time::Instant)>,
    pub(super) kitty: Option<RefCell<KittyPlacer>>,
    /// Set by view() when a poster was drawn this frame; used to clear stale placements.
    pub(super) poster_drawn: std::cell::Cell<bool>,
    // rss
    pub(super) feeds: Vec<RssFeed>,
    pub(super) feed_list: RefCell<ListState>,
    pub(super) item_list: RefCell<ListState>,
    pub(super) rss_focus_items: bool,
    pub(super) rss_loaded: bool,
    pub(super) rss_revision: u64,
    pub(super) rss_busy: bool,
    pub(super) rss_loading: bool,
    pub(super) rss_retry_until: Option<std::time::Instant>,
    pub(super) show_grabbing: HashSet<String>,
    // shows
    pub(super) shows: Shows,
    pub(super) show_list: RefCell<ListState>,
    pub(super) ep_list: RefCell<ListState>,
    pub(super) shows_focus_eps: bool,
    pub(super) shows_refreshing: HashSet<String>,
    pub(super) inbox_rows: Vec<ShowReleaseTarget>,
    /// Explicit undo restores the exact release last marked read in the inbox.
    pub(super) inbox_last_read: Option<ShowReleaseTarget>,
    pub(super) shows_rects: Cell<[Rect; 2]>,
    // imdb
    pub(super) imdb: HashMap<String, ImdbState>,
    pub(super) imdb_aliases: HashMap<String,String>,
    pub(super) poster_target: Option<(String,Option<String>)>,
    pub(super) graphics_suppressed: Cell<bool>,
    pub(super) prompt: Option<Prompt>,
    pub(super) status: String,
    pub(super) status_is_error: bool,
    pub(super) status_success: bool,
    pub(super) feedback:super::feedback::Feedback,
    pub(super) clipboard:super::clipboard::ClipboardState,
    pub(super) log_open:bool,
    pub(super) alt_busy:bool,
    pub(super) service_busy:bool,
    pub(super) settings_loading:bool,
    pub(super) follow_target:Option<String>,
    pub(super) topic_selection:super::remote::topics::Selection,
}

/// A row identity survives source reordering and the same episode key in another show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShowReleaseTarget { pub(super) imdb: String, pub(super) key: String }

pub enum Msg {
    Raw(Event),
    Poll,
    Api {connection:u64,update:super::attach::ApiUpdate,ack:std::sync::mpsc::SyncSender<()>},
    ApiBatch {connection:u64,updates:Vec<super::attach::ApiUpdate>,ack:std::sync::mpsc::SyncSender<()>},
    ApiRead {connection:Option<u64>,result:Result<super::remote::ReadSnapshot,String>},
    Attached(Result<Arc<super::attach::Presence>,super::attach::Failure>),
    OwnerShutdown(Result<(),String>),
    SettingsSaved(Result<Config,String>),
    SettingsOpened{connection:u64,result:Result<crate::api::Settings,String>},
    Torrents(Result<(Vec<Torrent>, TransferInfo), String>),
    ActionDone(Result<String, String>),
    AltDone(Result<String,String>),
    UiSubscribed{connection:u64,result:Result<(),String>},
    Imdb(String, Result<Option<(Title, Option<Poster>)>, String>),
    NativeSearchStarted {connection:u64,sequence:u64,result:Result<super::remote::search::Session,String>},
    SearchDisposed,
    NativeBrowse {connection:Option<u64>,sequence:u64,label:String,result:Result<Option<Vec<crate::api::SearchItem>>,String>},
    SearchBatch { id: SubId, results: Vec<SearchResult>, done: bool, error: Option<String> },
    Rss { revision: u64, result: Result<Vec<RssFeed>, String> },
    RssAction { revision: u64, label: String, result: Result<Vec<RssFeed>, String>, error: Option<String> },
    TopicsSelected{connection:u64,active:Vec<(u64,crate::api::Topic)>,result:Result<(),String>},
    ServiceDone{connection:Option<u64>,op:&'static str,args:serde_json::Value,label:String,result:Result<serde_json::Value,String>},
    Browse(String, Result<Vec<SearchResult>, String>),
    /// Fast timer: look the highlighted title up once the cursor has rested on it.
    Hover,
    Files(String, Result<Vec<TorrentFile>, String>),
    Trackers {connection:Option<u64>,hash:String,request:u64,forced:bool,result:Result<Vec<crate::api::Tracker>,String>},
    ApiFiles {connection:Option<u64>,hash:String,result:Result<Vec<TorrentFile>,String>},
    Plugins(Result<Vec<SearchPlugin>, String>),

}

pub enum ImdbState {
    Loading,
    Missing,
    Ready(Box<Title>, Option<Poster>),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen { Torrents, Search, Shows, Rss }

impl TuiApp {
    pub(super) fn new(cfg: Config, screen: Screen, shows: Shows) -> Self {
        let theme = Theme::load();
        let mut search_input = TextInput::new()
            .with_placeholder("search torrents, Enter to run")
            .with_style(theme.text())
            .with_cursor_style(Style::new().bg(theme.accent).fg(theme.selection));
        search_input.set_focused(screen == Screen::Search);
        let mut app = TuiApp {
            cfg,
            store:super::store::Store::default(),
            presence:None,recovery:super::attach::Recovery::default(),failed_polls:0,daemon_stopped:false,quit_notice:false,
            theme,
            footer_cache: RefCell::new(None),
            screen,
            removal_undo: None,
            release_sort: release_sort::ReleaseSort::Date,
            release_descending: true,
            tabs: RefCell::new(TabsState { active: screen.index(), offset: 0 }),
            show_help: false,
            help_scroll: 0,
            online: true,
            backend_error: String::new(),
            activity_text: None,
            torrents: vec![],
            transfer: TransferInfo::default(),
            tor_list: RefCell::new(ListState::default()),
            rects:RefCell::default(),
            torrent_table:RefCell::default(),
            list_tables:RefCell::new(std::array::from_fn(|_|Default::default())),
            torrent_revision:0,
            torrent_sort:super::store::TorrentSort::Added,
            torrent_descending:true,
            poll_inflight: false,
            tick: 0,
            search_input,
            search_focus_input: screen == Screen::Search,
            search_results: vec![],
            search_list: RefCell::new(ListState::default()),
            search_sub: None,
            search_running: false,
            native_search: None,
            search_wire: vec![],
            search_seq: 0,
            search_browse: false,
            poster_open: false,
            search_sort:super::store::SearchSort::Seeds,
            search_descending:true,
            browse_entries: browse::catalogue(),
            browse_list: RefCell::new(ListState::default()),
            browse_loading: None,
            files: None,
            details: None,
            providers: None,
            settings: None,
            plugins: vec![],
            ratings: None,
            name_ids: HashMap::new(),
            status_at: std::time::Instant::now(),
            theme_mtime: std::fs::metadata(theme::colors_path()).and_then(|m| m.modified()).ok(),
            hover_key: None,
            hover_since: std::time::Instant::now(),
            pointer_motion_on_start: false,
            termination_signal: None,
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
            imdb_aliases: HashMap::new(),
            poster_target: None,
            graphics_suppressed: Cell::new(false),
            prompt: None,
            status: String::new(),
            status_is_error: false,
            status_success:false,feedback:Default::default(),clipboard:Default::default(),log_open:false,alt_busy:false,service_busy:false,settings_loading:false,follow_target:None,topic_selection:Default::default(),
        };
        app.show_list.borrow_mut().select(Some(0));
        app.rebuild_inbox();
        app
    }

    pub(super) fn set_success(&mut self,message:impl Into<String>){self.set_status(message,false);self.status_success=true;let hash=self.selected_torrent().and_then(|t|crate::api::Hash::new(t.hash.clone()).ok());self.feedback.push(&self.status,false,hash);}

    pub(super) fn set_status(&mut self, s: impl Into<String>, err: bool) {
        self.status = s.into();
        self.status_is_error = err;
        if err {let hash=self.selected_torrent().and_then(|t|crate::api::Hash::new(t.hash.clone()).ok());self.feedback.push(&self.status,true,hash);}
        self.status_success=false;
        self.status_at = std::time::Instant::now();
    }
}


pub(super) const SPINNER: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];
