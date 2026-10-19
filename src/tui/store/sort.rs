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
        rows.sort_by(|left, right| {
            let order = match field {
                TorrentSort::Name => direction(left.name.cmp(&right.name), descending),
                TorrentSort::Size => optional(left.size, right.size, descending, u64::cmp),
                TorrentSort::Progress => {
                    optional(left.progress, right.progress, descending, f64::total_cmp)
                }
                TorrentSort::DownloadRate => {
                    optional(left.dl_bps, right.dl_bps, descending, u64::cmp)
                }
                TorrentSort::Added => direction(left.added_at.cmp(&right.added_at), descending),
                TorrentSort::State => direction(state_rank(left.state).cmp(&state_rank(right.state)),descending),
                TorrentSort::Eta => optional(left.eta_s,right.eta_s,descending,u64::cmp),
                TorrentSort::Peers => optional(left.peers,right.peers,descending,u64::cmp),
            };
            order.then_with(|| left.hash.cmp(&right.hash))
        });
        rows
    }
    pub fn sorted_files(&self, hash: &api::Hash) -> Vec<&api::File> {
        let Some(table) = self.files.get(hash) else {
            return Vec::new();
        };
        let mut rows: Vec<_> = table.values().collect();
        rows.sort_by(|left, right| {
            left.sort_key
                .cmp(&right.sort_key)
                .then(left.index.cmp(&right.index))
        });
        rows
    }
}
fn state_rank(state:api::TorrentState)->u8 {
    match state {api::TorrentState::Metadata=>0,api::TorrentState::Checking=>1,api::TorrentState::Downloading=>2,
        api::TorrentState::Stalled=>3,api::TorrentState::Queued=>4,api::TorrentState::Paused=>5,api::TorrentState::Done=>6,
        api::TorrentState::Seeding=>7,api::TorrentState::WaitingVpn=>8,api::TorrentState::Error=>9}
}
pub(in crate::tui) fn display_order(left:&crate::qbt::Torrent,right:&crate::qbt::Torrent,field:TorrentSort,descending:bool)->Ordering {
    let state=|t:&crate::qbt::Torrent|match t.state.as_str(){"metaDL"|"forcedMetaDL"=>0,"checkingDL"|"checkingUP"|"checkingResumeData"=>1,
        "downloading"|"forcedDL"=>2,"stalledDL"=>3,"queuedDL"|"queuedUP"=>4,"stoppedDL"|"pausedDL"=>5,
        "stoppedUP"|"pausedUP"=>6,"uploading"|"stalledUP"|"forcedUP"=>7,"waitingVPN"=>8,_=>9};
    let order=match field {
        TorrentSort::Name=>direction(left.name.cmp(&right.name),descending),
        TorrentSort::Size=>optional((left.progress_known||left.size>0).then_some(left.size),(right.progress_known||right.size>0).then_some(right.size),descending,i64::cmp),
        TorrentSort::Progress=>optional(left.progress_known.then_some(left.progress),right.progress_known.then_some(right.progress),descending,f64::total_cmp),
        TorrentSort::DownloadRate=>optional(left.is_active().then_some(left.dlspeed),right.is_active().then_some(right.dlspeed),descending,i64::cmp),
        TorrentSort::Added=>direction(left.added_on.cmp(&right.added_on),descending),
        TorrentSort::State=>direction(state(left).cmp(&state(right)),descending),
        TorrentSort::Eta=>optional((left.is_active()&&left.eta<8_640_000).then_some(left.eta),(right.is_active()&&right.eta<8_640_000).then_some(right.eta),descending,i64::cmp),
        TorrentSort::Peers=>optional(left.is_active().then_some(left.num_leechs.saturating_add(left.num_seeds)),right.is_active().then_some(right.num_leechs.saturating_add(right.num_seeds)),descending,i64::cmp),
    };order.then_with(||left.hash.cmp(&right.hash))
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
    #[test]
    fn file_order_uses_the_daemons_natural_sort_key() {
        let hash = api::Hash::new("a".repeat(40)).unwrap();
        let mut store = Store::default();
        let files = (0..3)
            .map(|index| {
                let episode = [10, 2, 1][index];
                (
                    index.to_string(),
                    api::File {
                        index: index as u64,
                        path: format!("e{episode}.mkv"),
                        size: 1,
                        progress: 0.,
                        included: true,
                        sort_key: format!("e{episode:010}.mkv"),
                        ready: None,
                    },
                )
            })
            .collect();
        store.files.insert(hash.clone(), files);
        assert_eq!(
            store
                .sorted_files(&hash)
                .iter()
                .map(|row| row.path.as_str())
                .collect::<Vec<_>>(),
            ["e1.mkv", "e2.mkv", "e10.mkv"]
        );
    }
}
