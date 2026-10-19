//! Existing footer behavior.
use super::*;

impl TuiApp {
    pub(super) fn view_status(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() { return; }
        let th = &self.theme;
        let (context, hints, last): (&str, Vec<&str>, &str) = if self.show_help {
            ("Help", vec!["(↑↓) Scroll", "(PgUp/PgDn) Page"], "(Esc) Close")
        } else if self.settings.is_some() {
            ("Settings", vec!["(↑↓) Choose", "(←→) Change", "(Enter) Save"], "(Esc) Cancel")
        } else if let Some(prompt) = &self.prompt {
            if prompt.yes_no {
                ("Confirm", vec!["(y) Remove"], "(Esc) Cancel")
            } else {
                let submit = match prompt.kind {
                    PromptKind::FollowShow => "(Enter) Follow",
                    PromptKind::AddFeed => "(Enter) Add feed",
                    _ => "(Enter) Add",
                };
                ("Input", vec![submit], "(Esc) Cancel")
            }
        } else if self.files.is_some() {
            let priority = if self.cfg.native() { "(1) Include" } else { "(1/2/3) Priority" };
            ("Files", vec!["(Space) Include/skip", priority, "(A)ll", "(N)one"], "(Esc) Close")
        } else if self.providers.is_some() {
            ("Providers", vec!["(Space) Toggle", "(A)ll", "(N)one", "(O)pen site"], "(Esc) Close")
        } else { match self.screen {
            Screen::Torrents => {
                let mut hints = if let Some(torrent) = self.selected_torrent() {
                    let mut hints = vec!["(Enter) Files", "(O)pen", if torrent.complete_no_seed() { "(U) Seed" }
                        else if torrent.is_stopped() { "(P) Resume" } else { "(P)ause" },
                        "(D) Remove", "(Shift+D) Delete files"];
                    if !torrent.complete_no_seed() {
                        hints.push(if torrent.seed_allowed() { "(U) Seed off" } else { "(U) Seed on" });
                    }
                    hints
                } else { Vec::new() };
                hints.extend(["(A)dd", "(C)lipboard"]);
                if self.cfg.native() { hints.push("(F6) Settings"); }
                else { hints.extend(["(S)equential", "(F) First/last"]); }
                ("Torrents", hints, "(?) Help")
            }
            Screen::Search if self.search_focus_input => {
                let enter = if self.search_input.value().trim().is_empty() { "(Enter) Browse" } else { "(Enter) Search" };
                ("Query", vec![enter, "(↓/Esc) Results"], "(Esc, ?) Help")
            }
            Screen::Search if self.search_browse => {
                let site = self.browse_list.borrow().selected().and_then(|i| self.browse_entries.get(i))
                    .is_some_and(|entry| matches!(entry.source, browse::Source::Site(_)));
                ("Browse", vec![if site { "(Enter) Open site" } else { "(Enter) Load list" }, "(Esc) Results", "(/) Query"], "(?) Help")
            }
            Screen::Search => {
                let mut hints = Vec::new();
                if self.selected_result().is_some() { hints.extend(["(Enter) Download", "(Shift+O) Source", "(F)ollow show"]); }
                hints.extend(["(/) Query", "(S) Providers", "(B)rowse lists"]);
                ("Results", hints, "(?) Help")
            }
            Screen::Rss if self.rss_focus_items => {
                let picked = self.selected_feed().and_then(|feed| self.item_list.borrow().selected().and_then(|i| feed.articles.get(i)));
                let mut hints = Vec::new();
                if let Some(article) = picked {
                    hints.extend(["(Enter) Download", if self.cfg.native() && article.is_read { "(M)ark unread" } else { "(M)ark read" }]);
                }
                if self.selected_feed().is_some() { hints.push("(Shift+M) All read"); }
                hints.extend(["(←) Feeds", "(R)efresh", "(F)ollow show"]);
                ("Items", hints, "(?) Help")
            }
            Screen::Rss => {
                let mut hints = Vec::new();
                if self.selected_feed().is_some() { hints.extend(["(Enter) Items", "(Shift+M) All read"]); }
                hints.extend(["(N)ew feed", "(R)efresh"]);
                if self.selected_feed().is_some() { hints.push("(X) Remove feed"); }
                ("Feeds", hints, "(?) Help")
            }
            Screen::Shows if self.shows_focus_eps => {
                let mut hints = Vec::new();
                if let Some((show, episode)) = self.selected_episode() {
                    hints.extend(["(Enter) Download", if show.is_new(&episode.key) { "(M)ark read" } else { "(M)ark unread" }]);
                }
                if self.shows_inbox() {
                    if self.inbox_last_read.is_some() { hints.push("(U)ndo"); }
                    hints.extend(["(←) Shows", "(R) Check all"]);
                    ("Unread", hints, "(?) Help")
                } else {
                    if self.selected_show().is_some() { hints.push("(Shift+M) All read"); }
                    hints.extend(["(←) Shows", "(Shift+R) Check show", "(R) Check all"]);
                    ("Releases", hints, "(?) Help")
                }
            }
            Screen::Shows => {
                let mut hints = Vec::new();
                hints.push("(Enter) Releases");
                if self.selected_show().is_some() { hints.push("(Shift+M) All read"); }
                hints.extend(["(N) Follow show", "(R) Check all"]);
                if self.selected_show().is_some() { hints.extend(["(Shift+R) Check show", "(X) Unfollow"]); }
                ("Shows", hints, "(?) Help")
            }
        }};
        StatusLine::new().style(th.dimmed()).render(area, frame);
        // Keep the exit/help affordance and pane identity visible. Feedback has
        // its own row so a success or error cannot conceal the available actions.
        let width = usize::from(area.width);
        let last_width = dw(last).min(width);
        let last_x = area.right().saturating_sub(last_width as u16);
        Paragraph::new(Text::from_line(shortcut_line(&[last], th))).wrap(WrapMode::None)
            .render(Rect::new(last_x, area.y, last_width as u16, 1), frame);
        let available = width.saturating_sub(last_width + 2);
        let context = format!("[{context}]");
        let context_width = dw(&context).min(available);
        Paragraph::new(context.clone()).style(th.title()).wrap(WrapMode::None)
            .render(Rect::new(area.x, area.y, context_width as u16, 1), frame);
        let labels_width = available.saturating_sub(context_width + 1);
        let mut shown = Vec::new();
        let mut used = 0;
        for hint in hints {
            let needed = dw(hint) + if shown.is_empty() { 0 } else { 2 };
            if used + needed <= labels_width { shown.push(hint); used += needed; }
        }
        let actions_x = area.x.saturating_add(context_width as u16).saturating_add(1);
        Paragraph::new(Text::from_line(shortcut_line(&shown, th))).wrap(WrapMode::None)
            .render(Rect::new(actions_x, area.y, labels_width as u16, 1), frame);
        if area.height > 1 {
            let feedback = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
            Paragraph::new(" ".repeat(width)).style(th.dimmed()).render(feedback, frame);
            if !self.status.is_empty() {
                Paragraph::new(short(&self.status, width)).style(if self.status_is_error { th.err() } else { th.ok() })
                    .wrap(WrapMode::None).render(feedback, frame);
            } else if self.screen == Screen::Search && self.search_focus_input {
                Paragraph::new("Type a query; Enter searches, Esc leaves the query box.")
                    .style(th.dimmed()).wrap(WrapMode::None).render(feedback, frame);
            } else if self.prompt.is_none() && self.files.is_none() && self.providers.is_none() && self.settings.is_none() && !self.show_help {
                Paragraph::new("(1–4) Screens  (Tab/Shift+Tab) Switch screen  (↑↓) Move")
                    .style(th.dimmed()).wrap(WrapMode::None).render(feedback, frame);
            }
        }
    }
}


/// Style the parenthesized key separately from its action while retaining the
/// complete label as readable text in terminals with reduced styling.
pub(super) fn shortcut_line(labels: &[&str], th: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        if index > 0 { spans.push(Span::raw("  ")); }
        if let Some(end) = label.find(')') {
            spans.push(Span::styled(label[..=end].to_string(), th.key()));
            spans.push(Span::styled(label[end + 1..].to_string(), th.dimmed()));
        } else {
            spans.push(Span::styled((*label).to_string(), th.dimmed()));
        }
    }
    Line::from_spans(spans)
}
