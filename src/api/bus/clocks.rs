//! Global history has its own clock; idle views do not need rebuilding for it.
use super::*;

impl Bus {
    pub(super) fn clocks_due(&self, active_work: bool) -> bool {
        let now = Instant::now();
        let history_due = self.daemon.history.lock().unwrap().due(now);
        if history_due {
            let rates = {
                let slot = self.daemon.slot.read().unwrap_or_else(|e| e.into_inner());
                match &slot.slot {
                    Slot::Running(app) => app.dispatcher().transfer_rates(),
                    _ => (0, 0),
                }
            };
            self.daemon.history.lock().unwrap().record(now, rates.0, rates.1);
        }
        let watch = self.daemon.watch.lock().unwrap();
        (history_due && active_work) || (watch.needs_tick() && watch.due())
    }
}

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
