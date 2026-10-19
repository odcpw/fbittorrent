//! Disk warnings come only from confirmed daemon fields, including unknown free space.
use super::*;
#[path = "fixture.rs"]
mod fixture;
fn frame(path: &str, field: &str) -> serde_json::Value {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| if field=="result" {v["frame"]["result"]["torrents"].as_array().is_some_and(|rows|!rows.is_empty())}else{v["frame"]["topic"]=="summary"&&v["frame"]["data"].is_object()})
        .unwrap()["frame"]
        .clone()
}
pub(super) fn prepare(app: &mut TuiApp) {
    let mut summary: crate::api::Summary =
        serde_json::from_value(frame("contracts/ipc/v1/subscribe.ndjson", "data")["data"].clone())
            .unwrap();
    summary.disk = crate::api::Disk {
        needed_bytes: 3_328_599_654,
        free_bytes: Some(1_288_490_188),
        low: true,
    };
    app.store.summary = Some(summary);
    app.theme = Theme::from_toml(include_str!("../../../contracts/ui/themes/owner.toml")).unwrap();
    let mut row: crate::api::Torrent = serde_json::from_value(
        frame("contracts/ipc/v1/torrents.ndjson", "result")["result"]["torrents"][0].clone(),
    )
    .unwrap();
    row.hash = crate::api::Hash::new(app.torrents[0].hash.clone()).unwrap();
    row.name = app.torrents[0].name.clone();
    row.state = crate::api::TorrentState::Error;
    row.reason = Some("disk_full".into());
    row.error = Some("Disk full; free space to resume downloads".into());
    row.dl_bps = None;
    row.ul_bps = None;
    row.eta_s = None;
    row.peers = None;
    row.seeds = None;
    row.playable = None;
    app.torrents[0] = format::torrent(&row);
    app.store.torrents.insert(row.hash.to_string(), row);
}
#[test]
fn disk_banner_and_state_are_worded_at_every_size() {
    for (w, h) in [(80, 24), (120, 30), (160, 45)] {
        let app = fixture::app("disk");
        let pane = super::golden::render(&app, w, h);
        let banner = pane.lines().nth(h as usize - 3).unwrap();
        assert!(banner.starts_with("! Not enough space for the running downloads:"));
        assert!(banner.contains("3.1 GiB needed, 1.2 GiB free"));
        assert!(pane.contains("! Disk"));
        assert_eq!(
            dw(format::state_cell(
                &app.torrents[0],
                app.store.torrents.get(&app.torrents[0].hash)
            )),
            9
        );
        assert!(pane
            .lines()
            .nth(h as usize - 2)
            .unwrap()
            .contains("(?) Help"));
        println!(
            "{w}x{h}: banner row={} {banner}; state=! Disk, footer starts at {}",
            h - 3,
            h - 2
        );
    }
}
#[test]
fn disk_warning_has_priority_and_unknown_space_never_claims_zero() {
    let mut app = fixture::app("disk");
    app.store.network = Some(crate::api::NetworkState {
        ready: false,
        message: "VPN missing".into(),
        selection: None,
        interface: None,
    });
    app.daemon_stopped = true;
    app.store.summary.as_mut().unwrap().daemon.upgrade_pending = true;
    let banner = app.priority_banner().unwrap();
    assert!(banner.starts_with("Not enough space"));
    app.store.summary.as_mut().unwrap().disk.free_bytes = None;
    let pane = super::golden::render(&app, 80, 24);
    assert!(pane.contains("free space unknown"));
    assert!(!pane.contains("0 B free"));
    app.store.summary.as_mut().unwrap().disk.low = false;
    assert!(app.priority_banner().unwrap().contains("Disk full"));
    for row in app.store.torrents.values_mut() {
        row.reason = None;
        row.error = None;
        row.state = crate::api::TorrentState::Downloading;
    }
    assert_eq!(app.priority_banner().as_deref(), Some("VPN missing"));
    app.store.network.as_mut().unwrap().ready = true;
    assert_eq!(
        app.priority_banner().as_deref(),
        Some("fbittorrent stopped · Enter starts it again")
    );
    app.daemon_stopped = false;
    assert!(app
        .priority_banner()
        .unwrap()
        .starts_with("Update installed"));
    app.store.summary.as_mut().unwrap().daemon.upgrade_pending = false;
    assert!(app.priority_banner().is_none());
    println!("priority: disk low > disk full > VPN > stopped > upgrade; unknown free space stays unknown; recovery clears the banner");
}
