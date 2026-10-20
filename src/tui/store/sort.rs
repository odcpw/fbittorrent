//! Compare typed values; unknowns remain last in either direction.
use super::*;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::tui) enum TorrentSort {
    Name,
    Size,
    Progress,
    DownloadRate,
    Added,
    State,
    Eta,
    Peers,
}
impl Store {
    /// Build only when data or sort selection changes, never from the frame renderer.
    pub fn sorted_torrents(&self, field: TorrentSort, descending: bool) -> Vec<&api::Torrent> {
        let mut rows: Vec<_> = self.torrents.values().collect();
        rows.sort_by(|left, right| torrent_order(left, right, field, descending));
        rows
    }
}
pub(in crate::tui) fn torrent_order(left: &api::Torrent, right: &api::Torrent, field: TorrentSort, descending: bool) -> Ordering {
    let order = match field {
        TorrentSort::Name => direction(left.name.cmp(&right.name), descending),
        TorrentSort::Size => optional(left.size, right.size, descending, u64::cmp),
        TorrentSort::Progress => optional(left.progress, right.progress, descending, f64::total_cmp),
        TorrentSort::DownloadRate => optional(left.dl_bps, right.dl_bps, descending, u64::cmp),
        TorrentSort::Added => direction(left.added_at.cmp(&right.added_at), descending),
        TorrentSort::State => direction(state_rank(left.state).cmp(&state_rank(right.state)),descending),
        TorrentSort::Eta => optional(left.eta_s,right.eta_s,descending,u64::cmp),
        TorrentSort::Peers => optional(left.peers,right.peers,descending,u64::cmp),
    };
    order.then_with(|| left.hash.cmp(&right.hash))
}
fn state_rank(state:api::TorrentState)->u8 {
    match state {api::TorrentState::Metadata=>0,api::TorrentState::Checking=>1,api::TorrentState::Downloading=>2,
        api::TorrentState::Stalled=>3,api::TorrentState::Queued=>4,api::TorrentState::Paused=>5,api::TorrentState::Done=>6,
        api::TorrentState::Seeding=>7,api::TorrentState::WaitingVpn=>8,api::TorrentState::Error=>9}
}
pub(super) fn direction(value: Ordering, descending: bool) -> Ordering {
    if descending {
        value.reverse()
    } else {
        value
    }
}
pub(super) fn optional<T>(
    left: Option<T>,
    right: Option<T>,
    descending: bool,
    compare: impl Fn(&T, &T) -> Ordering,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => direction(compare(&left, &right), descending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bytes_sort_numerically_and_unknown_is_last_in_either_direction() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("contracts/ipc/v1/torrents.ndjson"),
        )
        .unwrap();
        let frame: Value = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|line| line["frame"]["result"]["torrents"].is_array())
            .unwrap();
        let mut template: api::Torrent =
            serde_json::from_value(frame["frame"]["result"]["torrents"][0].clone()).unwrap();
        let mut store = Store::default();
        for (index, size) in [Some(1_181_116_006), Some(943_718_400), None]
            .into_iter()
            .enumerate()
        {
            template.hash = api::Hash::new(format!("{index:040x}")).unwrap();
            template.size = size;
            store
                .torrents
                .insert(template.hash.to_string(), template.clone());
        }
        let ascending = store.sorted_torrents(TorrentSort::Size, false);
        assert_eq!(
            ascending.iter().map(|row| row.size).collect::<Vec<_>>(),
            [Some(943_718_400), Some(1_181_116_006), None]
        );
        let descending = store.sorted_torrents(TorrentSort::Size, true);
        assert_eq!(
            descending.iter().map(|row| row.size).collect::<Vec<_>>(),
            [Some(1_181_116_006), Some(943_718_400), None]
        );
    }
}
