//! Stable-hash details. Opening a row always starts on Overview, at every size.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailsTab {
    Overview,
    Files,
}
pub(super) struct DetailsModal {
    pub hash: String,
    pub name: String,
    pub tab: DetailsTab,
}
pub(super) fn details_area(bounds: Rect) -> Rect {
    if bounds.width < 100 || bounds.height < 30 {
        return bounds;
    }
    let width = bounds.width.min(100);
    Rect::new(
        bounds.x + (bounds.width - width) / 2,
        bounds.y + (bounds.height - 30) / 2,
        width,
        30,
    )
}
impl TuiApp {
    pub(super) fn open_details(&mut self, torrent: &Torrent) -> Cmd<Msg> {
        self.files = None;
        self.details = Some(DetailsModal {
            hash: torrent.hash.clone(),
            name: torrent.name.clone(),
            tab: DetailsTab::Overview,
        });
        Cmd::none()
    }
    pub(super) fn details_tab(&mut self, next: bool) -> Cmd<Msg> {
        let Some(details) = self.details.as_mut() else {
            return Cmd::none();
        };
        match (details.tab, next) {
            (DetailsTab::Overview, true) => {
                details.tab = DetailsTab::Files;
                let torrent = self.selected_torrent().cloned();
                if let Some(torrent) = torrent {
                    return self.open_files(&torrent);
                }
            }
            (DetailsTab::Files, false) => {
                details.tab = DetailsTab::Overview;
                self.files = None;
            }
            _ => {}
        }
        Cmd::none()
    }
    pub(super) fn close_details(&mut self) -> Cmd<Msg> {
        self.details = None;
        self.files = None;
        Cmd::none()
    }
    pub(super) fn view_details(&self, frame: &mut Frame) {
        let Some(details) = &self.details else { return };
        let th = &self.theme;
        let bounds = frame.bounds();
        self.rects.borrow_mut().overlay();
        let area = details_area(bounds);
        self.rects.borrow_mut().details = Some(area);
        Paragraph::new(Text::from_lines((0..area.height).map(|_| {
            Line::styled(" ".repeat(area.width as usize), th.text().bg(th.selection))
        })))
        .render(area, frame);
        let footer = Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 2);
        let panel = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
        let title = format!(
            " {} ",
            short(&details.name, panel.width.saturating_sub(4) as usize)
        );
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border_focused())
            .title(&title)
            .padding(Sides::horizontal(1));
        let inner = block.inner(panel);
        block.render(panel, frame);
        let rows = Flex::vertical()
            .constraints([Constraint::Fixed(2), Constraint::Fill])
            .split(inner);
        let tabs = if details.tab == DetailsTab::Overview {
            "[Overview]  Files"
        } else {
            "Overview  [Files]"
        };
        Paragraph::new(format!("{tabs}    [ / ] tabs"))
            .style(th.accented())
            .render(rows[0], frame);
        if self.selected_torrent().is_none() {
            Paragraph::new("This torrent is no longer in your list.")
                .style(th.dimmed())
                .render(rows[1], frame);
        } else if details.tab == DetailsTab::Files {
            self.view_file_contents(rows[1], frame);
        } else if let Some(t) = self.selected_torrent() {
            let api = self.store.torrents.get(&t.hash);
            let metrics = format::Metrics { legacy: t, api };
            let kv = |key: &str, value: String| {
                Line::from_spans([
                    Span::styled(format!("{key:<10}"), th.dimmed()),
                    Span::styled(value, th.text()),
                ])
            };
            let mut lines = vec![kv(
                "state",
                format!(
                    "{} · {}",
                    format::state_cell(t, api).trim_end(),
                    self.torrent_reason(t)
                ),
            )];
            if let Some(playable) = api.and_then(|r| r.playable.as_ref()) {
                let ready = if !playable.ready { "Buffering" }
                    else if playable.stall_free==Some(true) || (playable.stall_free.is_none() && playable.buffered_s.is_some_and(|s|s>=120)) {"▶ Ready to watch"}
                    else {"Playable · may stall"};
                let buffer = playable
                    .buffered_s
                    .map(|s| format!(" · {} buffered", human_eta(s.min(i64::MAX as u64) as i64)))
                    .unwrap_or_default();
                let finish=playable.finishes_in_s.map(|s|format!(" · finishes in {}",human_eta(s.min(i64::MAX as u64) as i64))).unwrap_or_default();
                let estimate=if playable.buffered_s.is_none() {
                    let steady=api.is_some_and(|r|crate::daemon::watch::estimate_rate(r.size.unwrap_or(0),r.dl_bps.unwrap_or(0)));
                    if steady {" · rate estimate: keeps up with a 2 h video"}else{" · rate estimate: may stall for a 2 h video"}
                }else{""};
                lines.push(kv("ready", format!("{ready}{buffer}{finish}{estimate} · (o) Open")));
            }
            lines.extend([
                kv("progress", metrics.progress()),
                kv(
                    "speed",
                    format!("{} · ETA {}", metrics.speed(), metrics.eta()),
                ),
                kv("peers", metrics.peers(self.cfg.native())),
                kv("upload", metrics.upload()),
                kv("path", t.save_path.clone()),
                kv("added", ago(t.added_on)),
            ]);
            if let Some(row) = api {
                lines.push(kv(
                    "origin",
                    row.origin.clone().unwrap_or_else(|| "unknown".into()),
                ));
            }
            if let Some(error) = &t.error_message {
                lines.push(Line::styled(error.clone(), th.err().bold()));
            }
            let height = lines.len().min(u16::MAX as usize) as u16;
            let sections = Flex::vertical()
                .constraints([Constraint::Fixed(height + 1), Constraint::Fill])
                .split(rows[1]);
            Paragraph::new(Text::from_lines(lines))
                .wrap(WrapMode::None)
                .render(sections[0], frame);
            self.view_imdb_card(&t.name, sections[1], frame);
        }
        self.rects.borrow_mut().footer=footer;
        self.view_status(footer, frame);
    }
}
