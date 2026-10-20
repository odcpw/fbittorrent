//! Compact typed values used by the responsive torrent columns.
use super::*;

pub(in crate::tui) fn compact_rate(rate: u64) -> String {
    match rate {
        n if n >= 1_000_000_000 => format!("{:.1}G", n as f64 / 1_000_000_000.),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.),
        n if n >= 1_000 => format!("{:.0}K", n as f64 / 1_000.),
        n => n.to_string(),
    }
}
pub(in crate::tui) fn compact_eta(seconds: u64) -> String {
    match seconds {
        n if n >= 86400 => format!("{}d", n / 86400),
        n if n >= 3600 => format!("{}h", n / 3600),
        n if n >= 60 => format!("{}m", n / 60),
        n => format!("{n}s"),
    }
}
pub(in crate::tui) struct Columns {
    pub size: String,
    pub progress: String,
    pub down: String,
    pub eta: String,
    pub peers: String,
}
impl Columns {
    pub fn of(row: &api::Torrent) -> Self {
        let down = (row.state==api::TorrentState::Downloading).then_some(row.dl_bps).flatten();
        Self {
            size: row.size.map_or_else(|| "—".into(), |v| human_bytes(signed(v))),
            progress: crate::torrent_format::progress(row.progress),
            down: down.map_or_else(String::new, compact_rate),
            eta: if down.is_some() {
                row.eta_s.map_or_else(String::new, compact_eta)
            } else {
                String::new()
            },
            peers: row.peers.map_or_else(
                || if super::transferring(row.state) { "—".into() } else { String::new() },
                |p| row.seeds.map_or_else(||p.to_string(),|s|format!("{s}/{p}")),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_does_not_fabricate_transfer_numbers() {
        let frame: serde_json::Value = include_str!("../../../contracts/ipc/v1/torrents.ndjson")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|line| line["frame"]["result"]["torrents"].is_array())
            .unwrap();
        let rows: Vec<api::Torrent> =
            serde_json::from_value(frame["frame"]["result"]["torrents"].clone()).unwrap();
        let row = rows
            .iter()
            .find(|r| r.state == api::TorrentState::Metadata)
            .unwrap();
        let cells = Columns::of(row);
        assert_eq!(cells.size, "—");
        assert_eq!(cells.progress, "—");
        assert_eq!(cells.down, "");
        assert_eq!(cells.eta, "");
        assert_eq!(compact_rate(3_100_000), "3.1M");
        assert_eq!(compact_eta(720), "12m");
        for state in [api::TorrentState::Paused,api::TorrentState::Done,api::TorrentState::Seeding,api::TorrentState::Queued] {
            let mut row=row.clone();row.state=state;row.dl_bps=Some(0);row.eta_s=Some(0);
            let cells=Columns::of(&row);
            assert!(cells.down.is_empty()&&cells.eta.is_empty(),"inactive rate and ETA stay blank: {state:?}");
        }
    }
}
