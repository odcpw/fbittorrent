//! Sort the confirmed values once per user action/data update, outside render.
use super::*;
use crate::tui::store::TorrentSort;
impl TuiApp {
    pub(in crate::tui) fn sort_torrent_column(&mut self, column: Column) -> Cmd<Msg> {
        let Some(field) = column.sort_field() else {
            return Cmd::none();
        };
        let selected = self.selected_torrent().map(|t| t.hash.clone());
        self.torrent_descending = if self.torrent_sort == field {
            !self.torrent_descending
        } else {
            !matches!(field, TorrentSort::Name | TorrentSort::State)
        };
        self.torrent_sort = field;
        if self.cfg.native() && !self.store.torrents.is_empty() {
            self.torrents = self
                .store
                .sorted_torrents(field, self.torrent_descending)
                .into_iter()
                .map(format::torrent)
                .collect();
        } else {
            self.torrents.sort_by(|left, right| {
                store::legacy_order(left, right, field, self.torrent_descending)
            });
        }
        self.tor_list
            .borrow_mut()
            .select(selected.and_then(|hash| self.torrents.iter().position(|t| t.hash == hash)));
        self.set_status(
            format!(
                "Sorted by {} {}",
                column.label().to_lowercase(),
                if self.torrent_descending {
                    "↓"
                } else {
                    "↑"
                }
            ),
            false,
        );
        Cmd::none()
    }
}
