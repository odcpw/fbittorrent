//! The poster is reachable on every screen without changing the row selection.
use super::*;
pub(in crate::tui) fn poster_area(bounds: Rect) -> Rect {
    let width = bounds.width.saturating_sub(4).min(100);
    let top = u16::from(bounds.height > 0);
    let content = bounds.height.saturating_sub(top + 2);
    let height = content.min(24);
    Rect::new(
        bounds.x + (bounds.width - width) / 2,
        bounds.y + top + (content - height) / 2,
        width,
        height,
    )
}
impl TuiApp {
    pub(super) fn toggle_poster(&mut self) -> Cmd<Msg> {
        if self.poster_open {
            self.poster_open = false;
            return Cmd::none();
        }
        let Some((name, id)) = self.hovered_title() else {
            return Cmd::none();
        };
        self.poster_target = Some((name.clone(), id.clone()));
        self.poster_open = true;
        // Opening a cached card is local, including while its daemon is disconnected.
        if self.presence.as_ref().is_none_or(|c|!c.alive()){return Cmd::none();}
        self.lookup_imdb(name, id)
    }
    pub(super) fn close_poster(&mut self) -> Cmd<Msg> {
        self.poster_open = false;
        Cmd::none()
    }
    pub(super) fn poster_imdb(&mut self) -> Cmd<Msg> {
        let Some((name, id)) = self.poster_target.clone() else {
            return Cmd::none();
        };
        self.open_imdb(name, id)
    }
    pub(super) fn poster_wiki(&mut self) -> Cmd<Msg> {
        let Some((name, id)) = self.poster_target.clone() else {
            return Cmd::none();
        };
        self.open_wiki(name, id)
    }
    pub(super) fn view_poster_overlay(&self, frame: &mut Frame) {
        let Some((name, _)) = &self.poster_target else {
            return;
        };
        let th = &self.theme;
        let area = poster_area(frame.bounds());
        self.rects.borrow_mut().overlay();
        Paragraph::new(Text::from_lines((0..area.height).map(|_| {
            Line::styled(
                " ".repeat(area.width as usize),
                Style::new().bg(th.selection),
            )
        })))
        .render(area, frame);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border_focused())
            .padding(Sides::horizontal(1))
            .title(" IMDb · v/Esc Back ");
        let inner = block.inner(area);
        block.render(area, frame);
        let source = if self.screen == Screen::Search {
            self.selected_result().map(|r| r.engine.as_str())
        } else {
            None
        };
        let header = u16::from(source.is_some());
        if let Some(source) = source {
            Paragraph::new(source)
                .style(th.dimmed())
                .render(Rect::new(inner.x, inner.y, inner.width, 1), frame);
        }
        self.view_imdb_card(
            name,
            Rect::new(
                inner.x,
                inner.y + header,
                inner.width,
                inner.height.saturating_sub(header),
            ),
            frame,
        );
    }
}
