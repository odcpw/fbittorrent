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
                if field==TorrentSort::Added {"added".into()}else{column.label().to_lowercase()},
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

impl Column {
    pub(in crate::tui) fn search_field(self) -> Option<store::SearchSort> {
        use store::SearchSort as S;
        Some(match self {
            Self::Name => S::Name,
            Self::Size => S::Size,
            Self::Seeds => S::Seeds,
            Self::Rating => S::Rating,
            Self::Uploaded => S::Uploaded,
            Self::Provider => S::Provider,
            Self::Peers => S::Peers,
            _ => return None,
        })
    }
}
impl TuiApp {
    pub(in crate::tui) fn sort_search_column(&mut self, column: Column) -> Cmd<Msg> {
        let Some(field) = column.search_field() else {
            return Cmd::none();
        };
        let selected = self.selected_result().map(|r| r.file_url.clone());
        self.search_descending = if self.search_sort == field {
            !self.search_descending
        } else {
            !matches!(field, store::SearchSort::Name | store::SearchSort::Provider)
        };
        self.search_sort = field;
        if !self.search_wire.is_empty() {
            self.search_wire
                .sort_by(|a, b| store::search_order(a, b, field, self.search_descending));
            self.search_results = self
                .search_wire
                .iter()
                .take(MAX_RESULTS)
                .map(remote::search::display)
                .collect();
        } else {
            let mut rows = std::mem::take(&mut self.search_results);
            rows.sort_by(|a, b| {
                store::legacy_search_order(
                    a,
                    b,
                    (self.rating_for(a), self.rating_for(b)),
                    field,
                    self.search_descending,
                )
            });
            self.search_results = rows;
        }
        self.search_list.borrow_mut().select(
            selected.and_then(|url| self.search_results.iter().position(|r| r.file_url == url)),
        );
        self.set_status(
            format!(
                "Sorted by {} {}",
                column.label().to_lowercase(),
                if self.search_descending { "↓" } else { "↑" }
            ),
            false,
        );
        Cmd::none()
    }
}
