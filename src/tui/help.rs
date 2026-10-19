//! Existing help behavior.
use super::*;

impl TuiApp {
    pub(super) fn help_text(&self) -> &'static str {
        actions::help_text(self.cfg.native())
    }

    pub(super) fn help_max_scroll(&self) -> u16 {
        let height = self.last_bounds.get().height.min(34).saturating_sub(5);
        (self.help_text().lines().count() as u16).saturating_sub(height)
    }

    pub(super) fn view_help(&self, frame: &mut Frame) {
        let th = &self.theme;
        let b = frame.bounds();
        let w = 84.min(b.width);
        let h = 34.min(b.height);
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Keys ").padding(Sides::all(1))
            .style(Style::new().bg(th.selection));
        let inner = block.inner(area);
        block.render(area, frame);
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let text = self.help_text();
        let scroll = self.help_scroll.min(self.help_max_scroll());
        Paragraph::new(text).style(Style::new().fg(th.fg).bg(th.selection)).scroll((scroll, 0)).wrap(WrapMode::None).render(rows[0], frame);
        Paragraph::from_static_str(actions::help_controls())
            .style(Style::new().fg(th.accent).bg(th.selection)).wrap(WrapMode::None).render(rows[1], frame);
    }
}
