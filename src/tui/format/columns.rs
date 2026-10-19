//! Compact typed values used by the responsive torrent columns.
use super::*;

pub(in crate::tui) fn compact_rate(rate: u64) -> String {
    match rate {
        n if n>=1_000_000_000=>format!("{:.1}G",n as f64/1_000_000_000.),
        n if n>=1_000_000=>format!("{:.1}M",n as f64/1_000_000.),
        n if n>=1_000=>format!("{:.0}K",n as f64/1_000.),
        n=>n.to_string(),
    }
}
pub(in crate::tui) fn compact_eta(seconds: u64) -> String {
    match seconds { n if n>=86400=>format!("{}d",n/86400),n if n>=3600=>format!("{}h",n/3600),n if n>=60=>format!("{}m",n/60),n=>format!("{n}s") }
}
pub(in crate::tui) struct Columns {
    pub size: String, pub progress: String, pub down: String, pub eta: String, pub peers: String,
}
impl Columns {
    pub fn of(t: &Torrent, row: Option<&api::Torrent>) -> Self {
        let progress=row.map_or_else(||t.progress_known.then_some(t.progress),|r|r.progress);
        let size=row.map_or_else(||(t.progress_known||t.size>0).then_some(t.size.max(0) as u64),|r|r.size);
        let down=row.map_or_else(||(t.is_active()&&t.progress_known).then_some(t.dlspeed.max(0) as u64),|r|r.dl_bps);
        let eta=row.map_or_else(||(t.is_active()&&t.eta>0&&t.eta<8_640_000).then_some(t.eta as u64),|r|r.eta_s);
        let peers=row.map_or_else(||(t.is_active()||t.is_seeding()).then_some(t.num_leechs.max(0) as u64),|r|r.peers);
        Self {
            size:size.map_or_else(||"—".into(),|v|human_bytes(signed(v))),
            progress:progress.map_or_else(||"—".into(),|v|format!("{:.0}%",v*100.)),
            down:down.map_or_else(String::new,compact_rate),
            eta:if down.is_some(){eta.map_or_else(String::new,compact_eta)}else{String::new()},
            peers:peers.map_or_else(||if t.is_active()||t.is_seeding(){"—".into()}else{String::new()},|p|p.to_string()),
        }
    }
    pub fn tail(&self, extended:bool)->String {
        if extended { format!(" {:>9} {:>5} {:>6} {:>6} {:>5}",self.size,self.progress,self.down,self.eta,self.peers) }
        else {format!(" {:>5} {:>6} {:>6}",self.progress,self.down,self.eta)}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_does_not_fabricate_transfer_numbers() {
        let frame:serde_json::Value=include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines()
            .map(|line|serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|line|line["frame"]["result"]["torrents"].is_array()).unwrap();
        let rows:Vec<api::Torrent>=serde_json::from_value(frame["frame"]["result"]["torrents"].clone()).unwrap();
        let row=rows.iter().find(|r|r.state==api::TorrentState::Metadata).unwrap();
        let legacy=torrent(row);let cells=Columns::of(&legacy,Some(row));
        assert_eq!(cells.size,"—");assert_eq!(cells.progress,"—");assert_eq!(cells.down,"");assert_eq!(cells.eta,"");
        assert_eq!(compact_rate(3_100_000),"3.1M");assert_eq!(compact_eta(720),"12m");
    }
}
