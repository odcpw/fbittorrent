//! The contextual legend is a view of the action catalogue, never a second keymap.
use super::*;
mod cache;
pub(super) use cache::FooterCache;

impl TuiApp {
    pub(super) fn footer_context(&self) -> &'static str {
        use actions::{ModalKind, Pane, Scope};
        match actions::focused(self) {
            Scope::Global => "Global",
            Scope::Screen(Screen::Torrents) => "Torrents",
            Scope::Screen(_) => "Screen",
            Scope::Pane(_, pane) => match pane {
                Pane::Query => "Query",
                Pane::Results => "Results",
                Pane::Browse => "Browse",
                Pane::Feeds => "Feeds",
                Pane::Items => "Items",
                Pane::Shows => "Shows",
                Pane::Releases => {
                    if self.shows_inbox() {
                        "Unread"
                    } else {
                        "Releases"
                    }
                }
            },
            Scope::Modal(kind) => match kind {
                ModalKind::Log => "Log",
                ModalKind::Files => "Files",
                ModalKind::Details => "Details",
                ModalKind::Poster => "Card",
                ModalKind::Providers => "Providers",
                ModalKind::Settings => "Settings",
                ModalKind::Help => "Help",
                ModalKind::Stopped => "Stopped",
            },
            Scope::Prompt => {
                if self.prompt.as_ref().is_some_and(|p| p.yes_no) {
                    "Confirm"
                } else {
                    "Input"
                }
            }
        }
    }

    pub(super) fn footer_labels(&self) -> Vec<(u8, String)> {
        let scope = actions::focused(self);
        let mut labels = Vec::new();
        for candidate in actions::scopes(scope).into_iter().flatten() {
            // A modal has its own actions; global navigation remains available
            // through dispatch, but must not crowd the dialog's legend.
            if candidate == actions::Scope::Global && matches!(scope, actions::Scope::Modal(_) | actions::Scope::Prompt)
            {
                continue;
            }
            for action in actions::all().filter(|a| {
                a.scope == candidate
                    && a.id != "help"
                    && a.surfaces.contains(&actions::Surface::Tui)
                    && (a.enabled)(self)
            }) {
                if let (Some(key), Some(label)) = (action.keys.first(), (action.label)(self)) {
                    labels.push((action.priority, format!("({}) {label}", key.label())));
                }
            }
        }
        labels.sort_by_key(|(priority, _)| *priority);
        labels
    }

    pub(super) fn footer_selection(&self) -> Option<String> {
        if let Some(notice)=self.connection_banner() {return Some(notice.into_owned());}
        if self.daemon_stopped {return Some("fbittorrent stopped · Enter starts it again".into());}
        if self.poster_open {return self.poster_target.as_ref().map(|(name,_)|name.clone());}
        if let Some(files) = &self.files {
            return files
                .list
                .borrow()
                .selected()
                .and_then(|i| files.files.get(i))
                .map(|file| file.name.clone());
        }
        if let Some(providers) = &self.providers {
            return providers
                .list
                .borrow()
                .selected()
                .and_then(|i| providers.plugins.get(i))
                .map(|plugin| plugin.full_name.clone());
        }
        match self.screen {
            Screen::Torrents => self.selected_torrent().map(|torrent| torrent.name.clone()),
            Screen::Search if self.search_focus_input => {
                Some("Type a query; Enter searches, Esc leaves the query box.".into())
            }
            Screen::Search if self.search_browse => self
                .browse_list
                .borrow()
                .selected()
                .and_then(|i| self.browse_entries.get(i))
                .map(|entry| entry.label.to_owned()),
            Screen::Search => self.selected_result().map(|result| result.file_name.clone()),
            Screen::Rss => self.selected_feed().map(|feed| {
                if self.rss_focus_items {
                    self.item_list
                        .borrow()
                        .selected()
                        .and_then(|i| feed.articles.get(i))
                        .map(|article| format!("{} · {}", article.title, feed.name))
                        .unwrap_or_else(|| feed.name.clone())
                } else {
                    feed.name.clone()
                }
            }),
            Screen::Shows => {
                if self.shows_focus_eps {
                    self.selected_episode()
                        .map(|(show, episode)| format!("{} · {}", episode.title, show.title))
                } else {
                    self.selected_show()
                        .map(|show| show.title.clone())
                        .or_else(|| Some("All unread · releases across every followed show".into()))
                }
            }
        }
    }

    pub(super) fn view_status(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let th = &self.theme;
        StatusLine::new().style(th.dimmed()).render(area, frame);
        let signature = self.footer_signature(area.width);
        let mut cache = self.footer_cache.borrow_mut();
        if cache.as_ref().is_none_or(|cached| cached.signature != signature) {
            *cache = Some(self.build_footer(area.width, signature));
        }
        let cached = cache.as_ref().unwrap();
        cached
            .context
            .render(Rect::new(area.x, area.y, cached.context_width, 1), frame);
        cached.legend.render(
            Rect::new(area.x + cached.context_width + 1, area.y, cached.labels_width, 1),
            frame,
        );
        cached.help.render(
            Rect::new(area.right() - cached.help_width, area.y, cached.help_width, 1),
            frame,
        );
        if area.height > 1 {
            cached
                .feedback
                .render(Rect::new(area.x, area.y + 1, area.width, 1), frame);
        }
    }
}

pub(super) fn pack_labels(labels: &[(u8, String)], width: usize) -> String {
    let total = labels.iter().map(|(_, label)| dw(label)).sum::<usize>() + labels.len().saturating_sub(1) * 2;
    let overflow = total > width;
    let limit = if overflow { width.saturating_sub(2) } else { width };
    let mut out = String::new();
    let mut used = 0;
    for (_, label) in labels {
        let needed = dw(label) + if out.is_empty() { 0 } else { 2 };
        if used + needed > limit {
            continue;
        }
        if !out.is_empty() {
            out.push_str("  ");
        }
        out.push_str(label);
        used += needed;
    }
    if overflow && width > 0 {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('…');
    }
    out
}

/// Keys and labels retain the exact same readable text without colour.
pub(super) fn shortcut_line(labels: &[&str], th: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        for (piece_index, piece) in label.split("  ").enumerate() {
            if piece_index > 0 {
                spans.push(Span::raw("  "));
            }
            if let Some(end) = piece.find(')') {
                spans.push(Span::styled(piece[..=end].to_owned(), th.key()));
                spans.push(Span::styled(piece[end + 1..].to_owned(), th.dimmed()));
            } else {
                spans.push(Span::styled(piece.to_owned(), th.dimmed()));
            }
        }
    }
    Line::from_spans(spans)
}
