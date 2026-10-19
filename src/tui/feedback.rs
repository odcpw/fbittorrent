//! Bounded action history and ftui toast lifetimes; compact cached one-line views.
use super::*;
use crate::api::{LogEntry, LogLevel};
use ftui::widgets::{
    log_viewer::{LogViewer, LogViewerState},
    notification_queue::{NotificationQueue, QueueConfig},
    toast::{Toast, ToastId},
};
use std::collections::VecDeque;
const HISTORY: usize = 500;
const LEVELS: [LogLevel; 4] = [
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Error,
];
pub(super) struct Feedback {
    pub queue: NotificationQueue,
    pub history: VecDeque<LogEntry>,
    pub log: RefCell<LogPanel>,
    pub dirty: Cell<bool>,
    pub rects: Cell<[Option<Rect>; 2]>,
    next: u64,
    toasts: RefCell<Vec<(u64, Paragraph<'static>)>>,
    toast_palette: Cell<Option<(u32, u32, u32, u32, bool)>>,
}
pub(super) struct LogPanel {
    pub viewer: LogViewer,
    pub state: LogViewerState,
    pub header: Paragraph<'static>,
    pub level: usize,
    pub scroll: usize,
    pub source: Option<String>,
    pub sources: Vec<String>,
    hash: Option<String>,
    palette: Option<(u32, u32, u32, u32, u32, bool)>,
}
impl Default for Feedback {
    fn default() -> Self {
        Self {
            queue: NotificationQueue::new(
                QueueConfig::new()
                    .max_visible(2)
                    .max_queued(1)
                    .dedup_window_ms(0)
                    .default_duration(Duration::from_secs(10)),
            ),
            history: VecDeque::new(),
            next: 0,
            dirty: Cell::new(true),
            rects: Cell::new([None; 2]),
            log: RefCell::new(LogPanel {
                viewer: LogViewer::new(2500),
                state: LogViewerState::default(),
                header: Paragraph::new(""),
                level: 0,
                scroll: 0,
                source: None,
                sources: vec![],
                hash: None,
                palette: None,
            }),
            toasts: RefCell::new(vec![]),
            toast_palette: Cell::new(None),
        }
    }
}
impl Feedback {
    pub fn push(&mut self, message: &str, error: bool, hash: Option<crate::api::Hash>) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut message: String = message
            .chars()
            .take(2048)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        message.truncate(message.floor_char_boundary(2048));
        let level = if error {
            LogLevel::Error
        } else {
            LogLevel::Info
        };
        if self
            .history
            .back()
            .is_some_and(|last| last.ts == ts && last.message == message && last.level == level)
        {
            return;
        }
        self.next = self.next.saturating_add(1);
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(LogEntry {
            seq: self.next,
            ts,
            level,
            source: "tui".into(),
            message: message.clone(),
            hash,
        });
        self.queue.tick(Duration::ZERO);
        if self.queue.visible_count() == 2 {
            self.queue.visible_mut()[0].dismiss_immediately();
            self.queue.tick(Duration::ZERO);
        }
        self.queue.notify(
            Toast::with_id(ToastId(self.next), message)
                .duration(Duration::from_secs(10))
                .max_width(60)
                .no_animation(),
        );
        self.queue.tick(Duration::ZERO);
        self.dirty.set(true);
    }
    pub fn tick(&mut self) {
        self.queue.tick(Duration::from_millis(300));
    }
    pub fn refresh(&self, entries: &VecDeque<LogEntry>, theme: &Theme, hash: Option<&str>) {
        let mut panel = self.log.borrow_mut();
        let palette = (
            theme.fg.0,
            theme.dim.0,
            theme.selection.0,
            theme.red.0,
            theme.yellow.0,
            theme.low_chroma,
        );
        if !self.dirty.replace(false)
            && panel.hash.as_deref() == hash
            && panel.palette == Some(palette)
        {
            return;
        }
        if panel.hash.as_deref() != hash {
            panel.scroll = 0;
        }
        panel.hash = hash.map(str::to_owned);
        panel.palette = Some(palette);
        let mut all: Vec<_> = entries.iter().chain(self.history.iter()).collect();
        panel.sources.clear();
        for entry in &all {
            if !panel.sources.contains(&entry.source) {
                panel.sources.push(entry.source.clone());
            }
        }
        if panel
            .source
            .as_ref()
            .is_some_and(|s| !panel.sources.contains(s))
        {
            panel.source = None;
        }
        panel.header = Paragraph::new(format!(
            "level: {} · source: {} · l level · h source",
            panel.level_name(),
            panel.source.as_deref().unwrap_or("all")
        ))
        .style(theme.dimmed())
        .wrap(WrapMode::None);
        all.sort_by_key(|entry| (entry.ts, entry.seq));
        let old_count = panel.viewer.len();
        panel.viewer = LogViewer::new(2500).style(theme.text().bg(theme.selection));
        let level = LEVELS[panel.level];
        for entry in all {
            if hash.is_some_and(|h| entry.hash.as_ref().is_none_or(|key| key.as_str() != h))
                || panel.source.as_ref().is_some_and(|s| s != &entry.source)
                || (rank(entry.level) > rank(level) && !(level==LogLevel::Warn && entry.source=="tui"))
            {
                continue;
            }
            let marker = if entry.level == LogLevel::Error {
                "!"
            } else if entry.level == LogLevel::Warn {
                "△"
            } else if entry.source == "tui" {
                "✓"
            } else {
                "·"
            };
            let style = match entry.level {
                LogLevel::Error => theme.err(),
                LogLevel::Warn => theme.warn(),
                _ => theme.text(),
            }
            .bg(theme.selection);
            panel.viewer.push(Line::from_spans([
                Span::styled(
                    format!("{} {} {} ", entry.ts, level_name(entry.level), entry.source),
                    theme.dimmed(),
                ),
                Span::styled(format!("{marker} {}", entry.message), style),
            ]));
        }
        if panel.scroll > 0 {
            panel.scroll = panel
                .scroll
                .saturating_add(panel.viewer.len().saturating_sub(old_count));
        }
        if panel.scroll == 0 {
            panel.viewer.scroll_to_bottom();
        } else {
            let top = panel
                .viewer
                .len()
                .saturating_sub(usize::from(panel.state.last_viewport_height).max(1))
                .saturating_sub(panel.scroll);
            panel.viewer.scroll_to_top();
            panel.viewer.scroll_down(top);
        }
    }
    pub fn toast_rects(&self, area: Rect) -> [Option<Rect>; 2] {
        let count = self.queue.visible_count().min(2).min(area.height as usize);
        std::array::from_fn(|i| {
            if i < count {
                Some(Rect::new(
                    area.right().saturating_sub(area.width.min(60)),
                    area.bottom() - count as u16 + i as u16,
                    area.width.min(60),
                    1,
                ))
            } else {
                None
            }
        })
    }
    pub fn view_toasts(&self, area: Rect, theme: &Theme, frame: &mut Frame) {
        self.rects.set(self.toast_rects(area));
        let palette = (
            theme.fg.0,
            theme.selection.0,
            theme.red.0,
            theme.green.0,
            theme.low_chroma,
        );
        let mut cached = self.toasts.borrow_mut();
        if self.toast_palette.get() != Some(palette)
            || cached
                .iter()
                .map(|(id, _)| *id)
                .ne(self.queue.visible().iter().map(|t| t.id.0))
        {
            cached.clear();
            self.toast_palette.set(Some(palette));
            for toast in self.queue.visible() {
                let error = self
                    .history
                    .iter()
                    .rev()
                    .find(|e| e.seq == toast.id.0)
                    .is_some_and(|e| e.level == LogLevel::Error);
                let marker = if error { "!" } else { "✓" };
                let style = if error { theme.err() } else { theme.ok() }.bg(theme.selection);
                cached.push((
                    toast.id.0,
                    Paragraph::new(format!("{marker} {}", toast.content.message))
                        .style(style)
                        .wrap(WrapMode::None),
                ));
            }
        }
        for ((_, paragraph), rect) in cached
            .iter()
            .zip(self.toast_rects(area).into_iter().flatten())
        {
            let mut blank = ftui::Cell::from_char(' ');
            blank.fg = theme.fg;
            blank.bg = theme.selection;
            frame.buffer.fill(rect, blank);
            paragraph.render(rect, frame);
        }
    }
}
fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}
impl LogPanel {
    pub fn level_name(&self) -> &'static str {
        level_name(LEVELS[self.level])
    }
    pub fn next_source(&mut self) {
        self.source = match &self.source {
            None => self.sources.first().cloned(),
            Some(s) => self
                .sources
                .iter()
                .position(|x| x == s)
                .and_then(|i| self.sources.get(i + 1))
                .cloned(),
        };
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
    }
}
