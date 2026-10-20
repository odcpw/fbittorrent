//! Active work keeps background observation alive; the engine owns rate history.
use super::*;

// Active non-video work still needs disk monitoring and completion updates,
// even without clients. Only the idle/paused table can remain untouched.
pub(super) fn active_work(rows: &[Torrent]) -> bool {
    rows.iter().any(|row| !row.staged && match row.state {
        TorrentState::Metadata | TorrentState::Checking | TorrentState::Downloading
        | TorrentState::Stalled | TorrentState::Seeding => true,
        TorrentState::Queued | TorrentState::Paused | TorrentState::Done
        | TorrentState::WaitingVpn | TorrentState::Error => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_non_video_work_retains_background_monitoring() {
        let value = include_str!("../../../contracts/ipc/v1/torrents.ndjson").lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|line| line.pointer("/frame/result/torrents/0").cloned()).unwrap();
        let mut row: Torrent = serde_json::from_value(value).unwrap();
        for (state, active) in [(TorrentState::Metadata,true),(TorrentState::Checking,true),
            (TorrentState::Downloading,true),(TorrentState::Stalled,true),(TorrentState::Seeding,true),
            (TorrentState::Queued,false),(TorrentState::Paused,false),(TorrentState::Done,false),
            (TorrentState::WaitingVpn,false),(TorrentState::Error,false)] {
            row.state = state;
            assert_eq!(active_work(std::slice::from_ref(&row)),active,"{state:?}");
            row.staged = true;
            assert!(!active_work(std::slice::from_ref(&row)));
            row.staged = false;
        }
    }
}
