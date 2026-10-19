//! Visible torrent rows only; unchanged frames reuse owned cell formatting.
use super::*;
use ftui::widgets::{
    scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState},
    table::{Row, Table, TableState},
    virtualized::Virtualized,
};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::tui) enum Column {
    Marker,
    State,
    Name,
    Size,
    Progress,
    Download,
    Eta,
    Peers,
}
impl Column {
    pub fn sort_field(self) -> Option<store::TorrentSort> {
        Some(match self {
            Self::Marker => return None,
            Self::State => store::TorrentSort::State,
            Self::Name => store::TorrentSort::Name,
            Self::Size => store::TorrentSort::Size,
            Self::Progress => store::TorrentSort::Progress,
            Self::Download => store::TorrentSort::DownloadRate,
            Self::Eta => store::TorrentSort::Eta,
            Self::Peers => store::TorrentSort::Peers,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Marker => "",
            Self::State => "STATE",
            Self::Name => "NAME",
            Self::Size => "SIZE",
            Self::Progress => "%",
            Self::Download => "↓",
            Self::Eta => "ETA",
            Self::Peers => "PEERS",
        }
    }
}
#[derive(Default)]
pub(in crate::tui) struct Rects {
    pub rows: Vec<(String, Rect)>,
    pub headers: Vec<(Column, Rect)>,
    pub tabs: [Rect; 4],
    pub footer: Rect,
    pub table: Option<Rect>,
}
impl Rects {
    pub fn begin(&mut self) {
        self.headers.clear();
        self.table = None;
    }
    fn rows(&mut self, app: &TuiApp, range: std::ops::Range<usize>, area: Rect) {
        self.table = Some(area);
        let count = range.len();
        for (line, index) in range.enumerate() {
            let rect = Rect::new(area.x, area.y + line as u16, area.width, 1);
            if let Some((hash, old)) = self.rows.get_mut(line) {
                hash.clone_from(&app.torrents[index].hash);
                *old = rect;
            } else {
                self.rows.push((app.torrents[index].hash.clone(), rect));
            }
        }
        self.rows.truncate(count);
    }
}
#[derive(Default)]
pub(in crate::tui) struct TorrentTable {
    table: Table<'static>,
    state: TableState,
    signature: Option<u64>,
    #[cfg(test)]
    pub builds: usize,
}

pub(in crate::tui) fn visible_range(
    state: &mut ListState,
    count: usize,
    height: u16,
) -> std::ops::Range<usize> {
    if height == 0 || count == 0 {
        return 0..0;
    }
    let height = usize::from(height);
    let mut offset = state.offset.min(count.saturating_sub(height));
    if let Some(selected) = state.selected().filter(|i| *i < count) {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    state.offset = offset;
    let mut virtualized = Virtualized::<()>::external(count, 0)
        .with_fixed_height(1)
        .with_overscan(0);
    virtualized.scroll_to(offset);
    virtualized.visible_range(height as u16)
}

struct ColumnSet {
    values: [(Column, u16); 8],
    count: usize,
}
impl ColumnSet {
    fn iter(&self) -> std::slice::Iter<'_, (Column, u16)> {
        self.values[..self.count].iter()
    }
}
fn columns(width: u16, tier: Tier) -> ColumnSet {
    let mut values = if tier.extended() {
        [
            (Column::Marker, 1),
            (Column::State, 9),
            (Column::Name, 0),
            (Column::Size, 9),
            (Column::Progress, 5),
            (Column::Download, 6),
            (Column::Eta, 6),
            (Column::Peers, 5),
        ]
    } else {
        [
            (Column::Marker, 1),
            (Column::State, 9),
            (Column::Name, 0),
            (Column::Progress, 5),
            (Column::Download, 6),
            (Column::Eta, 6),
            (Column::Eta, 0),
            (Column::Peers, 0),
        ]
    };
    let count = if width < 32 {
        3
    } else if tier.extended() {
        8
    } else {
        6
    };
    let fixed = values[..count].iter().map(|(_, w)| *w).sum::<u16>() + (count - 1) as u16;
    values[2].1 = width.saturating_sub(fixed);
    ColumnSet { values, count }
}
fn content(text: String, width: u16, right: bool, style: Style) -> Text<'static> {
    let text = if right {
        format!("{:>width$}", text, width = width as usize)
    } else {
        text
    };
    Text::from_line(Line::styled(text, style))
}
impl TuiApp {
    pub(in crate::tui) fn view_torrent_table(&self, area: Rect, frame: &mut Frame) {
        if area.height == 0 || area.width < 12 {
            return;
        }
        let body = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        let range = visible_range(
            &mut self.tor_list.borrow_mut(),
            self.torrents.len(),
            body.height,
        );
        let selected = self.tor_list.borrow().selected();
        let hovered = self.tor_list.borrow().hovered;
        let tier = Tier::at(frame.bounds().width);
        let stale = self.recovery.active && self.store.stale;
        let mut signature = std::collections::hash_map::DefaultHasher::new();
        (
            area.width,
            area.height,
            tier,
            range.start,
            range.end,
            selected,
            hovered,
            stale,
            self.theme_mtime,
        )
            .hash(&mut signature);
        self.theme.name.hash(&mut signature);
        (self.torrent_sort, self.torrent_descending).hash(&mut signature);
        [
            self.theme.fg.0,
            self.theme.dim.0,
            self.theme.accent.0,
            self.theme.selection.0,
            self.theme.red.0,
            self.theme.green.0,
        ]
        .hash(&mut signature);
        let marquee = self.marquee_elapsed();
        if hovered.is_some() {
            marquee.map(|d| d.as_millis() / 125).hash(&mut signature);
        }
        for index in range.clone() {
            let t = &self.torrents[index];
            (
                &t.hash,
                &t.name,
                &t.state,
                t.progress.to_bits(),
                t.progress_known,
                t.size,
                t.dlspeed,
                t.eta,
                t.num_leechs,
                t.num_seeds,
            )
                .hash(&mut signature);
            self.store.pending_for_key(&t.hash).hash(&mut signature);
            if let Some(row) = self.store.torrents.get(&t.hash) {
                (
                    row.size,
                    row.progress.map(f64::to_bits),
                    row.dl_bps,
                    row.eta_s,
                    row.peers,
                    row.reason.as_deref(),
                    row.playable.as_ref().map(|p| p.ready),
                )
                    .hash(&mut signature);
            }
        }
        let signature = signature.finish();
        let mut cache = self.torrent_table.borrow_mut();
        if cache.signature != Some(signature) {
            let geometry = columns(area.width, tier);
            let header = Row::new(geometry.iter().map(|(column, width)| {
                content(
                    if column.sort_field() == Some(self.torrent_sort) {
                        format!(
                            "{}{}",
                            column.label(),
                            if self.torrent_descending {
                                "↓"
                            } else {
                                "↑"
                            }
                        )
                    } else {
                        column.label().into()
                    },
                    *width,
                    !matches!(column, Column::State | Column::Name | Column::Marker),
                    if column.sort_field() == Some(self.torrent_sort) {
                        self.theme.accented()
                    } else {
                        self.theme.dimmed().bold()
                    },
                )
            }));
            let rows = range.clone().map(|index| {
                let t = &self.torrents[index];
                let row = self.store.torrents.get(&t.hash);
                let pending = self.store.pending_for_key(&t.hash);
                let style = if stale || pending.is_some() || t.is_stopped() {
                    self.theme.dimmed()
                } else {
                    self.theme.text()
                };
                let metrics = format::Columns::of(t, row);
                let name = if let Some(op) = pending {
                    format!("{}… {}", remote::mutation_verb(op), t.name)
                } else {
                    t.name.clone()
                };
                Row::new(geometry.iter().map(|(column, width)| {
                    let text = match column {
                        Column::Marker => {
                            if selected == Some(index) {
                                "▌".into()
                            } else {
                                String::new()
                            }
                        }
                        Column::State => format::state_cell(t, row).into(),
                        Column::Name => readable_cell(
                            &name,
                            *width as usize,
                            hovered_for(&self.tor_list, index, marquee),
                        ),
                        Column::Size => metrics.size.clone(),
                        Column::Progress => metrics.progress.clone(),
                        Column::Download => metrics.down.clone(),
                        Column::Eta => metrics.eta.clone(),
                        Column::Peers => metrics.peers.clone(),
                    };
                    let style = if *column == Column::State
                        && (matches!(row.map(|r| r.state), Some(crate::api::TorrentState::Error))
                            || matches!(t.state.as_str(), "error" | "missingFiles" | "disk_full"))
                        && !stale
                        && pending.is_none()
                    {
                        self.theme.err().bold()
                    } else {
                        style
                    };
                    content(
                        text,
                        *width,
                        !matches!(column, Column::Marker | Column::State | Column::Name),
                        style,
                    )
                }))
            });
            cache.table = Table::new(
                rows,
                geometry
                    .iter()
                    .enumerate()
                    .map(|(i, (_, w))| Constraint::Fixed(*w + u16::from(i + 1 < geometry.count))),
            )
            .column_spacing(0)
            .header(header)
            .style(self.theme.text())
            .highlight_style(self.theme.selected())
            .data_hash(signature);
            cache.signature = Some(signature);
            #[cfg(test)]
            {
                cache.builds += 1;
            }
        }
        cache.state.selected = selected.and_then(|i| range.contains(&i).then(|| i - range.start));
        cache.state.hovered = hovered.and_then(|i| range.contains(&i).then(|| i - range.start));
        cache.state.sort_column = None;
        cache.state.offset = 0;
        let TorrentTable { table, state, .. } = &mut *cache;
        StatefulWidget::render(table, area, frame, state);
        drop(cache);
        let mut rects = self.rects.borrow_mut();
        rects.rows(self, range.clone(), body);
        let mut x = area.x;
        for &(column, width) in columns(area.width, tier).iter() {
            if column != Column::Marker {
                rects.headers.push((column, Rect::new(x, area.y, width, 1)));
            }
            x = x.saturating_add(width + 1);
        }
        if self.torrents.len() > usize::from(body.height) {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(self.theme.dimmed())
                .track_style(self.theme.border());
            let mut state =
                ScrollbarState::new(self.torrents.len(), range.start, usize::from(body.height));
            StatefulWidget::render(
                &scrollbar,
                Rect::new(area.right(), body.y, 1, body.height),
                frame,
                &mut state,
            );
        }
    }
}
