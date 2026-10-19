//! State words and glyphs come from the same frozen table as the shell plugin.
use super::*;
use serde::Deserialize;
use std::sync::OnceLock;

pub(in crate::tui) const STATE_WIDTH: usize = 9;
#[derive(Deserialize)]
struct Definition {
    state: &'static str,
    text_glyph: &'static str,
    short: &'static str,
    word: &'static str,
}
struct StateCell { definition: Definition, padded: String }
fn definitions() -> &'static [StateCell] {
    static STATES: OnceLock<Vec<StateCell>> = OnceLock::new();
    STATES.get_or_init(|| {
        let table: Vec<Definition> = serde_json::from_str(include_str!("../../../contracts/ui/states.json"))
            .expect("checked-in state contract is valid");
        table.into_iter().map(|definition| {
            let mut padded = format!("{} {}",definition.text_glyph,definition.short);
            let width = dw(&padded);
            assert!(width <= STATE_WIDTH,"state contract exceeds the state column");
            padded.extend(std::iter::repeat_n(' ',STATE_WIDTH-width));
            StateCell { definition,padded }
        }).collect()
    })
}
fn key(legacy: &Torrent, row: Option<&api::Torrent>) -> &'static str {
    if let Some(row) = row {
        if row.state == api::TorrentState::Error && row.reason.as_deref() == Some("disk_full") { return "disk_full"; }
        return match row.state {
            api::TorrentState::Metadata=>"metadata", api::TorrentState::Checking=>"checking",
            api::TorrentState::Downloading if row.playable.as_ref().is_some_and(|p|p.ready)=>"ready",
            api::TorrentState::Downloading=>"downloading", api::TorrentState::Stalled=>"stalled",
            api::TorrentState::Queued=>"queued", api::TorrentState::Paused=>"paused",
            api::TorrentState::Done=>"done",api::TorrentState::Seeding=>"seeding",
            api::TorrentState::WaitingVpn=>"waiting_vpn",api::TorrentState::Error=>"error",
        };
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
    let key=key(legacy,row);
    definitions().iter().find(|s|s.definition.state==key).map_or("? Unknown",|s|s.padded.as_str())
}
pub(in crate::tui) fn state_word(legacy: &Torrent, row: Option<&api::Torrent>) -> &'static str {
    let key=key(legacy,row);
    definitions().iter().find(|s|s.definition.state==key).map_or("Unknown",|s|s.definition.word)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            ("stoppedUP","✓ Done"),("uploading","↑ Seed"),("waitingVPN","⏚ VPN"),("error","! Error")] {
            let row=Torrent {state:state.into(),..Default::default()};
            assert_eq!(state_cell(&row,None).trim_end(),label);
            assert_eq!(dw(state_cell(&row,None)),9);
            for width in [80,120,160] { assert!(width>dw(state_cell(&row,None))); }
        }
    }
}
