//! Pure Summary counters shared with ledger/policy characterization tests.
use crate::api::{Counts, Torrent, TorrentState};

pub(crate) fn counts(rows: &[Torrent]) -> Counts {
    let mut counts = Counts {
        total: 0,
        downloading: 0,
        stalled: 0,
        metadata: 0,
        checking: 0,
        queued: 0,
        paused: 0,
        done: 0,
        seeding: 0,
        waiting_vpn: 0,
        error: 0,
        staged: 0,
        ready: 0,
    };
    for row in rows {
        if row.staged {
            counts.staged += 1;
            continue;
        }
        counts.total += 1;
        match row.state {
            TorrentState::Metadata => counts.metadata += 1,
            TorrentState::Checking => counts.checking += 1,
            TorrentState::Downloading => counts.downloading += 1,
            TorrentState::Stalled => counts.stalled += 1,
            TorrentState::Queued => counts.queued += 1,
            TorrentState::Paused => counts.paused += 1,
            TorrentState::Done => counts.done += 1,
            TorrentState::Seeding => counts.seeding += 1,
            TorrentState::WaitingVpn => counts.waiting_vpn += 1,
            TorrentState::Error => counts.error += 1,
        }
        counts.ready += u64::from(row.playable.as_ref().is_some_and(|value| value.ready));
    }
    counts
}
