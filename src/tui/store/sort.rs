//! Compare typed values; unknowns remain last in either direction.
use super::*;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy)]
pub(in crate::tui) enum TorrentSort {
    Name,
    Size,
    Progress,
    DownloadRate,
    Added,
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
fn direction(value: Ordering, descending: bool) -> Ordering {
    if descending {
        value.reverse()
    } else {
        value
    }
}
fn optional<T>(
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
