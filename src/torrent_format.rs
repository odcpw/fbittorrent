//! Human CLI and TUI use one state vocabulary and progress rounding.
use crate::api;
use unicode_width::UnicodeWidthStr;
fn dw(value:&str)->usize {UnicodeWidthStr::width(value)}
use serde::Deserialize;
use std::sync::OnceLock;

pub(crate) const STATE_WIDTH: usize = 9;
#[derive(Deserialize)]
pub(crate) struct Definition {
    pub(crate) state: &'static str,
    pub(crate) text_glyph: &'static str,
    pub(crate) short: &'static str,
    pub(crate) word: &'static str,
    #[serde(default)]
    pub(crate) phrases: std::collections::BTreeMap<&'static str, &'static str>,
}
pub(crate) struct StateCell { pub(crate) definition: Definition, pub(crate) padded: String }
pub(crate) fn definitions() -> &'static [StateCell] {
    static STATES: OnceLock<Vec<StateCell>> = OnceLock::new();
    STATES.get_or_init(|| {
        let table: Vec<Definition> = serde_json::from_str(include_str!("../contracts/ui/states.json"))
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

pub(crate) fn key(state:api::TorrentState,reason:Option<&str>,ready:bool)->&'static str {
    if state==api::TorrentState::Error && reason==Some("disk_full") {return "disk_full";}
    match state {
        api::TorrentState::Metadata=>"metadata",api::TorrentState::Checking=>"checking",
        api::TorrentState::Downloading if ready=>"ready",api::TorrentState::Downloading=>"downloading",
        api::TorrentState::Stalled=>"stalled",api::TorrentState::Queued=>"queued",api::TorrentState::Paused=>"paused",
        api::TorrentState::Done=>"done",api::TorrentState::Seeding=>"seeding",
        api::TorrentState::WaitingVpn=>"waiting_vpn",api::TorrentState::Error=>"error",
    }
}
pub(crate) fn cell(key:&str,reason:Option<&str>)->&'static str {
    if key=="error" && reason==Some("missing_files") {return "! Missing";}
    definitions().iter().find(|s|s.definition.state==key).map_or("? Unknown",|s|s.padded.as_str())
}
pub(crate) fn progress(value:Option<f64>)->String {
    value.map_or_else(||"—".into(),|value|format!("{:.0}%",value*100.))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_progress_stays_distinct_from_zero_and_known_progress_rounds_once() {
        assert_eq!(progress(None),"—");assert_eq!(progress(Some(0.)),"0%");
        assert_eq!(progress(Some(0.439)),"44%");assert_eq!(progress(Some(1.)),"100%");
    }
}
