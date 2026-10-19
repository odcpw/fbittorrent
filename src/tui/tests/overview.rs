//! The two Overview editors keep hash identity and use confirmed daemon writes.
use super::*;
#[path="fixture.rs"]
mod fixture;
#[test]
fn overview_prompts_capture_hash_and_validate_without_editing_the_row() {
    let mut app=fixture::app("torrents");
    let hash=app.torrents[0].hash.clone();
    app.overview_prompt(false);
    assert_eq!(app.prompt.as_ref().unwrap().kind,PromptKind::MoveTorrent{hash:hash.clone()});
    assert_eq!(app.prompt.as_ref().unwrap().input.value(),"/fixture/downloads");
    let command=app.apply_overview(PromptKind::MoveTorrent {hash:hash.clone()},"relative/path");
    assert!(matches!(command,Cmd::None));assert!(app.status_is_error);
    assert!(app.prompt.is_some());assert_eq!(app.torrents[0].save_path,"/fixture/downloads");
    app.close_prompt();
    assert!(matches!(app.apply_overview(PromptKind::MoveTorrent {hash:hash.clone()},"/private/new location"),Cmd::Task(..)));
    assert_eq!(app.torrents[0].save_path,"/fixture/downloads");
    app.service_busy=false;
    assert!(matches!(app.apply_overview(PromptKind::TorrentDownloadCap {hash:hash.clone()},"64K"),Cmd::Task(..)));
    app.service_busy=false;
    assert!(matches!(app.apply_overview(PromptKind::TorrentDownloadCap {hash:hash.clone()},""),Cmd::Task(..)));
    app.service_busy=false;
    assert!(matches!(app.apply_overview(PromptKind::TorrentDownloadCap {hash},"1K"),Cmd::None));
    assert!(app.prompt.is_some());assert!(app.status_is_error);
}
