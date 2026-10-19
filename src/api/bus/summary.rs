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

/// The glance-only bar receives current counts without subscribing to peers.
/// At most five rows are projected, and absent observations stay absent.
pub(super) fn top(rows: &[Torrent]) -> Vec<serde_json::Value> {
    use serde_json::json;
    rows.iter().filter(|row| !row.staged).take(5).map(|row| {
        let mut value=json!({"hash":row.hash,"name":row.name,"state":row.state});
        for (key,field) in [
            ("progress",json!(row.progress)),("dl_bps",json!(row.dl_bps)),
            ("eta_s",json!(row.eta_s)),("peers",json!(row.peers)),("seeds",json!(row.seeds)),
        ] {if !field.is_null() {value[key]=field;}}
        value
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glance_peer_counts_are_bounded_known_only_and_not_swarm_totals() {
        let mut row:Torrent=serde_json::from_str(include_str!("../fixtures/undo_detail.json")).unwrap();
        row.staged=false;row.peers=Some(2);row.seeds=Some(1);
        let mut unknown=row.clone();unknown.peers=None;unknown.seeds=None;
        let mut zero=row.clone();zero.peers=Some(0);zero.seeds=Some(0);
        let mut staged=row.clone();staged.staged=true;
        let rows=top(&[staged,row.clone(),unknown,zero,row.clone(),row.clone(),row]);
        assert_eq!(rows.len(),5);
        assert_eq!(rows[0]["peers"],2);assert_eq!(rows[0]["seeds"],1);
        assert!(rows[1].get("peers").is_none() && rows[1].get("seeds").is_none());
        assert_eq!(rows[2]["peers"],0);assert_eq!(rows[2]["seeds"],0);
    }
}
