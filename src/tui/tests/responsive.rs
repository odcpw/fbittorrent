//! Rendered geometry, typed header sorting and bounded row-formatting controls.
use super::*;
#[path = "fixture.rs"]
mod fixture;

fn draw(app: &TuiApp, width: u16, height: u16) {
    let mut pool = ftui::GraphemePool::new();
    let mut frame = Frame::new(width, height, &mut pool);
    app.view(&mut frame);
}
#[test]
fn click_hits_rows_at_each_size() {
    use ftui::{MouseButton, MouseEvent, MouseEventKind};
    for (width, height) in [(80, 24), (120, 30), (160, 45)] {
        let mut app = fixture::app("torrents");
        app.torrents = (0..100)
            .map(|i| Torrent {
                hash: format!("{i:040x}"),
                name: format!("Episode.S01E{i:03}.1080p.mkv"),
                state: "downloading".into(),
                progress_known: true,
                ..Default::default()
            })
            .collect();
        app.tor_list.borrow_mut().select(Some(0));
        draw(&app, width, height);
        let rows = app.rects.borrow().rows.clone();
        assert!(rows.len() > 10 && rows.len() < 100);
        for (hash, rect) in rows {
            let x = rect.x + 12;
            let y = rect.y;
            app.on_mouse(MouseEvent::new(
                MouseEventKind::Down(MouseButton::Left),
                x,
                y,
            ));
            assert_eq!(
                app.selected_torrent().unwrap().hash,
                hash,
                "{width}x{height} at {x},{y}"
            );
            println!("{width}x{height} click ({x},{y}) -> hash={hash}");
        }
    }
}
#[test]
fn recorded_hash_survives_a_model_reorder_before_the_next_frame() {
    use ftui::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = fixture::app("torrents");
    draw(&app, 80, 24);
    let (hash, rect) = app.rects.borrow().rows[0].clone();
    app.torrents.swap(0, 1);
    app.on_mouse(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        rect.x + 12,
        rect.y,
    ));
    assert_eq!(app.selected_torrent().unwrap().hash, hash);
}
#[test]
fn unchanged_frames_format_only_one_visible_window() {
    let mut app = fixture::app("torrents");
    app.torrents = (0..5000)
        .map(|i| Torrent {
            hash: format!("{i:040x}"),
            name: format!("Episode.{i:05}.mkv"),
            state: "downloading".into(),
            progress_known: true,
            ..Default::default()
        })
        .collect();
    app.tor_list.borrow_mut().select(Some(0));
    for _ in 0..32 {
        draw(&app, 120, 30);
    }
    assert_eq!(app.torrent_table.borrow().builds, 1);
    assert!(app.rects.borrow().rows.len() < 30);
    app.torrents[4999].name.push_str(".changed");
    draw(&app, 120, 30);
    assert_eq!(
        app.torrent_table.borrow().builds,
        1,
        "off-screen changes need no row formatting"
    );
    app.torrents[0].progress = 0.43;
    draw(&app, 120, 30);
    assert_eq!(app.torrent_table.borrow().builds, 2);
    app.theme.fg = ftui::PackedRgba::rgb(1, 2, 3);
    draw(&app, 120, 30);
    assert_eq!(app.torrent_table.borrow().builds, 3);
    println!(
        "5000 rows, {} visible, one row-format build over 32 unchanged frames",
        app.rects.borrow().rows.len()
    );
}
#[test]
fn header_clicks_sort_bytes_numerically_and_keep_unknown_last() {
    use crate::api;
    use ftui::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = fixture::app("torrents");
    let frame: serde_json::Value = include_str!("../../../contracts/ipc/v1/torrents.ndjson")
        .lines()
        .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
        .find(|v| v["frame"]["result"]["torrents"].is_array())
        .unwrap();
    let template: api::Torrent =
        serde_json::from_value(frame["frame"]["result"]["torrents"][0].clone()).unwrap();
    for (i, size) in [Some(943_718_400), Some(1_181_116_006), None]
        .into_iter()
        .enumerate()
    {
        let mut row = template.clone();
        row.hash = api::Hash::new(format!("{i:040x}")).unwrap();
        row.size = size;
        app.store.torrents.insert(row.hash.to_string(), row);
    }
    app.torrents = app
        .store
        .sorted_torrents(store::TorrentSort::Added, true)
        .into_iter()
        .map(format::torrent)
        .collect();
    let sizes = |app: &TuiApp| {
        app.torrents
            .iter()
            .map(|t| app.store.torrents[&t.hash].size)
            .collect::<Vec<_>>()
    };
    draw(&app, 120, 30);
    let header = app
        .rects
        .borrow()
        .headers
        .iter()
        .find(|(c, _)| *c == Column::Size)
        .unwrap()
        .1;
    app.on_mouse(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        header.x,
        header.y,
    ));
    assert_eq!(
        sizes(&app),
        vec![Some(1_181_116_006), Some(943_718_400), None]
    );
    draw(&app, 120, 30);
    app.on_mouse(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        header.x,
        header.y,
    ));
    assert_eq!(
        sizes(&app),
        vec![Some(943_718_400), Some(1_181_116_006), None]
    );
}

#[test]
fn enter_opens_overview_and_brackets_stop_at_tab_ends_at_each_size() {
    for (width, height) in [(80, 24), (120, 30), (160, 45), (120, 24)] {
        let mut app = fixture::app("torrents");
        app.on_key(KeyEvent::new(KeyCode::Enter));
        assert!(app.details.is_some());
        assert!(app.files.is_none(), "Overview must not fetch file data");
        let text = super::golden::render(&app, width, height);
        assert!(text.contains("[Overview]  Files"));
        assert!(text.contains("↓ Down · downloading"));
        let rect = app.rects.borrow().details.unwrap();
        assert_eq!(
            rect,
            if width < 100 || height < 30 {
                Rect::new(0, 0, width, height)
            } else {
                Rect::new((width - 100) / 2, (height - 30) / 2, 100, 30)
            }
        );
        app.on_key(KeyEvent::new(KeyCode::Char('[')));
        assert_eq!(app.details.as_ref().unwrap().tab, DetailsTab::Overview);
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char(']'))),
            Cmd::Task(..)
        ));
        assert_eq!(app.details.as_ref().unwrap().tab, DetailsTab::Files);
        assert!(app.files.is_some());
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char(']'))),
            Cmd::None
        ));
        app.on_key(KeyEvent::new(KeyCode::Char('[')));
        assert!(app.files.is_none());
        assert_eq!(app.details.as_ref().unwrap().tab, DetailsTab::Overview);
        app.on_key(KeyEvent::new(KeyCode::Char('h')));
        assert!(app.details.is_none());
        println!(
            "{width}x{height}: Enter Overview; ] Files; ] at end; [ Overview; [ at start; h Back"
        );
    }
}
#[test]
fn details_keeps_its_hash_and_never_retargets_a_removed_torrent() {
    let mut app = fixture::app("torrents");
    app.on_key(KeyEvent::new(KeyCode::Enter));
    let hash = app.selected_torrent().unwrap().hash.clone();
    app.torrents.swap(0, 1);
    assert_eq!(app.selected_torrent().unwrap().hash, hash);
    app.torrents.retain(|t| t.hash != hash);
    assert!(app.selected_torrent().is_none());
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Char('p'))),
        Cmd::None
    ));
    let text = super::golden::render(&app, 80, 24);
    assert!(text.contains("no longer in your list"));
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert!(app.details.is_none());
}
#[test]
fn priority_banner_preserves_disk_vpn_and_reconnect_order() {
    let mut app = fixture::app("torrents");
    app.daemon_stopped = true;
    assert!(app.priority_banner().unwrap().contains("stopped"));
    app.store.network = Some(crate::api::NetworkState {
        ready: false,
        message: "Waiting for ProtonVPN (proton0). Downloads resume when it connects.".into(),
        selection: Some("proton".into()),
        interface: Some("proton0".into()),
    });
    assert!(app
        .priority_banner()
        .unwrap()
        .starts_with("Waiting for ProtonVPN"));
    let frame: serde_json::Value = include_str!("../../../contracts/ipc/v1/torrents.ndjson")
        .lines()
        .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
        .find(|v| v["frame"]["result"]["torrents"].is_array())
        .unwrap();
    let mut row: crate::api::Torrent =
        serde_json::from_value(frame["frame"]["result"]["torrents"][0].clone()).unwrap();
    row.reason = Some("disk_full".into());
    row.state = crate::api::TorrentState::Error;
    row.error = Some("Disk full; free space".into());
    app.store.torrents.insert(row.hash.to_string(), row);
    assert!(app.priority_banner().unwrap().starts_with("Disk full"));
    let text = super::golden::render(&app, 80, 24);
    assert!(text.contains("! Disk full; free space"));
    app.store.torrents.clear();
    app.store.network = None;
    app.daemon_stopped = false;
    assert!(
        app.priority_banner().is_none(),
        "no permanent empty banner slot"
    );
}

#[test]
fn click_every_other_visible_list_row_at_each_size() {
    use ftui::{MouseButton, MouseEvent, MouseEventKind};
    for (width, height) in [(80, 24), (120, 30), (160, 45)] {
        for surface in ["search", "shows", "rss", "files", "providers"] {
            let mut app = fixture::app(surface);
            draw(&app, width, height);
            let lists = app.rects.borrow().lists;
            for (list, rect) in [
                MouseList::Results,
                MouseList::Shows,
                MouseList::Episodes,
                MouseList::Feeds,
                MouseList::Items,
                MouseList::Files,
                MouseList::Providers,
            ]
            .into_iter()
            .filter_map(|list| lists[list as usize].map(|rect| (list, rect)))
            {
                let offset = app.mouse_list_state(list).borrow().offset;
                for local in
                    0..usize::from(rect.height).min(app.mouse_list_len(list).saturating_sub(offset))
                {
                    let index = offset + local;
                    let x = rect.x + 2;
                    let y = rect.y + local as u16;
                    let identity = app.mouse_item_identity(list, index).unwrap();
                    app.on_mouse(MouseEvent::new(
                        MouseEventKind::Down(MouseButton::Left),
                        x,
                        y,
                    ));
                    assert_eq!(
                        app.mouse_list_state(list).borrow().selected(),
                        Some(index),
                        "{surface} {width}x{height} {list:?} {x},{y}"
                    );
                    println!("{width}x{height} {list:?} click ({x},{y}) -> {identity}");
                }
            }
        }
    }
}
#[test]
fn every_non_torrent_list_formats_only_its_visible_slice_once() {
    for surface in ["search", "shows", "rss", "files", "providers"] {
        let mut app = fixture::app(surface);
        let list = match surface {
            "search" => {
                let template = app.search_results[0].clone();
                app.search_results = (0..5000)
                    .map(|i| {
                        let mut r = template.clone();
                        r.file_name = format!("Result {i}");
                        r.file_url = format!("magnet:{i}");
                        r
                    })
                    .collect();
                MouseList::Results
            }
            "shows" => {
                app.select_show_parent(1);
                app.shows_focus_eps = true;
                let template = app.shows.shows[0].episodes[0].clone();
                app.shows.shows[0].episodes = (0..5000)
                    .map(|i| {
                        let mut e = template.clone();
                        e.key = format!("S100E{i:04}");
                        e
                    })
                    .collect();
                MouseList::Episodes
            }
            "rss" => {
                let t = app.feeds[0].articles[0].clone();
                app.feeds[0].articles = (0..5000)
                    .map(|i| {
                        let mut a = t.clone();
                        a.id = i.to_string();
                        a
                    })
                    .collect();
                MouseList::Items
            }
            "files" => {
                let m = app.files.as_mut().unwrap();
                let t = m.files[0].clone();
                m.files = (0..5000)
                    .map(|i| {
                        let mut f = t.clone();
                        f.index = i;
                        f
                    })
                    .collect();
                MouseList::Files
            }
            _ => {
                let m = app.providers.as_mut().unwrap();
                let t = m.plugins[0].clone();
                m.plugins = (0..5000)
                    .map(|i| {
                        let mut p = t.clone();
                        p.name = i.to_string();
                        p
                    })
                    .collect();
                MouseList::Providers
            }
        };
        for _ in 0..32 {
            draw(&app, 80, 24);
        }
        assert_eq!(
            app.list_tables.borrow()[list as usize].builds,
            1,
            "{surface}"
        );
        let area = app.rects.borrow().lists[list as usize].unwrap();
        assert!(area.height < 24);
        let max = app.mouse_list_len(list) - 1;
        app.mouse_list_state(list).borrow_mut().select(Some(max));
        draw(&app, 80, 24);
        assert_eq!(
            app.list_tables.borrow()[list as usize].builds,
            2,
            "scroll formats one new slice for {surface}"
        );
        let state = app.mouse_list_state(list).borrow();
        assert!(state.offset <= max && max < state.offset + area.height as usize);
        println!("{surface}: 5000 rows, {} visible, one formatting build over 32 unchanged frames, last row reachable",area.height);
    }
}
#[test]
fn narrow_breadcrumb_navigation_and_search_card_do_not_start_work() {
    let mut app = fixture::app("shows");
    let before = app.shows.new_total();
    let text = super::golden::render(&app, 80, 24);
    assert!(text.contains("Shows › All unread"));
    assert!(text.contains("S04E01") && text.contains("● Unread"));
    app.on_key(KeyEvent::new(KeyCode::Char('h')));
    assert!(!app.shows_focus_eps);
    assert!(super::golden::render(&app, 80, 24).contains("Shows › Followed"));
    app.on_key(KeyEvent::new(KeyCode::Char('l')));
    assert!(app.shows_focus_eps);
    assert_eq!(app.shows.new_total(), before);
    let mut app = fixture::app("rss");
    assert!(super::golden::render(&app, 80, 24).contains("RSS › Blender releases"));
    app.on_key(KeyEvent::new(KeyCode::Char('h')));
    assert!(!app.rss_focus_items);
    assert!(super::golden::render(&app, 80, 24).contains("RSS › Feeds"));
    app.on_key(KeyEvent::new(KeyCode::Char('l')));
    assert!(app.rss_focus_items);
    assert!(!app.rss_busy);
    let mut app = fixture::app("search");
    for width in [80, 120, 160] {
        let text = super::golden::render(&app, width, 30);
        assert!(text.contains("SEEDS") && text.contains('★'));
        if width >= 100 {
            assert!(
                text.contains("UPLOADED") && text.contains("PROVIDER") && text.contains("PEERS")
            );
        }
    }
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Char('v'))),
        Cmd::None
    ));
    assert!(app.poster_open);
    assert!(super::golden::render(&app, 80, 24).contains("╭ IMDb"));
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Escape)),
        Cmd::None
    ));
    assert!(!app.poster_open);
    assert_eq!(app.search_results.len(), 10);
}

#[test]
fn search_header_clicks_compare_bytes_and_preserve_the_selected_source() {
    use ftui::{MouseButton, MouseEvent, MouseEventKind};
    for (width, height) in [(80, 24), (120, 30), (160, 45)] {
        let mut app = fixture::app("search");
        app.search_results.truncate(3);
        for (row, size) in app
            .search_results
            .iter_mut()
            .zip([943_718_400, 1_181_116_006, -1])
        {
            row.file_size = size;
        }
        draw(&app, width, height);
        let header = app
            .rects
            .borrow()
            .headers
            .iter()
            .find(|(c, _)| *c == Column::Size)
            .unwrap()
            .1;
        let click = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), header.x, header.y);
        let source = app.selected_result().unwrap().file_url.clone();
        app.on_mouse(click);
        assert_eq!(
            app.search_results
                .iter()
                .map(|r| r.file_size)
                .collect::<Vec<_>>(),
            [1_181_116_006, 943_718_400, -1]
        );
        assert_eq!(app.selected_result().unwrap().file_url, source);
        app.on_mouse(click);
        assert_eq!(
            app.search_results
                .iter()
                .map(|r| r.file_size)
                .collect::<Vec<_>>(),
            [943_718_400, 1_181_116_006, -1]
        );
        assert_eq!(app.selected_result().unwrap().file_url, source);
        println!("{width}x{height} search header: bytes compare numerically, absent last, selected URL preserved");
    }
}

#[test]
fn known_zero_search_size_and_absent_size_render_differently(){
    let zero=SearchResult{file_size:0,..Default::default()};
    let absent=SearchResult{file_size:-1,..Default::default()};
    let theme=Theme::default();
    assert!(result_line(&zero,None,80,&theme,None).spans()[2].content.contains("0 B"));
    assert_eq!(result_line(&absent,None,80,&theme,None).spans()[2].content.trim(),"—");
}

#[test]
fn details_draws_one_legend_and_keeps_confirmation_visible(){
    for(width,height)in[(80,24),(120,30),(160,45)] {
        let mut app=fixture::app("details");
        let text=super::golden::render(&app,width,height);
        assert_eq!(text.matches("[Details]").count(),1,"{text}");
        app.on_key(KeyEvent::new(KeyCode::Char('d')));
        assert!(app.prompt.as_ref().is_some_and(|p|p.yes_no));
        let text=super::golden::render(&app,width,height);
        assert!(text.contains("keep files"),"{text}");
        assert!(text.contains("y / N (default: No)"),"{text}");
        app.on_key(KeyEvent::new(KeyCode::Escape));
        assert!(app.prompt.is_none() && app.details.is_some());
    }
}
