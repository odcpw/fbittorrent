//! Local render state only: never init/update the model or start subscriptions.
use super::*;

pub(super) const SURFACES: [&str; 11] = [
    "torrents", "search", "shows", "rss", "files", "providers", "settings", "prompt", "help", "confirm", "done",
];

pub(super) fn app(surface: &str) -> TuiApp {
    let mut shows = Shows::default();
    shows.shows = [
        ("tt17491088", "The Diplomat", "S04E01", 4, 1),
        ("tt0903747", "Breaking Bad", "S05E16", 5, 16),
    ].into_iter().map(|(imdb, title, key, season, episode)| Show {
        imdb: imdb.into(), title: title.into(), catalogue_complete: true, last_checked: 1_792_281_600,
        episodes: vec![Episode {
            key: key.into(), season, episode, title: format!("{title}.{key}.1080p.WEB-DL"),
            magnet: format!("magnet:?xt=urn:btih:{episode:040x}"), quality: "1080p".into(),
            seeds: 42, size: 805_306_368, released: 1_792_281_600, candidates: 2,
        }], ..Default::default()
    }).collect();
    let mut app = TuiApp::new(Config::default(), Screen::Torrents, shows);
    // Theme names appear in the header; neither host theme nor terminal graphics
    // should affect the text baseline. No poster placement may reach the terminal.
    app.theme = Theme::default();
    app.kitty = None;
    app.rss_initial_refresh_pending = false;
    app.rss_loaded = true;
    app.search_focus_input = false;
    app.search_input.set_value("Sintel");
    app.search_input.set_focused(false);
    app.torrents = [
        ("The Diplomat S04E01 1080p WEB-DL H264-GROUP", "downloading", 0.42),
        ("Sintel.2010.1080p", "stoppedUP", 1.0),
        ("Other.Movie.2026.1080p", "metaDL", 0.0),
    ].into_iter().enumerate().map(|(i, (name, state, progress))| Torrent {
        hash: format!("{:040x}", i + 1), name: name.into(), state: state.into(), progress,
        progress_known: i != 2, size: if i == 2 { 0 } else { 1_073_741_824 },
        completed: (progress * 1_073_741_824.0) as i64,
        dlspeed: if i == 0 { 2_097_152 } else { 0 }, upspeed: if i == 0 { 1024 } else { 0 },
        eta: if i == 0 { 297 } else { 0 }, num_seeds: 0, num_leechs: 3,
        seq_dl: true, f_l_piece_prio: true,
        save_path: "/fixture/downloads".into(), up_limit: crate::qbt::NO_UPLOAD_LIMIT,
        completion_on: if i == 1 { 1_792_281_600 } else { 0 }, ratio_limit: 0.0,
        // Render immediately after construction, keeping the relative label "just now".
        added_on: crate::shows::now(), ..Default::default()
    }).collect();
    app.transfer = TransferInfo { dl_info_speed: 2_097_152, up_info_speed: 1024, ..Default::default() };
    app.search_results = [
        "Sintel", "Sintel - 2010 - Multisubs mkv", "Sintel.2010.1080p.WEB.H264-CLASSiCALHD",
        "Sintel (2010) [720p] [x264] [bluray] [YTS]", "Sintel 2010 x264 DMRip 1080p mkv",
        "Sintel 2010 820p H264", "Sintel (2010) BR2Mini-DVD DD5 1 NL Subs",
        "Sintel.2010.720p.WEB.H264-CLASSiCALHD", "Sintel [辛特尔] Cafe\u{301}",
        "[philosophy-raws][è¾›ç‰¹å°”][sintel][BDRIP][10bitåŽ‹åˆ¶][2Kç..",
    ].into_iter().enumerate().map(|(i, name)| SearchResult {
        file_name: name.into(), file_url: format!("magnet:?xt=urn:btih:{:040x}", i + 100),
        file_size: 536_870_912, seeders: 124 - i as i64, leechers: 3,
        engine: "fixture".into(), pub_date: 1_792_281_600, ..Default::default()
    }).collect();
    app.feeds = vec![RssFeed {
        name: "Blender releases".into(), title: "Blender releases".into(),
        url: "http://fixture.invalid/rss".into(),
        articles: ["Sintel.2010.1080p", "Big.Buck.Bunny.2008.1080p", "Elephants.Dream.2006.1080p"]
            .into_iter().enumerate().map(|(i, title)| crate::qbt::RssArticle {
                id: format!("item-{i}"), title: title.into(), is_read: i == 2,
                size: Some(536_870_912), ts: 1_792_281_600,
                torrent_url: format!("magnet:?xt=urn:btih:{:040x}", i + 200), ..Default::default()
            }).collect(), ..Default::default()
    }];
    app.plugins = ["EZTV", "YTS", "The Pirate Bay"].into_iter().enumerate().map(|(i, name)| SearchPlugin {
        name: name.to_lowercase(), full_name: name.into(), enabled: i != 1,
        url: format!("https://provider-{i}.invalid"), ..Default::default()
    }).collect();
    for list in [&app.tor_list, &app.search_list, &app.feed_list, &app.item_list, &app.ep_list] {
        list.borrow_mut().select(Some(0));
    }
    match surface {
        "torrents" => {},
        "disk"=>crate::tui::disk_tests::prepare(&mut app),
        "details" => {let t=app.torrents[0].clone(); let _=app.open_details(&t);},
        "poster" => {let key=app.torrents[0].name.clone();crate::tui::poster_tests::ready(&mut app,&key);let _=app.on_key(KeyEvent::new(KeyCode::Char('v')));},
        "done" => app.tor_list.borrow_mut().select(Some(1)),
        "search" => { let _ = app.set_screen(Screen::Search); },
        "shows" => { let _ = app.set_screen(Screen::Shows); },
        "rss" => { let _ = app.set_screen(Screen::Rss); app.rss_focus_items = true; },
        "files" => app.files = Some(FilesModal {
            hash: app.torrents[0].hash.clone(), name: app.torrents[0].name.clone(),
            files: ["The.Diplomat.S04E01.mkv", "Subtitles/English.srt", "sample.mkv"]
                .into_iter().enumerate().map(|(i, name)| TorrentFile {
                    index: i as i64, name: name.into(), size: 268_435_456,
                    progress: 0.42, progress_known: true, priority: if i == 2 { 0 } else { 1 },
                }).collect(),
            list: selected_list(), loading: false, error: String::new(),
        }),
        "providers" => {
            let _ = app.set_screen(Screen::Search);
            app.providers = Some(ProvidersModal {
                plugins: app.plugins.clone(), list: selected_list(), loading: false, error: String::new(),
            });
        },
        "settings" => app.settings = Some(SettingsModal {
            original:app.cfg.clone(),cfg: app.cfg.clone(), list: selected_list(), saving:false, connections: vec![Some("proton".into()), None],
            error: String::new(),
        }),
        "prompt" => {
            app.open_prompt(PromptKind::AddMagnet, " Add magnet link or URL ", false);
            app.prompt.as_mut().unwrap().input.set_value("magnet:?xt=urn:btih:0000000000000000000000000000000000000001");
        },
        "help" => app.show_help = true,
        "confirm" => { let _ = app.on_torrents_key(KeyEvent::new(KeyCode::Char('D'))); },
        _ => panic!("unknown render fixture: {surface}"),
    }
    app
}

fn selected_list() -> RefCell<ListState> {
    let mut state = ListState::default();
    state.select(Some(0));
    RefCell::new(state)
}
