//! Sort the confirmed values once per user action/data update, outside render.
use super::*;
use crate::tui::store::TorrentSort;
impl TuiApp {
    pub(in crate::tui) fn sort_torrent_column(&mut self, column: Column) -> Cmd<Msg> {
        let Some(field) = column.sort_field() else {
            return Cmd::none();
        };
        let descending=if self.torrent_sort==field { !self.torrent_descending } else { !matches!(field,TorrentSort::Name|TorrentSort::State) };
        self.apply_torrent_sort(field,descending,column.label())
    }
    pub(in crate::tui) fn cycle_torrent_sort(&mut self, reverse: bool) -> Cmd<Msg> {
        use TorrentSort::*;
        let fields=[Added,Name,Progress,DownloadRate,Size,State,Eta];
        let field=if reverse {self.torrent_sort} else {
            fields[(fields.iter().position(|field|*field==self.torrent_sort).unwrap_or(0)+1)%fields.len()]
        };
        let label=match field {Added=>"added",Name=>"name",Progress=>"progress",DownloadRate=>"download",Size=>"size",State=>"state",Eta=>"eta",Peers=>"peers"};
        self.apply_torrent_sort(field,if reverse {!self.torrent_descending} else {!matches!(field,Name|State)},label)
    }
    fn apply_torrent_sort(&mut self, field: TorrentSort, descending: bool, label: &str) -> Cmd<Msg> {
        let selected = self.selected_torrent().map(|t| t.hash.clone());
        self.torrent_sort=field;self.torrent_descending=descending;
        if !self.store.torrents.is_empty() {
            self.torrents = self
                .store
                .sorted_torrents(field, self.torrent_descending)
                .into_iter()
                .map(format::torrent)
                .collect();
        } else {
            self.torrents.sort_by(|left, right| {
                store::display_order(left, right, field, self.torrent_descending)
            });
        }
        self.tor_list
            .borrow_mut()
            .select(selected.and_then(|hash| self.torrents.iter().position(|t| t.hash == hash)));
        self.set_status(
            format!(
                "Sorted by {} {}",
                label.to_lowercase(),
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
                store::display_search_order(
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
