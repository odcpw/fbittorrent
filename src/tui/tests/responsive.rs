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
