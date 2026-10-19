use super::*;
use crate::api;
#[path="fixture.rs"] mod fixture;

pub(super) fn prepare(app: &mut TuiApp) {
    let frame:serde_json::Value=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v|v["frame"]["result"]["caps"].is_array()).unwrap();
    let mut hello:api::HelloResult=serde_json::from_value(frame["frame"]["result"].clone()).unwrap();
    if !hello.caps.iter().any(|c|c=="peers") {hello.caps.push("peers".into());}
    app.store.hello(hello);
    app.store.stale=false;
    let t=app.torrents[0].clone();let _=app.open_details(&t);
    app.details.as_mut().unwrap().tab=DetailsTab::Peers;
    let peers=vec![
        api::Peer {addr:"127.0.0.1:42001".into(),client:Some("rqbit 9.0.1".into()),conn:api::PeerConnection::TcpMse,incoming:false,state:"live".into(),downloaded:524288,uploaded:16384,dl_bps:Some(65536),ul_bps:Some(8192),seed:Some(true)},
        api::Peer {addr:"127.0.0.1:42002".into(),client:None,conn:api::PeerConnection::Tcp,incoming:true,state:"live".into(),downloaded:32768,uploaded:0,dl_bps:None,ul_bps:Some(0),seed:Some(false)},
    ];
    app.store.peers.insert(api::Hash::new(t.hash).unwrap(),peers.into_iter().map(|p|(p.addr.clone(),p)).collect());
}
#[test]
fn peers_tab_is_capability_gated_and_topic_exists_only_while_visible() {
    let mut app=fixture::app("torrents");let t=app.torrents[0].clone();app.open_details(&t);
    assert!(!app.details_tabs().contains(&DetailsTab::Peers));assert!(app.peer_topic().is_none());
    prepare(&mut app);
    assert_eq!(app.details_tabs(),[DetailsTab::Overview,DetailsTab::Files,DetailsTab::Peers,DetailsTab::Log]);
    assert!(matches!(app.peer_topic(),Some(api::Topic::Peers(_))));
    app.details_tab(true);assert_eq!(app.details.as_ref().unwrap().tab,DetailsTab::Log);assert!(app.peer_topic().is_none());assert!(app.store.peers.is_empty());
    app.details_tab(false);assert_eq!(app.details.as_ref().unwrap().tab,DetailsTab::Peers);
    app.close_details();assert!(app.peer_topic().is_none());
}
#[test]
fn peer_rows_preserve_unknowns_direction_and_scroll_without_retargeting() {
    let mut app=fixture::app("torrents");prepare(&mut app);
    for width in [80,120,160] {
        let text=golden::render(&app,width,30);
        for expected in ["[Peers]","127.0.0.1:42001","127.0.0.1:42002","rqbit 9.0.1","MSE","yes","no","66K","—"] {assert!(text.contains(expected),"missing {expected}:\n{text}");}
    }
    app.on_key(KeyEvent::new(KeyCode::Down));assert_eq!(app.details.as_ref().unwrap().peer_offset,1);
    let text=golden::render(&app,120,30);assert!(!text.contains("127.0.0.1:42001"));assert!(text.contains("127.0.0.1:42002"));
    app.on_key(KeyEvent::new(KeyCode::Up));assert_eq!(app.details.as_ref().unwrap().peer_offset,0);
}
#[test]
fn overview_and_table_show_known_seeds_over_total_and_keep_unknowns_absent() {
    let mut app=fixture::app("torrents");
    let frame:serde_json::Value=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|v|v["frame"]["result"]["torrents"].is_array()).unwrap();
    let mut row:api::Torrent=serde_json::from_value(frame["frame"]["result"]["torrents"][0].clone()).unwrap();
    row.peers=Some(41);row.seeds=Some(1);let t=format::torrent(&row);
    assert_eq!(format::Columns::of(&t,Some(&row)).peers,"1/41");
    assert_eq!((format::Metrics{legacy:&t,api:Some(&row)}).peers(true),"1 seeds / 41 peers connected");
    row.seeds=None;assert_eq!(format::Columns::of(&t,Some(&row)).peers,"41");
    row.peers=None;assert_eq!((format::Metrics{legacy:&t,api:Some(&row)}).peers(true),"unknown");
    app.store.torrents.insert(row.hash.to_string(),row);
}
