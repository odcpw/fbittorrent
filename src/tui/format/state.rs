//! State words and glyphs come from the same frozen table as the shell plugin.
use super::*;
use crate::torrent_format::definitions;
pub(in crate::tui) use crate::torrent_format::STATE_WIDTH;
fn key(legacy: &Torrent, row: Option<&api::Torrent>) -> &'static str {
    if let Some(row) = row {
        return crate::torrent_format::key(row.state,row.reason.as_deref(),row.playable.as_ref().is_some_and(|p|p.ready));
    }
    match legacy.state.as_str() {
        "metaDL"|"forcedMetaDL"=>"metadata", "checkingDL"|"checkingUP"|"checkingResumeData"=>"checking",
        "downloading"|"forcedDL"=>"downloading", "stalledDL"=>"stalled", "queuedDL"|"queuedUP"=>"queued",
        "stoppedDL"|"pausedDL"=>"paused", "stoppedUP"|"pausedUP"=>"done",
        "uploading"|"stalledUP"|"forcedUP"=>"seeding", "waitingVPN"=>"waiting_vpn",
        "error"|"missingFiles"=>"error", "disk_full"=>"disk_full", _=>"unknown",
    }
}
pub(in crate::tui) fn state_cell(legacy: &Torrent, row: Option<&api::Torrent>) -> &'static str {
    crate::torrent_format::cell(key(legacy,row),row.and_then(|r|r.reason.as_deref()))
}

pub(in crate::tui) fn state_word(legacy: &Torrent, row: Option<&api::Torrent>) -> &'static str {
    if row.is_some_and(|r|r.state==api::TorrentState::Error && r.reason.as_deref()==Some("missing_files")) {return "Files missing";}
    if let Some(p)=row.filter(|r|matches!(r.state,api::TorrentState::Downloading|api::TorrentState::Stalled)).and_then(|r|r.playable.as_ref()).filter(|p|p.ready) {
        if p.stall_free!=Some(true) && !(p.stall_free.is_none() && p.buffered_s.is_some_and(|s|s>=120)) {return "Playable · may stall";}
    }
    let key=key(legacy,row);
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
        let legacy=super::super::torrent(&row);
        assert_eq!(state_word(&legacy,Some(&row)),"Files missing");
        assert_eq!(state_cell(&legacy,Some(&row)),"! Missing");
        assert_eq!(super::super::Metrics{legacy:&legacy,api:Some(&row)}.progress(),"Files missing");
    }

    #[test]
    fn state_glyph_width_one() {
        for row in definitions() {
            println!("state={} glyph={} display_width={} cell={:?}",row.definition.state,row.definition.text_glyph,dw(row.definition.text_glyph),row.padded);
            assert_eq!(dw(row.definition.text_glyph),1,"{}",row.definition.state);
            assert_eq!(dw(&row.padded),STATE_WIDTH,"{}",row.definition.state);
        }
    }
    #[test]
    fn every_torrent_state_has_a_word_at_every_width() {
        for (state,label) in [("metaDL","◌ Meta"),("checkingDL","⟳ Check"),("downloading","↓ Down"),
            ("stalledDL","~ Stall"),("queuedDL","… Queue"),("stoppedDL","‖ Pause"),
            ("stoppedUP","✓ Done"),("uploading","↑ Upload"),("waitingVPN","⏚ VPN"),("error","! Error")] {
            let row=Torrent {state:state.into(),..Default::default()};
            assert_eq!(state_cell(&row,None).trim_end(),label);
            assert_eq!(dw(state_cell(&row,None)),9);
            for width in [80,120,160] { assert!(width>dw(state_cell(&row,None))); }
        }
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    #[test]
    fn ready_and_may_stall_words_are_truthful() {
        let mut row:api::Torrent=serde_json::from_value(serde_json::json!({"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"Video.mkv","state":"downloading","intent":"run","staged":false,"checked":true,"seed":false,"upload_policy":"off","save_path":"/tmp/fixture","added_at":1,"force":false,"playable":{"file":0,"prefix_bytes":33554432,"ready":true,"acked":false,"buffered_s":32,"stall_free":false}})).unwrap();
        let legacy=Torrent::default();
        assert_eq!(state_cell(&legacy,Some(&row)).trim_end(),"▶ Ready");
        assert_eq!(state_word(&legacy,Some(&row)),"Playable · may stall");
        row.state=api::TorrentState::Stalled;
        assert_eq!(state_word(&legacy,Some(&row)),"Playable · may stall");
        row.state=api::TorrentState::Downloading;
        row.playable.as_mut().unwrap().stall_free=Some(true);
        assert_eq!(state_word(&legacy,Some(&row)),"Ready to watch");
        row.playable.as_mut().unwrap().stall_free=None;
        row.playable.as_mut().unwrap().buffered_s=Some(120);
        assert_eq!(state_word(&legacy,Some(&row)),"Ready to watch");
        println!("same ready glyph; truthful may-stall and known/120s-buffer words");
    }
}
