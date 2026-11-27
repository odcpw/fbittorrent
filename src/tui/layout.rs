//! Existing layout behavior.
use super::*;
mod table;
mod sort;
mod notice;
mod list;
mod history;
mod status;
pub(super) use list::{ListTable,fingerprint};
pub(super) use table::{Column,Rects,TorrentTable,visible_range};

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

        let shows_tab=if self.shows.shows.is_empty()&&self.store.shows.is_empty() {" 3 Shows ".to_string()}else{format!(" 3 Shows ({}) ",self.shows_unread())};
        let rss_tab=if self.store.rss_feeds.is_empty() {
            if self.feeds.is_empty() {" 4 RSS ".to_string()}else{format!(" 4 RSS ({}) ",self.feeds.iter().flat_map(|f|&f.articles).filter(|a|!a.is_read).count())}
        }else{format!(" 4 RSS ({}) ",self.store.rss_feeds.values().map(|f|f.unread).sum::<u64>())};
        let mut labels = [" 1 Torrents ".to_string(), " 2 Search ".to_string(), shows_tab, rss_tab];
        let status = self.header_status(area.width);
        // Keep destinations stationary as the connection and transfer rates change.
        let brand = if area.width >= 92 { "  ▥  FBITTORRENT  " } else if area.width >= 56 { " ▥  " } else { "" };
        let brand_width = dw(brand) as u16;
        frame.buffer.fill(area, ftui::Cell::from_char(' ').with_bg(th.panel));
        Paragraph::new(brand).style(th.accented().bg(th.panel)).render(
            Rect::new(area.x, area.y, brand_width, 1), frame
        );
        let nav = Rect::new(area.x + brand_width, area.y, area.width - brand_width, 1);
        if labels.iter().map(|label| dw(label) + 2).sum::<usize>() > usize::from(nav.width) {
            labels = ["1", "2", "3", "4"].map(str::to_owned);
            labels[self.screen.index()] = format!("{} {}", self.screen.index() + 1,
                ["Torrents", "Search", "Shows", "RSS"][self.screen.index()]);
        }
        let mut x = nav.x;
        let regions = labels.each_ref().map(|label| {
            // Tabs wraps each title with two visible bracket/space cells.
            let width = (dw(&label).saturating_add(2) as u16).min(nav.right().saturating_sub(x));
            let region = Rect::new(x, area.y, width, area.height);
            x = x.saturating_add(width);
            region
        });
        self.tab_rects.set(regions);
        self.rects.borrow_mut().tabs=regions;
        let tabs = Tabs::new(labels.each_ref().map(|label|Tab::new(label.as_str())))
            .style(th.dimmed().bg(th.panel))
            .active_style(th.selected())
            .separator("");
        StatefulWidget::render(&tabs, nav, frame, &mut self.tabs.borrow_mut());

        let right=Line::styled(status,if self.online {th.text()}else{th.err()});
        let w = right.width() as u16;
        let tabs_right = regions.iter().map(Rect::right).max().unwrap_or(area.x);
        let right_x = (area.x + area.width.saturating_sub(w + 1)).max(tabs_right).min(area.right());
        let r = Rect::new(right_x, area.y, w.min(area.right().saturating_sub(right_x)), 1);
        self.view_speed_history(Rect::new(tabs_right.saturating_add(1),area.y,right_x.saturating_sub(tabs_right+2),1),frame);
        Paragraph::new(Text::from_line(right)).render(r, frame);
    }
}


pub(super) fn pad(r: Rect) -> Rect { r.inner(Sides::horizontal(1)) }

pub(super) fn detail_columns(area: Rect) -> [Rect; 2] {
    columns_with_details(area, 140)
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
    let height = b.height.min(22);
    Rect::new(b.x + b.width.saturating_sub(width) / 2, b.y + b.height.saturating_sub(height) / 2, width, height)
}

pub(super) fn files_area(b: Rect) -> Rect {
    let w = (b.width * 9 / 10).max(40.min(b.width));
    let h = (b.height * 8 / 10).max(8.min(b.height));
    Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h)
}
