
    use super::*;

    fn destructive_keys(open: fn(&mut TuiApp), unfollow: bool) {
        for code in [KeyCode::Enter, KeyCode::Escape, KeyCode::Char('n'), KeyCode::Char(' '), KeyCode::Char('q'), KeyCode::Char('y'), KeyCode::Char('Y'), KeyCode::Char('Q'), KeyCode::Char('?'), KeyCode::Char('1'), KeyCode::Tab] {
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
            let yes = matches!(code, KeyCode::Char('y'));
            assert!(app.presence.is_none(), "confirmation key {code:?} must not open an owner connection");
            if unfollow {
                assert_eq!(matches!(command, Cmd::Task(..)),yes,"only y dispatches the daemon intent");
                assert_eq!(app.shows.shows.len(),2,"unfollow acknowledgement cannot remove a confirmed row");
                assert!(!app.status.starts_with("unfollowed"),"success waits for the daemon");
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
        // Keep this destructive-key control in a child with
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
            app
    }

    #[test]
    fn client_poll_and_detach_do_not_consume_an_owner_request() {
        use crate::daemon::lifecycle::{Lifecycle, Request as LifecycleRequest, RestartReason};
        let lifecycle = Lifecycle::default();
        let mut app = app();
        lifecycle.request_restart(RestartReason::Settings);
        assert!(matches!(app.update(Msg::Poll), Cmd::Task(..)));
        assert_eq!(lifecycle.pending_request(), Some(LifecycleRequest::Restart(RestartReason::Settings)));
        // Detaching a client cannot cancel or replace the daemon owner's restart.
        assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('q'))), Cmd::Quit));
        assert_eq!(lifecycle.wait(Duration::ZERO), Some(LifecycleRequest::Restart(RestartReason::Settings)));
        assert_eq!(lifecycle.pending_request(), None);
    }

    #[test]
    fn hover_acknowledges_termination_before_quitting() {
        ftui::core::shutdown_signal::with_test_signal_serialization(|| {
            for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
                let mut app = app();
                ftui::core::shutdown_signal::record_pending_termination_signal(signal);
                assert!(matches!(app.update(Msg::Hover), Cmd::Quit));
                assert_eq!(ftui::core::shutdown_signal::pending_termination_signal(), None);
                assert_eq!(app.termination_signal, Some(signal));
            }
        });
    }

    fn article(id: &str, title: &str) -> crate::qbt::RssArticle {
        crate::qbt::RssArticle { id: id.into(), title: title.into(), torrent_url: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789".into(), ..Default::default() }
    }

    fn feed(articles: Vec<crate::qbt::RssArticle>) -> RssFeed {
        RssFeed { uid:"fixture-feed".into(),name: "Example feed".into(), title: "Example feed".into(), articles, ..Default::default() }
    }

    fn render(app: &TuiApp, width: u16, height: u16) -> String {
        let mut pool = ftui::GraphemePool::new();
        let mut frame = Frame::new(width, height, &mut pool);
        app.view(&mut frame);
        (0..height).map(|y| (0..width).map(|x| frame.buffer.get(x, y).and_then(|cell| cell.content.as_char()).unwrap_or(' ')).collect::<String>())
            .collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn torrent_details_label_the_native_total_as_connected_peers() {
        let mut app = app();
        app.torrents.push(Torrent { name: "Peer label fixture".into(), num_seeds: 0,
            num_leechs: 7, progress_known: true, ..Default::default() });
        app.tor_list.borrow_mut().select(Some(0));
        let native = render(&app, 160, 45);
        assert!(native.contains("7 peers connected"));
        assert!(!native.contains("0 seeds") && !native.contains("leechers"));
        app.torrents[0].num_seeds = 3;
        assert!(render(&app, 160, 45).contains("7 peers connected"));
    }

    #[test]
    fn search_unknown_peer_counts_are_omitted_but_known_zero_is_retained() {
        let mut app = app();
        app.screen = Screen::Search;
        app.search_focus_input = false;
        app.search_list.borrow_mut().select(Some(0));
        app.search_results.push(SearchResult { file_name: "YTS missing counts".into(),
            engine: "yts".into(), seeders: -1, leechers: -1, ..Default::default() });
        let unknown = render(&app, 160, 45);
        assert!(!unknown.contains("seeds") && !unknown.contains("leechers"));
        // Check only the seed column: names, dates and sizes may contain digits.
        for width in [50, 80, 120] {
            let line = result_line(&app.search_results[0], None, width, &app.theme, None);
            assert_eq!(line.spans()[4].content.as_ref(), "      ");
        }
        app.search_results[0].seeders = 0;
        app.search_results[0].leechers = 4;
        let known = render(&app, 160, 45);
        assert!(known.contains("0 seeds, 4 leechers"));
    }

    #[test]
    fn torrent_restore_error_is_visible_in_details() {
        let mut app = app();
        app.torrents = vec![Torrent { hash: "a".repeat(40), name: "failed fixture".into(),
            state: "error".into(), error_message: Some("could not restore: corrupt cache".into()), ..Default::default() }];
        app.tor_list.borrow_mut().select(Some(0));
        let text = render(&app, 160, 40);
        assert!(text.contains("could not restore: corrupt cache"), "{text}");
    }

    #[test]
    fn reconnect_keeps_confirmed_rows_visible_without_retrying_mutations() {
        let mut app=app();
        app.torrents=vec![Torrent {hash:"a".repeat(40),name:"confirmed before disconnect".into(),state:"downloading".into(),..Default::default()}];
        app.tor_list.borrow_mut().select(Some(0));
        let reference=crate::api::MutationRef::new("remove-before-disconnect").unwrap();
        app.store.begin_mutation(reference.clone(),"torrents.remove".into(),vec![crate::api::Hash::new("a".repeat(40)).unwrap()]).unwrap();
        app.recovery.connected("0.1.0","0.1.0");
        app.connection_lost(false);
        let text=render(&app,120,30);
        assert!(text.contains("Connecting")&&text.contains("confirmed before disconnect"),"{text}");
        assert!(app.store.stale);
        assert!(app.store.pending_for_key(&"a".repeat(40)).is_none());
        assert!(matches!(app.recovery_tick(),Cmd::None),"the initial backoff does not dispatch a request");
        assert_eq!(app.store.pending.len(),1,"the unknown ref is retained for reconciliation");
        assert!(app.status.contains("Result unknown"));
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
        assert!(text.lines().nth(22).unwrap().contains("(u) Undo read"));
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
        app.update(Msg::ServiceDone { connection:None, op:"shows.grab", args:serde_json::json!({"imdb":"tt111","key":"S01E01"}), label:String::new(), result:Err("fixture add failed".into()) });
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
    fn marquee_once_then_stops() {
        let name="abcdefghij";
        assert_eq!(readable_cell(name,5,None),"ab…ij");
        let mut offsets=Vec::new();
        let mut cells=Vec::new();
        for tick in 0..24 {
            let elapsed=Duration::from_millis(tick*300);
            let offset=marquee_offset(dw(name),5,elapsed).unwrap_or(0);
            let cell=readable_cell(name,5,Some(elapsed));
            println!("fake_ms={} offset={offset} rendered={cell:?}",elapsed.as_millis());
            assert_eq!(dw(&cell),5);
            offsets.push(offset);cells.push(cell);
        }
        assert!(cells.iter().any(|cell|cell=="cdefg"),"8 cells/s must reach offset 2 at 1200 ms");
        assert!(cells.iter().any(|cell|cell=="fghij"),"must show the final complete window");
        assert!(cells[7..].iter().all(|cell|cell=="ab…ij"),"one pass must stay at its elided start: {cells:?}");
        assert!(offsets[7..].iter().all(|offset|*offset==0));
        for tick in 0..30 {
            let cell=readable_cell("字幕ファイル.mkv",8,Some(Duration::from_millis(tick*300)));
            assert_eq!(dw(&cell),8,"wide names must keep their row width: {cell:?}");
        }
    }

    #[test]
    fn reduce_motion_no_scroll() {
        let mut app=app();
        app.torrents=vec![Torrent { name:"Selected".into(),..Default::default() },Torrent { name:"Long hovered name, including the entire tail".into(),..Default::default() }];
        app.tor_list.borrow_mut().select(Some(0));
        app.tor_list.borrow_mut().hovered=Some(1);
        app.cfg.reduce_motion=true;
        app.sync_marquee();
        let expected=cell(&app.torrents[1].name,12);
        for tick in 0..24 {
            app.marquee_since=std::time::Instant::now()-Duration::from_millis(tick*300);
            let shown=readable_cell(&app.torrents[1].name,12,hovered_for(&app.tor_list,1,app.marquee_elapsed()));
            println!("reduce_motion fake_ms={} rendered={shown:?}",tick*300);
            assert_eq!(shown,expected);
            assert!(app.marquee_key.is_none());
        }
    }

    #[test]
    fn selected_long_name_is_static_even_under_pointer() {
        let mut app=app();
        app.torrents=vec![Torrent { hash:"static".into(),name:"A selected long name whose complete tail remains in the footer".into(),state:"stoppedDL".into(),..Default::default() }];
        app.tor_list.borrow_mut().select(Some(0));
        app.tor_list.borrow_mut().hovered=Some(0);
        app.sync_marquee();
        let before=render(&app,120,30);
        for ms in [0,900,1200,2100,5000,600_000] {
            app.marquee_since=std::time::Instant::now()-Duration::from_millis(ms);
            assert!(app.marquee_elapsed().is_none());
            assert_eq!(render(&app,120,30),before,"selected row moved at {ms} ms");
        }
        assert!(before.lines().last().unwrap().contains(&app.torrents[0].name));
    }

    #[test]
    fn hover_identity_change_rearms_the_one_pass_clock() {
        let mut app=app();
        app.torrents=vec![Torrent { name:"Selected".into(),..Default::default() },Torrent { hash:"hovered".into(),name:"Another long name".into(),..Default::default() }];
        app.tor_list.borrow_mut().select(Some(0));app.tor_list.borrow_mut().hovered=Some(1);
        app.sync_marquee();let previous=app.marquee_key;
        app.marquee_since=std::time::Instant::now()-Duration::from_secs(20);
        app.torrents[1].name.push_str(" changed");app.sync_marquee();
        assert_ne!(app.marquee_key,previous);
        assert!(app.marquee_elapsed().unwrap()<Duration::from_millis(300));
        app.clear_pointer_hover();assert!(app.marquee_elapsed().is_none());
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
            assert_eq!(text.contains("╭ Details "), width >= 140, "width {width}: {text}");
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
            assert_eq!(render(&app, width, 24).contains("╭ Details "), width >= 140);
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
        assert!(action_row.contains("(m) Mark read"), "{action_row}");
        assert!(action_row.contains("…"), "{action_row}");
        assert!(app.help_text().contains("(M) All read"));
        assert!(action_row.contains("(?) Help"), "{action_row}");
        app.set_status("A long action result that explains what happened without covering any command in the row above", false);
        app.feeds[0].articles[0].is_read = true;
        let text = render(&app, 80, 24);
        let action_row = text.lines().nth(22).unwrap();
        assert!(action_row.contains("(m) Mark unread"), "{action_row}");
        assert!(action_row.contains("…"), "{action_row}");
        assert!(app.help_text().contains("(M) All read"));
        assert!(action_row.contains("(?) Help"), "{action_row}");
        assert!(text.lines().nth(23).unwrap().contains("A long action result"));
    }

    #[test]
    fn settings_controls_and_small_terminal_keep_meaningful_labels() {
        let mut app = app();
        let mut list = ListState::default();
        list.select(Some(0));
        app.settings = Some(SettingsModal { original:Config::default(),saving:false, cfg: Config::default(), list: RefCell::new(list), connections: vec![Some("proton".into()), None], error: String::new() });
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
        app.on_key(KeyEvent::new(KeyCode::Down));
        app.on_key(KeyEvent::new(KeyCode::Right));
        assert!(app.settings.as_ref().unwrap().cfg.reduce_motion);
        assert!(!app.cfg.reduce_motion,"the modal stages its change until Save");
        assert!(render(&app,80,24).contains("Reduce motion"));
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
        assert!(text.contains("… Queue  "), "queued state must occupy its contract state cell: {text}");
        assert!(text.contains("⏚ VPN    "), "VPN state must occupy its contract state cell: {text}");
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
    fn rss_no_initial_refresh() {
        let mut app=app();
        assert!(matches!(app.update(Msg::Rss {revision:0,result:Ok(vec![feed(vec![article("item","FeedItemAlpha")])])}),Cmd::None));
        assert!(!app.rss_busy,"loading cached rows cannot start a network refresh");
    }

    #[test]
    fn rss_refresh_and_download_dispatch_only_once() {
        let mut app = app();
        let feed:crate::api::Feed=serde_json::from_value(serde_json::json!({"id":"fixture-feed","name":"Example feed","url":"http://127.0.0.1/feed","unread":1,"loading":false})).unwrap();
        let item:crate::api::FeedItem=serde_json::from_value(serde_json::json!({"id":"item","title":"FeedItemAlpha","read":false,"grabbed":false})).unwrap();
        app.store.rss_feeds.insert(feed.id.clone(),feed);
        app.store.rss_items.insert("fixture-feed".into(),[(item.id.clone(),item)].into());
        app.sync_native_feeds();
        assert!(matches!(app.refresh_rss(),Cmd::Task(..)));
        assert!(app.rss_busy);
        assert!(matches!(app.refresh_rss(), Cmd::None));
        app.service_done(None,"rss.refresh",serde_json::json!({}),"checked".into(),Ok(serde_json::json!({"state":"done"})));
        assert!(!app.rss_busy);
        app.on_rss_key(KeyEvent::new(KeyCode::Right));
        assert!(!app.feeds[0].articles[0].is_read);
        assert!(matches!(app.on_rss_key(KeyEvent::new(KeyCode::Enter)), Cmd::Task(..)));
        assert!(matches!(app.on_rss_key(KeyEvent::new(KeyCode::Enter)), Cmd::None));
        assert!(!app.feeds[0].articles[0].is_read,"grab intent cannot manufacture the daemon read flag");
    }

    #[test]
    fn native_controls_do_not_offer_unsupported_priorities_or_toggles() {
        let mut app = app();
        app.files = Some(FilesModal { hash: "hash".into(), name: "Torrent".into(), files: vec![TorrentFile { index: 0, name: "File".into(), priority: 1, ..Default::default() }], list: RefCell::new(ListState::default()), loading: false, saving:false,error: String::new() });
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
                if screen==Screen::Rss {app.rss_focus_items=true;}
                let text = render(&app, width, height);
                assert!(text.contains(expected), "{screen:?} {width}x{height}: {text}");
            }
            app.show_help = true;
            app.help_scroll = 0;
            assert!(render(&app, width, height).contains("GLOBAL"));
            app.on_key(KeyEvent::new(KeyCode::End));
            let text = render(&app, width, height);
            assert!(text.contains("Native streaming priority"), "help end {width}x{height}: {text}");
            assert!(text.contains("(Esc/q/?) Close"));
            app.show_help = false;
        }
        app.screen = Screen::Shows;
        render(&app, 80, 24);
        let area = app.shows_rects.get()[1];
        assert_eq!(app.mouse_list_at(area.x, area.y).unwrap().0, MouseList::Episodes);
        app.ep_list.borrow_mut().offset = 10;
        assert_eq!(mouse_row_index(area, area.x, area.y + 2, app.ep_list.borrow().offset, 30), Some(12));
    }
