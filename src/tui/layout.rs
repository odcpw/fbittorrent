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

pub(super) fn header_height(height: u16) -> u16 {
    if height >= 17 { 3 } else { 1 }
}

impl TuiApp {
    pub(super) fn view_tabs(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        frame.buffer.fill(area, ftui::Cell::from_char(' ').with_bg(th.panel));
        if area.is_empty() { return; }

        let gutter = if area.width >= 45 { 3 } else { 1 };
        let bar = Rect::new(area.x, area.y + area.height / 2, area.width, 1)
            .inner(Sides::horizontal(gutter));
        let brand = if area.width >= 92 || (area.width >= 56 && area.height == 3) {
            "▥  FBITTORRENT"
        } else {
            "▥"
        };
        Paragraph::new(brand).style(th.accented()).render(bar, frame);
        let mut x = (bar.x + dw(brand) as u16 + 2).min(bar.right());

        let help = if area.width >= 56 { "?  Keys" } else { "?" };
        let help_width = (dw(help) as u16).min(bar.right() - x);
        let help_rect = Rect::new(bar.right() - help_width, bar.y, help_width, 1);
        Paragraph::new(help).style(th.key()).render(help_rect, frame);
        self.rects.borrow_mut().help = help_rect;

        let mut names = ["1 Torrents", "2 Search", "3 Shows", "4 RSS"].map(str::to_owned);
        if !self.shows.shows.is_empty() || !self.store.shows.is_empty() {
            names[2].push_str(&format!(" ({})", self.shows_unread()));
        }
        if !self.feeds.is_empty() || !self.store.rss_feeds.is_empty() {
            let unread = if self.store.rss_feeds.is_empty() {
                self.feeds.iter().flat_map(|f| &f.articles).filter(|a| !a.is_read).count() as u64
            } else {
                self.store.rss_feeds.values().map(|f| f.unread).sum()
            };
            names[3].push_str(&format!(" ({unread})"));
        }

        let nav_end = help_rect.x.saturating_sub(1).max(x);
        let mut suffix = if area.width < 80 { "" } else { " " };
        let all_width: usize = names.iter().map(|label| dw(label) + 1 + suffix.len()).sum();
        let selected = self.screen.index();
        if all_width > usize::from(nav_end - x) {
            suffix = " ";
            for (index, label) in names.iter_mut().enumerate() {
                if index != selected { *label = (index + 1).to_string(); }
            }
        }
        let mut regions = [Rect::default(); 4];
        for (index, label) in names.iter().enumerate() {
            let width = ((dw(label) + 1 + suffix.len()) as u16).min(nav_end - x);
            let region = Rect::new(x, bar.y, width, 1);
            let (mark, style) = if index == selected {
                ("▌", th.selected())
            } else {
                (" ", th.dimmed())
            };
            Paragraph::new(format!("{mark}{label}{suffix}")).style(style).render(region, frame);
            regions[index] = region;
            x += width;
        }
        self.rects.borrow_mut().tabs = regions;

        let status = self.header_status(area.width);
        let width = dw(&status) as u16;
        // A compact tile can use the masthead's bottom row for the daemon state.
        let (left, right, y) = if area.height == 3 && width + 2 > nav_end - x {
            (bar.x, bar.right(), area.bottom() - 1)
        } else {
            (x, nav_end, bar.y)
        };
        let status_x = right.saturating_sub(width).max(left);
        self.view_speed_history(Rect::new(left, y, status_x.saturating_sub(left + 1), 1), frame);
        let status_rect = Rect::new(status_x, y, right - status_x, 1);
        Paragraph::new(status).style(if self.online { th.dimmed() } else { th.err() })
            .render(status_rect, frame);
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
