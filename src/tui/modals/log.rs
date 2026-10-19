//! One history view for the global modal and the torrent Details Log tab.
use super::*;
impl TuiApp {
    pub(super) fn toggle_log(&mut self) -> Cmd<Msg> {
        self.log_open = !self.log_open;
        Cmd::none()
    }
    pub(super) fn on_log_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        use actions::Intent;
        if intent == Intent::Cancel {
            if self.log_open {
                self.log_open = false;
                return Cmd::none();
            }
            return self.close_details();
        }
        let hash = if self.log_open {
            None
        } else {
            self.details.as_ref().map(|d| d.hash.as_str())
        };
        self.feedback.refresh(&self.store.logs, &self.theme, hash);
        let mut panel = self.feedback.log.borrow_mut();
        match intent {
            Intent::Right => {
                panel.level = (panel.level + 1) % 4;
                panel.scroll = 0;
                self.feedback.dirty.set(true);
            }
            Intent::Left => {
                panel.next_source();
                panel.scroll = 0;
                self.feedback.dirty.set(true);
            }
            Intent::Up => {
                panel.viewer.scroll_up(1);
                panel.scroll = panel.scroll.saturating_add(1).min(panel.viewer.len());
            }
            Intent::Down => {
                panel.viewer.scroll_down(1);
                panel.scroll = panel.scroll.saturating_sub(1);
            }
            Intent::First => {
                panel.viewer.scroll_to_top();
                panel.scroll = panel.viewer.len();
            }
            Intent::Last => {
                panel.viewer.scroll_to_bottom();
                panel.scroll = 0;
            }
            _ => {}
        }
        Cmd::none()
    }
    pub(super) fn view_log_contents(&self, area: Rect, hash: Option<&str>, frame: &mut Frame) {
        self.feedback.refresh(&self.store.logs, &self.theme, hash);
        let mut panel = self.feedback.log.borrow_mut();
        let header = Rect::new(area.x, area.y, area.width, area.height.min(1));
        panel.header.render(header, frame);
        let body = Rect::new(
            area.x,
            area.y + header.height,
            area.width,
            area.height.saturating_sub(header.height),
        );
        let feedback::LogPanel { viewer, state, .. } = &mut *panel;
        if viewer.is_empty() {
            Paragraph::new("No matching messages")
                .style(self.theme.dimmed())
                .render(body, frame);
        } else {
            StatefulWidget::render(viewer, body, frame, state);
        }
    }
    pub(super) fn view_log(&self, frame: &mut Frame) {
        self.rects.borrow_mut().overlay();
        let b = frame.bounds();
        let width = b.width.min(116);
        let height = b.height.saturating_sub(3).min(32);
        let area = Rect::new(b.x + (b.width - width) / 2, b.y + 1, width, height);
        frame.buffer.fill(area, ftui::Cell::from_char(' '));
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.theme.border_focused())
            .title(" Log · L/Esc Back ")
            .style(self.theme.text().bg(self.theme.selection));
        let inner = block.inner(area);
        block.render(area, frame);
        let stats=self.store.stats.as_ref().filter(|s|self.online && !self.store.stale && s.session.is_some() && s.all_time.is_some());
        let rows=if stats.is_some(){if inner.width<100 {2}else{1}}else{0};
        let rows=rows.min(inner.height);
        self.view_log_contents(Rect::new(inner.x,inner.y,inner.width,inner.height.saturating_sub(rows)), None, frame);
        if let Some(stats)=stats {self.statistics_footer.borrow_mut().render(stats,self.theme.dimmed(),Rect::new(inner.x,inner.bottom()-rows,inner.width,rows),frame);}

    }
}
