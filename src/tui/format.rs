//! Adapter to the current screen rows. The confirmed mirror retains optional values.
use super::*;
pub(super) mod state;
pub(super) mod columns;
pub(super) use columns::Columns;
pub(super) use state::{state_cell,state_word,STATE_WIDTH,reason};
use crate::api;
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
pub(super) fn torrent(row: &api::Torrent) -> Torrent {
    let state = match row.state {
        api::TorrentState::Metadata => "metaDL",
        api::TorrentState::Checking => "checkingDL",
        api::TorrentState::Downloading => "downloading",
        api::TorrentState::Stalled => "stalledDL",
        api::TorrentState::Queued => "queuedDL",
        api::TorrentState::Paused => "stoppedDL",
        api::TorrentState::Done => "stoppedUP",
        api::TorrentState::Seeding => "uploading",
        api::TorrentState::WaitingVpn => "waitingVPN",
        api::TorrentState::Error => "error",
    };
    Torrent {
        hash: row.hash.to_string(),
        staged: row.staged,
        name: row.name.clone(),
        state: state.into(),
        error_message: row.error.clone(),
        progress: row.progress.unwrap_or_default(),
        progress_known: row.progress.is_some(),
        size: row.size.map(signed).unwrap_or_default(),
        completed: row.completed_bytes.map(signed).unwrap_or_default(),
        dlspeed: row.dl_bps.map(signed).unwrap_or_default(),
        upspeed: row.ul_bps.map(signed).unwrap_or_default(),
        eta: row.eta_s.map(signed).unwrap_or(8_640_000),
        num_seeds: row.seeds.map(signed).unwrap_or_default(),
        num_leechs: row.peers.map(signed).unwrap_or_default(),
        ratio: row.ratio.unwrap_or_default(),
        seq_dl: true,
        f_l_piece_prio: true,
        save_path: row.save_path.clone(),
        content_path: row.content_path.clone().unwrap_or_default(),
        category: String::new(),
        added_on: signed(row.added_at),
        completion_on: row.completed_at.map(signed).unwrap_or_default(),
        up_limit: row
            .upload_limit_bps
            .map(signed)
            .unwrap_or(if row.seed { 0 } else { 32768 }),
        ratio_limit: if row.seed { -1.0 } else { 0.0 },
        tags: if row.seed {
            "seed".into()
        } else {
            String::new()
        },
    }
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

pub(super) struct Metrics<'a> {
    pub legacy: &'a Torrent,
    pub api: Option<&'a api::Torrent>,
}
impl Metrics<'_> {
    pub fn progress(&self) -> String {
        if let Some(row) = self.api {
            return row.progress.map_or_else(
                || {
                    if row.state == api::TorrentState::WaitingVpn {
                        "checked when the VPN connects".into()
                    } else {
                        "waiting for metadata or verification".into()
                    }
                },
                |progress| {
                    format!(
                        "{:.1}%  {} of {}",
                        progress * 100.,
                        bytes(row.completed_bytes),
                        bytes(row.size)
                    )
                },
            );
        }
        let row = self.legacy;
        if row.progress_known {
            format!(
                "{:.1}%  {} of {}",
                row.progress * 100.,
                human_bytes(row.completed),
                human_bytes(row.size)
            )
        } else if row.state == "waitingVPN" {
            "checked when the VPN connects".into()
        } else {
            "waiting for metadata or verification".into()
        }
    }
    pub fn speed(&self) -> String {
        self.api.map_or_else(
            || {
                format!(
                    "↓ {}   ↑ {}",
                    human_rate(self.legacy.dlspeed),
                    human_rate(self.legacy.upspeed)
                )
            },
            |row| format!("↓ {}   ↑ {}", rate(row.dl_bps), rate(row.ul_bps)),
        )
    }
    pub fn eta(&self) -> String {
        self.api.map_or_else(
            || human_eta(self.legacy.eta),
            |row| {
                row.eta_s
                    .map_or_else(|| "—".into(), |seconds| human_eta(signed(seconds)))
            },
        )
    }
    pub fn peers(&self, _native: bool) -> String {
        self.api.map_or_else(
            || {
                format!(
                    "{} peers connected",
                    self.legacy.num_leechs
                )
            },
            |row| {
                row.peers.map_or_else(
                    || "unknown".into(),
                    |peers| row.seeds.map_or_else(||format!("{peers} peers connected"),|seeds|format!("{seeds} seeds / {peers} peers connected")),
                )
            },
        )
    }
    pub fn ratio(&self) -> String {
        self.api.map_or_else(
            || format!("{:.2}", self.legacy.ratio),
            |row| {
                row.ratio
                    .map_or_else(|| "—".into(), |ratio| format!("{ratio:.2}"))
            },
        )
    }
    pub fn upload(&self) -> String {
        let seed=self.api.map_or_else(||self.legacy.seed_allowed(),|row|row.seed);
        let mut text=format!("Upload: {}",if seed {"on"}else{"off"});
        if let Some(limit)=self.api.and_then(|row|row.upload_limit_bps) {
            text.push_str(&format!(" · {} upload cap",rate(Some(limit))));
        }
        text
    }

}
pub(super) fn rate(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |value| human_rate(signed(value)))
}
pub(super) fn bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |value| human_bytes(signed(value)))
}
