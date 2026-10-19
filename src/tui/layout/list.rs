//! A shared, cached Table viewport for every non-torrent list.
use super::*;
use ftui::widgets::{
    scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState},
    table::{Row, Table, TableState},
};
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub(in crate::tui) struct ListTable {
    table: Table<'static>,
    state: TableState,
    signature: Option<u64>,
    #[cfg(test)]
    pub builds: usize,
}
/// Streaming serialization hashes typed fields without constructing JSON buffers.
/// Visible rows only are visited; a row's owned formatting happens on cache misses.
pub(in crate::tui) fn fingerprint(value: &impl serde::Serialize) -> u64 {
    struct Sink(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink(Default::default());
    serde_json::to_writer(&mut sink, value).expect("UI row serialization is infallible");
    sink.0.finish()
}
impl TuiApp {
    pub(in crate::tui) fn view_rows(
        &self,
        list: MouseList,
        area: Rect,
        count: usize,
        fingerprint: impl Fn(usize) -> u64,
        row: impl Fn(usize) -> Line<'static>,
        frame: &mut Frame,
    ) {
        if area.is_empty() {
            return;
        }
        let stale = self.recovery.active && self.store.stale && !matches!(list,MouseList::Browse|MouseList::Settings);
        let state = self.mouse_list_state(list);
        let range = visible_range(&mut state.borrow_mut(), count, area.height);
        let selected = state.borrow().selected();
        let hovered = state.borrow().hovered;
        let mut signature = std::collections::hash_map::DefaultHasher::new();
        (
            area.width,
            area.height,
            count,
            range.start,
            range.end,
            selected,
            hovered,
            stale,
        )
            .hash(&mut signature);
        [
            self.theme.fg.0,
            self.theme.dim.0,
            self.theme.accent.0,
            self.theme.selection.0,
            self.theme.red.0,
            self.theme.green.0,
            self.theme.yellow.0,
        ]
        .hash(&mut signature);
        self.theme_mtime.hash(&mut signature);
        (self.theme.low_chroma,self.theme.accent_text.0).hash(&mut signature);
        if hovered.is_some() {
            self.marquee_elapsed()
                .map(|t| t.as_millis() / 125)
                .hash(&mut signature);
        }
        for index in range.clone() {
            fingerprint(index).hash(&mut signature);
        }
        let signature = signature.finish();
        let mut caches = self.list_tables.borrow_mut();
        let cache = &mut caches[list as usize];
        if cache.signature != Some(signature) {
            let rows = range.clone().map(|i| {
                Row::new([
                    Line::raw(if selected == Some(i) { "▌ " } else { "  " }),
                    if stale {Line::from_spans(row(i).into_iter().map(|mut span| {span.style=Some(span.style.unwrap_or_default().fg(self.theme.dim).dim());span}))} else {row(i)},
                ])
            });
            let modal = matches!(list, MouseList::Files | MouseList::Providers);
            let style = if modal {
                self.theme.text().bg(self.theme.selection)
            } else {
                self.theme.text()
            };
            let highlight = if modal {
                self.theme
                    .selected()
                    .bg(self.theme.accent)
                    .fg(self.theme.selection)
            } else {
                self.theme.selected()
            };
            cache.table = Table::new(rows, [Constraint::Fixed(2), Constraint::Fill])
                .column_spacing(0)
                .style(style)
                .highlight_style(highlight)
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
        let ListTable { table, state, .. } = cache;
        StatefulWidget::render(table, area, frame, state);
        self.rects.borrow_mut().lists[list as usize] = Some(area);
        if count > usize::from(area.height) {
            let mut state = ScrollbarState::new(count, range.start, usize::from(area.height));
            StatefulWidget::render(
                &Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(self.theme.dimmed())
                    .track_style(self.theme.border()),
                Rect::new(area.right()+u16::from(matches!(list,MouseList::Files|MouseList::Providers)), area.y, 1, area.height),
                frame,
                &mut state,
            );
        }
    }
}
