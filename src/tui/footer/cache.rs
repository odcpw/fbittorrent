//! Cache owned widgets so an unchanged footer does not format/allocate per frame.
use super::*;
use std::hash::{Hash, Hasher};

pub(in crate::tui) struct FooterCache {
    pub(super) signature: u64,
    pub(super) context_width: u16,
    pub(super) labels_width: u16,
    pub(super) help_width: u16,
    pub(super) context: Paragraph<'static>,
    pub(super) legend: Paragraph<'static>,
    pub(super) help: Paragraph<'static>,
    pub(super) feedback: Paragraph<'static>,
}

impl TuiApp {
    pub(in crate::tui) fn footer_signature(&self, width: u16) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        width.hash(&mut hash);
        self.recovery.notice(std::time::Instant::now()).hash(&mut hash);
        self.screen.index().hash(&mut hash);
        self.footer_context().hash(&mut hash);
        self.cfg.native().hash(&mut hash);
        self.theme_mtime.hash(&mut hash);
        self.status.hash(&mut hash);
        self.status_is_error.hash(&mut hash);
        self.search_card.hash(&mut hash);
        self.search_input.value().trim().is_empty().hash(&mut hash);
        self.inbox_last_read.is_some().hash(&mut hash);
        self.shows_inbox().hash(&mut hash);
        if let Some(torrent) = self.selected_torrent() {
            self.store.pending_for_key(&torrent.hash).hash(&mut hash);
            torrent.name.hash(&mut hash);
            torrent.state.hash(&mut hash);
            self.complete_no_seed(torrent).hash(&mut hash);
            self.torrent_paused(torrent).hash(&mut hash);
            self.seed_allowed(torrent).hash(&mut hash);
        }
        if let Some(result) = self.selected_result() {
            result.file_name.hash(&mut hash);
        }
        if let Some(feed) = self.selected_feed() {
            feed.name.hash(&mut hash);
            feed.articles.len().hash(&mut hash);
            if let Some(article) = self.item_list.borrow().selected().and_then(|i| feed.articles.get(i)) {
                article.title.hash(&mut hash);
                article.is_read.hash(&mut hash);
            }
        }
        if let Some(show) = self.selected_show() {
            show.title.hash(&mut hash);
        }
        if let Some((show, episode)) = self.selected_episode() {
            show.title.hash(&mut hash);
            episode.title.hash(&mut hash);
            show.is_new(&episode.key).hash(&mut hash);
        }
        if let Some(entry) = self
            .browse_list
            .borrow()
            .selected()
            .and_then(|i| self.browse_entries.get(i))
        {
            entry.label.hash(&mut hash);
            matches!(entry.source, browse::Source::Site(_)).hash(&mut hash);
        }
        if let Some(files) = &self.files {
            files
                .list
                .borrow()
                .selected()
                .and_then(|i| files.files.get(i))
                .map(|file| file.name.as_str())
                .hash(&mut hash);
        }
        if let Some(providers) = &self.providers {
            providers
                .list
                .borrow()
                .selected()
                .and_then(|i| providers.plugins.get(i))
                .map(|plugin| plugin.full_name.as_str())
                .hash(&mut hash);
        }
        if let Some(prompt) = &self.prompt {
            std::mem::discriminant(&prompt.kind).hash(&mut hash);
            prompt.add_flags.hash(&mut hash);
            prompt.native_streaming.hash(&mut hash);
        }
        hash.finish()
    }

    pub(super) fn build_footer(&self, width: u16, signature: u64) -> FooterCache {
        let th = &self.theme;
        let help = actions::all().find(|a| a.id == "help").unwrap();
        let help = format!("({}) {}", help.keys[0].label(), (help.label)(self).unwrap());
        let help_width = dw(&help).min(usize::from(width));
        let available = usize::from(width).saturating_sub(help_width + 2);
        let context = format!("[{}]", self.footer_context());
        let context_width = dw(&context).min(available);
        let labels_width = available.saturating_sub(context_width + 1);
        let legend = pack_labels(&self.footer_labels(), labels_width);
        let feedback = if self.recovery.notice(std::time::Instant::now()).is_some() || self.daemon_stopped || self.status.is_empty() {
            self.footer_selection().unwrap_or_default()
        } else {
            self.status.clone()
        };
        FooterCache {
            signature,
            context_width: context_width as u16,
            labels_width: labels_width as u16,
            help_width: help_width as u16,
            context: Paragraph::new(context).style(th.title()).wrap(WrapMode::None),
            legend: Paragraph::new(Text::from_line(shortcut_line(&[&legend], th))).wrap(WrapMode::None),
            help: Paragraph::new(Text::from_line(shortcut_line(&[&help], th))).wrap(WrapMode::None),
            feedback: Paragraph::new(feedback)
                .style(if self.status_is_error { th.err() } else { th.dimmed() })
                .wrap(WrapMode::None),
        }
    }
}
