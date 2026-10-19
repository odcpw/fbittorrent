//! Existing modals prompt behavior.
use super::*;

pub(super) struct Prompt {
    pub(super) kind: PromptKind,
    pub(super) label: String,
    pub(super) input: TextInput,
    pub(super) yes_no: bool,
    /// Per-add overrides for the magnet prompt: (sequential, first/last, allow upload).
    pub(super) add_flags: (bool, bool, bool),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum PromptKind {
    AddMagnet,
    RenameTorrent {hash:String},
    SettingLimit(usize),
    SettingPeerPort,
    SettingDestination,
    SettingIdleExit,
    AddFeed,
    AddTracker { hash:String },
    ConfirmRemoveTracker { hash:String,url:String },
    ConfirmDeleteTorrent { hash: String, files: bool },
    ConfirmRemoveFeed { name: String },
    FollowShow,
    ShowQuality {imdb:String},
    ShowFilters,
    Shutdown,
    ConfirmUnfollow { imdb: String, title: String },
}

impl TuiApp {
    pub(super) fn open_prompt(&mut self, kind: PromptKind, label: &str, yes_no: bool) {
        let mut input = TextInput::new()
            .with_style(self.theme.text())
            .with_cursor_style(Style::new().bg(self.theme.accent).fg(self.theme.selection));
        if matches!(kind,PromptKind::SettingLimit(_)){input=input.with_max_length(16);}
        if kind==PromptKind::SettingPeerPort {input=input.with_max_length(5);}
        if matches!(kind,PromptKind::RenameTorrent {..}) {input=input.with_max_length(512);}
        if matches!(kind,PromptKind::AddTracker{..}) {input=input.with_max_length(4096).with_mask('•');}
        input.set_focused(!yes_no);
        self.search_input.set_focused(false);
        let add_flags = (true, true, self.cfg.seed_by_default);
        self.prompt = Some(Prompt { kind, label: label.to_string(), input, yes_no, add_flags });
    }

    pub(super) fn rename_prompt(&mut self)->Cmd<Msg> {
        if let Some(t)=self.selected_torrent() {
            let (hash,name)=(t.hash.clone(),t.name.clone());
            self.open_prompt(PromptKind::RenameTorrent {hash},"Rename to: (empty restores original)",false);
            self.prompt.as_mut().unwrap().input.set_value(name);
        }
        Cmd::none()
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
                        PromptKind::Shutdown => self.shutdown_daemon(),
                        PromptKind::ConfirmRemoveTracker{hash,url}=>self.edit_tracker(&hash,&url,false),
                        PromptKind::ConfirmDeleteTorrent { hash, files } =>
                            self.service_action("torrents.remove",serde_json::json!({"hashes":[hash],"delete_files":files}),if files {"deleted with files"}else{"removed · U to undo"}),
                        PromptKind::ConfirmRemoveFeed { name } => {
                            self.native_feed_action("rss.remove",&name,None,None,format!("removed feed {name}"))
                        }
                        PromptKind::ConfirmUnfollow { imdb, title } => self.service_action("shows.unfollow",serde_json::json!({"imdb":imdb}),format!("unfollowed {title}")),
                        _ => Cmd::none(),
                    };
                }
                _ => { self.close_prompt(); return Cmd::none(); }
            }
        }
        match intent {
            actions::Intent::Cancel => { self.close_prompt(); Cmd::none() }
            actions::Intent::Seed if p.kind == PromptKind::AddMagnet => { p.add_flags.2 = !p.add_flags.2; Cmd::none() }
            actions::Intent::Primary => {
                let raw=p.input.value().to_string();
                let value = raw.trim().to_string();
                let kind = p.kind.clone();
                let (seq, fl, seed) = p.add_flags;
                self.close_prompt();
                if let PromptKind::SettingLimit(index)=kind{return self.apply_limit_input(index,&value);}
                if kind==PromptKind::SettingPeerPort {return self.apply_peer_port_input(&value);}
                if let PromptKind::ShowQuality {imdb}=kind {return self.apply_show_quality(imdb,&value);}
                if kind==PromptKind::ShowFilters {return self.apply_show_filters(&value);}
                if let PromptKind::RenameTorrent {hash}=kind {
                    if !crate::engine::rename::valid_name(&raw){self.set_status("Name must contain no controls and at most 512 characters",true);return Cmd::none();}
                    return self.service_action("torrents.rename",serde_json::json!({"hash":hash,"name":raw}),"renamed torrent");
                }
                if kind==PromptKind::SettingDestination {return self.apply_destination(&value);}
                if kind==PromptKind::SettingIdleExit {return self.apply_idle_exit(&value);}
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
                        self.service_action("rss.add",serde_json::json!({"url":value,"name":name}),"feed added")
                    }
                    PromptKind::FollowShow => self.follow(value, None),
                    PromptKind::AddTracker{hash}=>self.edit_tracker(&hash,&value,true),
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
        format!("{} · streaming automatic · F4 seed:{} ", self.label, if self.add_flags.2 { "on" } else { "off" })
    }
}
