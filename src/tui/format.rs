//! Adapter to the current screen rows. The confirmed mirror retains optional values.
use super::*;
pub(super) mod state;
pub(super) mod columns;
pub(super) use columns::Columns;
pub(super) use state::{state_cell,state_word,reason};
use crate::api;
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
pub(super) fn file(row: &api::File) -> TorrentFile {
    TorrentFile {
        index: signed(row.index),
        name: row.path.clone(),
        size: signed(row.size),
        progress: row.progress,
        progress_known: true,
        priority: i64::from(row.included),
    }
}
pub(super) fn files<'a>(rows:impl IntoIterator<Item=&'a api::File>)->Vec<TorrentFile> {
    let mut rows=rows.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a,b|a.sort_key.cmp(&b.sort_key).then(a.index.cmp(&b.index)));
    rows.into_iter().map(file).collect()
}
pub(super) fn transfer(
    row: &api::TransferInfo,
    network: Option<&api::NetworkState>,
) -> TransferInfo {
    TransferInfo {
        dl_info_speed: row.dl_bps.map(signed).unwrap_or_default(),
        up_info_speed: row.ul_bps.map(signed).unwrap_or_default(),
        dl_info_data: signed(row.session_downloaded),
        up_info_data: signed(row.session_uploaded),
        connection_status: if network.is_some_and(|network| !network.ready) {
            "vpn_waiting"
        } else {
            "connected"
        }
        .into(),
    }
}

/// Display text for one torrent row's known values; unknown stays unknown.
pub(super) struct Metrics<'a>(pub &'a api::Torrent);
impl Metrics<'_> {
    pub fn progress(&self) -> String {
        let row = self.0;
        if row.reason.as_deref()==Some("missing_files") { return "Files missing".into(); }
        row.progress.map_or_else(
            || {
                if row.state == api::TorrentState::WaitingVpn {
                    "checked when the VPN connects".into()
                } else {
                    "waiting for metadata or verification".into()
                }
            },
            |progress| format!("{:.1}%  {} of {}", progress * 100., bytes(row.completed_bytes), bytes(row.size)),
        )
    }
    pub fn speed(&self) -> String {
        format!("↓ {}   ↑ {}", rate(self.0.dl_bps), rate(self.0.ul_bps))
    }
    pub fn eta(&self) -> String {
        self.0.eta_s.map_or_else(|| "—".into(), |seconds| human_eta(signed(seconds)))
    }
    pub fn peers(&self) -> String {
        let row = self.0;
        row.peers.map_or_else(
            || "unknown".into(),
            |peers| row.seeds.map_or_else(||format!("{peers} peers connected"),|seeds|format!("{seeds} seeds / {peers} peers connected")),
        )
    }
    pub fn ratio(&self) -> String {
        self.0.ratio.map_or_else(|| "—".into(), |ratio| format!("{ratio:.2}"))
    }
    pub fn upload(&self) -> String {
        let mut text=format!("Upload: {}",if self.0.upload_enabled {"on"}else{"off"});
        if let Some(limit)=self.0.upload_limit_bps.filter(|limit|*limit>0) {
            text.push_str(&format!(" · {} upload cap",rate(Some(limit))));
        }
        text
    }
}
/// States in which a torrent exchanges data or looks for peers.
pub(super) fn transferring(state: api::TorrentState) -> bool {
    use api::TorrentState::*;
    matches!(state, Metadata | Checking | Downloading | Stalled | Seeding)
}
pub(super) fn rate(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |value| human_rate(signed(value)))
}
pub(super) fn bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |value| human_bytes(signed(value)))
}
