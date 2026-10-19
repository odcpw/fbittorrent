//! Existing layout behavior.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Tier { Narrow, Medium, Wide }
impl Tier {
    pub fn at(width: u16) -> Self {
        if width < 100 { Self::Narrow } else if width < 140 { Self::Medium } else { Self::Wide }
    }
    pub fn extended(self) -> bool { self != Self::Narrow }
}

impl TuiApp {
    pub(super) fn view_tabs(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let cols = Flex::horizontal().constraints([Constraint::Fill, Constraint::FitContent]).split(area);
        let new = self.inbox_rows.len();
        let shows_tab = format!(" 3 Shows ({new}) ");
        let labels = [" 1 Torrents ".to_string(), " 2 Search ".to_string(), shows_tab.clone(), " 4 RSS ".to_string()];
        let mut x = area.x;
        let regions = labels.map(|label| {
            // Tabs wraps each title with two visible bracket/space cells.
            let width = (dw(&label).saturating_add(2) as u16).min(cols[0].right().saturating_sub(x));
            let region = Rect::new(x, area.y, width, area.height);
            x = x.saturating_add(width);
            region
        });
        self.tab_rects.set(regions);
        let tabs = Tabs::new([Tab::new(" 1 Torrents "), Tab::new(" 2 Search "), Tab::new(shows_tab.as_str()), Tab::new(" 4 RSS ")])
            .style(th.dimmed())
            .active_style(th.selected())
            .separator("");
        StatefulWidget::render(&tabs, cols[0], frame, &mut self.tabs.borrow_mut());

        let right = if self.online {
            let mut spans = vec![
                Span::styled("↓ ", th.dimmed()), Span::styled(self.store.transfer.as_ref().map_or_else(||human_rate(self.transfer.dl_info_speed),|info|format::rate(info.dl_bps)), th.text()),
                Span::styled("  ↑ ", th.dimmed()), Span::styled(self.store.transfer.as_ref().map_or_else(||human_rate(self.transfer.up_info_speed),|info|format::rate(info.ul_bps)), th.text()),
                Span::styled(format!("  {} ", th.name), th.dimmed()),
            ];
            if self.poll_inflight { spans.insert(0, Span::styled(format!("{} ", SPINNER[self.tick % 4]), th.dimmed())); }
            Line::from_spans(spans)
        } else {
            Line::from_spans([Span::styled("● offline ", th.err())])
        };
        let w = right.width() as u16;
        let tabs_right = regions.iter().map(Rect::right).max().unwrap_or(area.x);
        let right_x = (area.x + area.width.saturating_sub(w + 1)).max(tabs_right).min(area.right());
        let r = Rect::new(right_x, area.y, w.min(area.right().saturating_sub(right_x)), 1);
        Paragraph::new(Text::from_line(right)).render(r, frame);
    }
}


pub(super) fn pad(r: Rect) -> Rect { r.inner(Sides::horizontal(1)) }

pub(super) fn detail_columns(area: Rect) -> [Rect; 2] {
    columns_with_details(area, 100)
}

pub(super) fn torrent_columns(area: Rect) -> [Rect; 2] {
    columns_with_details(area, 140)
}

pub(super) fn columns_with_details(area: Rect, minimum_width: u16) -> [Rect; 2] {
    if area.width < minimum_width {
        [area, Rect::new(area.x.saturating_add(area.width), area.y, 0, area.height)]
    } else {
        let cols = Flex::horizontal().constraints([Constraint::Percentage(58.0), Constraint::Fill]).split(area);
        [cols[0], cols[1]]
    }
}

pub(super) fn episode_rows(inner: Rect, warning: bool) -> [Rect; 3] {
    let rows = Flex::vertical().constraints([Constraint::Fixed(1), Constraint::Fill, Constraint::Fixed(if warning { 2 } else { 0 })]).split(inner);
    [rows[0], rows[1], rows[2]]
}

pub(super) fn providers_area(b: Rect, count: usize) -> Rect {
    let w = 84.min(b.width);
    let h = (count.saturating_add(4).min(u16::MAX as usize) as u16).max(8).min(b.height);
    Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h)
}

pub(super) fn settings_area(b: Rect) -> Rect {
    let width = b.width.min(78);
    let height = b.height.min(18);
    Rect::new(b.x + b.width.saturating_sub(width) / 2, b.y + b.height.saturating_sub(height) / 2, width, height)
}

pub(super) fn files_area(b: Rect) -> Rect {
    let w = (b.width * 9 / 10).max(40.min(b.width));
    let h = (b.height * 8 / 10).max(8.min(b.height));
    Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h)
}
