//! Existing modals prompt behavior.
use super::*;

pub(super) struct Prompt {
    pub(super) kind: PromptKind,
    pub(super) label: String,
    pub(super) input: TextInput,
    pub(super) yes_no: bool,
    /// Per-add overrides for the magnet prompt: (sequential, first/last, allow upload).
    pub(super) add_flags: (bool, bool, bool),
    pub(super) native_streaming: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PromptKind {
    AddMagnet,
    AddFeed,
    ConfirmDeleteTorrent { hash: String, files: bool },
    ConfirmRemoveFeed { name: String },
    FollowShow,
    ConfirmUnfollow { imdb: String, title: String },
}

impl TuiApp {
    pub(super) fn open_prompt(&mut self, kind: PromptKind, label: &str, yes_no: bool) {
        let mut input = TextInput::new()
            .with_style(self.theme.text())
            .with_cursor_style(Style::new().bg(self.theme.accent).fg(self.theme.selection));
        input.set_focused(!yes_no);
        self.search_input.set_focused(false);
        let add_flags = (self.cfg.native() || self.cfg.sequential, self.cfg.native() || self.cfg.first_last, self.cfg.seed_by_default);
        self.prompt = Some(Prompt { kind, label: label.to_string(), input, yes_no, add_flags, native_streaming: self.cfg.native() });
    }

    pub(super) fn close_prompt(&mut self) {
        self.prompt = None;
        self.search_input.set_focused(self.screen == Screen::Search && self.search_focus_input);
    }

    #[cfg(test)]
    pub(super) fn on_prompt_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Prompt, key)
    }

    pub(super) fn on_prompt_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let Some(p) = self.prompt.as_mut() else { return Cmd::none() };
        if p.yes_no {
            let kind = p.kind.clone();
            match intent {
                actions::Intent::Confirm => {
                    self.close_prompt();
                    return match kind {
                        PromptKind::ConfirmDeleteTorrent { hash, files } =>
                            self.action(if files { "deleted with files" } else { "removed" }, move |c| c.delete(&[hash], files)),
                        PromptKind::ConfirmRemoveFeed { name } => {
                            self.rss_action(format!("removed feed {name}"), move |c| c.rss_remove(&name))
                        }
                        PromptKind::ConfirmUnfollow { imdb, title } => {
                            self.shows.unfollow(&imdb);
                            if self.inbox_last_read.as_ref().is_some_and(|target| target.imdb == imdb) { self.inbox_last_read = None; }
                            self.save_shows();
                            let n = self.shows.shows.len() + 1;
                            let mut st = self.show_list.borrow_mut();
                            match st.selected() { Some(i) if i < n => {}, _ => st.select(n.checked_sub(1)) }
                            drop(st);
                            self.ep_list.borrow_mut().select(None);
                            self.rebuild_inbox();
                            self.set_status(format!("unfollowed {title}"), false);
                            Cmd::none()
                        }
                        _ => Cmd::none(),
                    };
                }
                _ => { self.close_prompt(); return Cmd::none(); }
            }
        }
        match intent {
            actions::Intent::Cancel => { self.close_prompt(); Cmd::none() }
            actions::Intent::Sequential if p.kind == PromptKind::AddMagnet && !p.native_streaming => { p.add_flags.0 = !p.add_flags.0; Cmd::none() }
            actions::Intent::FirstLast if p.kind == PromptKind::AddMagnet && !p.native_streaming => { p.add_flags.1 = !p.add_flags.1; Cmd::none() }
            actions::Intent::Seed if p.kind == PromptKind::AddMagnet => { p.add_flags.2 = !p.add_flags.2; Cmd::none() }
            actions::Intent::Primary => {
                let value = p.input.value().trim().to_string();
                let kind = p.kind.clone();
                let (seq, fl, seed) = p.add_flags;
                self.close_prompt();
                if value.is_empty() { return Cmd::none(); }
                match kind {
                    PromptKind::AddMagnet => {
                        let label = crate::magnet_name(&value).unwrap_or_else(|| value.chars().take(40).collect());
                        let mut opts = self.add_opts();
                        opts.sequential = seq; opts.first_last = fl; opts.seed = seed;
                        let flags = format!("{}{}{}", if seq { "S" } else { "" }, if fl { "F" } else { "" }, if seed { "U" } else { "" });
                        self.action(format!("added {label} [{flags}]"), move |c| c.add_url(&value, &opts))
                    }
                    PromptKind::AddFeed => {
                        let name = crate::host_of(&value);
                        self.rss_action("feed added", move |c| c.rss_add_feed(&value, name.as_deref()))
                    }
                    PromptKind::FollowShow => self.follow(value, None),
                    _ => Cmd::none(),
                }
            }
            _ => Cmd::none()
        }
    }
}


impl Prompt {
    /// Title with the live flag state, so the toggles are discoverable while typing.
    pub(super) fn title(&self) -> String {
        if self.kind != PromptKind::AddMagnet { return self.label.clone(); }
        let (s, f, u) = self.add_flags;
        if self.native_streaming {
            return format!("{} · streaming automatic · F4 seed:{} ", self.label, if u { "on" } else { "off" });
        }
        format!("{}  ·  F2 sequential:{}  F3 first/last:{}  F4 upload:{} ", self.label,
            if s { "on" } else { "off" }, if f { "on" } else { "off" }, if u { "on" } else { "off" })
    }
}
