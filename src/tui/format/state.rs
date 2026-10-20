//! State words and glyphs come from the same frozen table as the shell plugin.
use super::*;
use crate::torrent_format::definitions;
fn key(row: &api::Torrent) -> &'static str {
    crate::torrent_format::key(row.state,row.reason.as_deref(),row.playable.as_ref().is_some_and(|p|p.ready))
}
pub(in crate::tui) fn state_cell(row: &api::Torrent) -> &'static str {
    crate::torrent_format::cell(key(row),row.reason.as_deref())
}

pub(in crate::tui) fn state_word(row: &api::Torrent) -> &'static str {
    if row.state==api::TorrentState::Error && row.reason.as_deref()==Some("missing_files") {return "Files missing";}
    if let Some(p)=Some(row).filter(|r|matches!(r.state,api::TorrentState::Downloading|api::TorrentState::Stalled)).and_then(|r|r.playable.as_ref()).filter(|p|p.ready) {
        if p.stall_free!=Some(true) && !(p.stall_free.is_none() && p.buffered_s.is_some_and(|s|s>=120)) {return "Playable · may stall";}
    }
    let key=key(row);
    definitions().iter().find(|s|s.definition.state==key).map_or("Unknown",|s|s.definition.word)
}

pub(in crate::tui) fn reason(code: &str) -> &str {
    definitions().iter().find(|s|s.definition.state=="reason")
        .and_then(|s|s.definition.phrases.get(code).copied()).unwrap_or(code)
}

pub(in crate::tui) fn tracker_label(status: &api::TrackerStatus) -> String {
    let key=match status {api::TrackerStatus::Ok=>"tracker.ok",api::TrackerStatus::Error=>"tracker.error",api::TrackerStatus::Updating=>"tracker.updating",api::TrackerStatus::NotContacted=>"tracker.not_contacted"};
    let state=&definitions().iter().find(|s|s.definition.state==key).expect("tracker state contract").definition;
    let gap=if dw(state.text_glyph)+dw(state.word)+1>14 {""}else{" "};
    format!("{}{gap}{}",state.text_glyph,state.word)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_files_replaces_done_and_percentage_in_every_tui_view() {
        let row:api::Torrent=serde_json::from_value(serde_json::json!({
            "hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"Deleted film",
            "state":"error","reason":"missing_files","intent":"run","staged":false,
            "checked":false,"seed":false,"upload_enabled":false,"upload_policy":"off",
            "save_path":"/fixture/downloads","added_at":1,"force":false
        })).unwrap();
        assert_eq!(state_word(&row),"Files missing");
        assert_eq!(state_cell(&row),"! Missing");
        assert_eq!(super::super::Metrics(&row).progress(),"Files missing");
    }

    #[test]
    fn state_glyph_width_one() {
        for row in definitions() {
            println!("state={} glyph={} display_width={} cell={:?}",row.definition.state,row.definition.text_glyph,dw(row.definition.text_glyph),row.padded);
            assert_eq!(dw(row.definition.text_glyph),1,"{}",row.definition.state);
            assert_eq!(dw(&row.padded),crate::torrent_format::STATE_WIDTH,"{}",row.definition.state);
        }
    }
    #[test]
    fn every_torrent_state_has_a_word_at_every_width() {
        use api::TorrentState::*;
        let mut row:api::Torrent=serde_json::from_value(serde_json::json!({"hash":"a".repeat(40),"name":"Row","state":"paused",
            "intent":"run","staged":false,"checked":true,"seed":false,"upload_policy":"off","save_path":"/fixture","added_at":1,"force":false})).unwrap();
        for (state,label) in [(Metadata,"◌ Meta"),(Checking,"⟳ Check"),(Downloading,"↓ Down"),
            (Stalled,"~ Stall"),(Queued,"… Queue"),(Paused,"‖ Pause"),
            (Done,"✓ Done"),(Seeding,"↑ Upload"),(WaitingVpn,"⏚ VPN"),(Error,"! Error")] {
            row.state=state;
            assert_eq!(state_cell(&row).trim_end(),label);
            assert_eq!(dw(state_cell(&row)),9);
            for width in [80,120,160] { assert!(width>dw(state_cell(&row))); }
        }
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    #[test]
    fn ready_and_may_stall_words_are_truthful() {
        let mut row:api::Torrent=serde_json::from_value(serde_json::json!({"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"Video.mkv","state":"downloading","intent":"run","staged":false,"checked":true,"seed":false,"upload_policy":"off","save_path":"/tmp/fixture","added_at":1,"force":false,"playable":{"file":0,"prefix_bytes":33554432,"ready":true,"acked":false,"buffered_s":32,"stall_free":false}})).unwrap();
        assert_eq!(state_cell(&row).trim_end(),"▶ Ready");
        assert_eq!(state_word(&row),"Playable · may stall");
        row.state=api::TorrentState::Stalled;
        assert_eq!(state_word(&row),"Playable · may stall");
        row.state=api::TorrentState::Downloading;
        row.playable.as_mut().unwrap().stall_free=Some(true);
        assert_eq!(state_word(&row),"Ready to watch");
        row.playable.as_mut().unwrap().stall_free=None;
        row.playable.as_mut().unwrap().buffered_s=Some(120);
        assert_eq!(state_word(&row),"Ready to watch");
        println!("same ready glyph; truthful may-stall and known/120s-buffer words");
    }
}
