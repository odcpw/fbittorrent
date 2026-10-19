//! Existing screens shows behavior.
use super::*;

impl TuiApp {
    pub(super) fn shows_inbox(&self) -> bool { self.show_list.borrow().selected() == Some(0) }

    pub(super) fn show_at(&self, index: usize) -> Option<&Show> {
        index.checked_sub(1).and_then(|i| self.shows.shows.get(i))
    }

    pub(super) fn selected_show(&self) -> Option<&Show> {
        self.show_list.borrow().selected().and_then(|i| self.show_at(i))
    }

    pub(super) fn episode_at(&self, index: usize) -> Option<(&Show, &Episode)> {
        if self.shows_inbox() {
            let target = self.inbox_rows.get(index)?;
            let show = self.shows.get(&target.imdb)?;
            Some((show, show.episode(&target.key)?))
        } else {
            let show = self.selected_show()?;
            show.episodes.get(index).map(|episode| (show, episode))
        }
    }

    pub(super) fn selected_episode(&self) -> Option<(&Show, &Episode)> {
        self.ep_list.borrow().selected().and_then(|i| self.episode_at(i))
    }

    pub(super) fn selected_release_target(&self) -> Option<ShowReleaseTarget> {
        if self.shows_inbox() {
            self.ep_list.borrow().selected().and_then(|i| self.inbox_rows.get(i)).cloned()
        } else {
            self.selected_episode().map(|(show, episode)| ShowReleaseTarget { imdb: show.imdb.clone(), key: episode.key.clone() })
        }
    }

    pub(super) fn episode_count(&self) -> usize {
        if self.shows_inbox() { self.inbox_rows.len() } else { self.selected_show().map_or(0, |s| s.episodes.len()) }
    }

    pub(super) fn rebuild_inbox(&mut self) {
        let selected = self.shows_inbox().then(|| self.selected_release_target()).flatten();
        let old_index = self.ep_list.borrow().selected().unwrap_or(0);
        let read=self.show_controls.read;let quality=self.show_controls.quality.as_deref();
        let mut releases: Vec<_> = self.shows.shows.iter().flat_map(|show| show.episodes.iter()
             .filter(move |episode| (read==ReadFilter::All || show.is_new(&episode.key)==(read==ReadFilter::Unread))
                && quality.is_none_or(|quality|quality==episode.quality))
            .map(move |episode| (show, episode))).collect();
        releases.sort_by(|(a_show, a), (b_show, b)| release_sort::episode_order(a,b,self.release_sort,self.release_descending)
            .then_with(|| a_show.title.cmp(&b_show.title)).then_with(|| b.key.cmp(&a.key)).then_with(|| a_show.imdb.cmp(&b_show.imdb)));
        self.inbox_rows = releases.into_iter().map(|(show, episode)| ShowReleaseTarget { imdb: show.imdb.clone(), key: episode.key.clone() }).collect();
        self.ep_list.borrow_mut().hovered = None;
        self.last_click = None;
        if self.shows_inbox() {
            let index = match selected {
                Some(target) => self.inbox_rows.iter().position(|row| *row == target)
                    .or_else(|| self.inbox_rows.len().checked_sub(1).map(|last| old_index.min(last))),
                None => (!self.inbox_rows.is_empty()).then_some(0),
            };
            self.ep_list.borrow_mut().select(index);
        }
    }

    pub(super) fn select_show_parent(&mut self, index: usize) {
        self.shows_focus_eps = false;
        self.show_list.borrow_mut().select(Some(index));
        self.ep_list.borrow_mut().select(None);
        self.rebuild_inbox();
    }

    pub(super) fn release_show(&self) -> Option<&Show> {
        if self.shows_inbox() { self.selected_episode().map(|(show, _)| show) } else { self.selected_show() }
    }

    pub(super) fn refresh_show(&mut self, imdb: String) -> Cmd<Msg> {
        self.service_action("shows.check",serde_json::json!({"imdb":imdb}),"shows checked")
    }

    pub(super) fn refresh_shows(&mut self, force: bool) -> Cmd<Msg> {
        if force {self.service_action("shows.check",serde_json::json!({}),"shows checked")}else{Cmd::none()}
    }

    pub(super) fn follow(&mut self, query: String, id: Option<String>) -> Cmd<Msg> {
        let id=id.or_else(||crate::imdb::service::valid_id(&query).then(||query.clone()));
        let args=if let Some(id)=id{serde_json::json!({"imdb":id})}else{serde_json::json!({"query":query})};
        self.service_action("shows.follow",args,"show followed")
    }

    #[cfg(test)]
    pub(super) fn on_shows_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        actions::dispatch_scope(self, actions::Scope::Screen(Screen::Shows), key)
    }

    pub(super) fn on_shows_action(&mut self, intent: actions::Intent) -> Cmd<Msg> {
        let ns = self.shows.shows.len() + 1;
        let ne = self.episode_count();
        let navigation = match intent {
            actions::Intent::Left => { self.shows_focus_eps = false; true }
            actions::Intent::Right => {
                self.shows_focus_eps = true;
                if ne > 0 && self.ep_list.borrow().selected().is_none() { self.ep_list.borrow_mut().select(Some(0)); }
                true
            }
            actions::Intent::Down | actions::Intent::Up | actions::Intent::First | actions::Intent::Last => {
                let state = if self.shows_focus_eps { &self.ep_list } else { &self.show_list };
                let n = if self.shows_focus_eps { ne } else { ns };
                let old = state.borrow().selected();
                match intent {
                    actions::Intent::Down => if n > 0 { state.borrow_mut().select_next(n); },
                    actions::Intent::Up => state.borrow_mut().select_previous(),
                    actions::Intent::First => state.borrow_mut().select((n > 0).then_some(0)),
                    _ => state.borrow_mut().select(n.checked_sub(1)),
                }
                if !self.shows_focus_eps {
                    let selected = self.show_list.borrow().selected();
                    if selected != old { if let Some(index) = selected { self.select_show_parent(index); } }
                }
                true
            }
            _ => false,
        };
        if navigation { return Cmd::none(); }
        match intent {
            actions::Intent::New => { self.open_prompt(PromptKind::FollowShow, "Follow show: title or IMDb id (tt…)", false); }
            actions::Intent::Remove => if let Some(s) = self.selected_show() {
                let (imdb, title) = (s.imdb.clone(), s.label());
                self.open_prompt(PromptKind::ConfirmUnfollow { imdb, title: title.clone() }, &format!("Unfollow '{title}'? y/N"), true);
            },
            actions::Intent::Refresh => return self.refresh_shows(true),
            actions::Intent::Reload => if let Some(s) = self.release_show() { let id = s.imdb.clone(); return self.refresh_show(id); },
            actions::Intent::Mark => {
                if !self.shows_focus_eps {
                    self.set_status("Enter opens releases · Shift+M marks one selected show's releases read", false);
                } else {
                    if let Some(target)=self.selected_release_target(){let read=!self.shows.get(&target.imdb).is_some_and(|s|s.seen.contains(&target.key));return self.service_action("shows.set_read",serde_json::json!({"imdb":target.imdb,"key":target.key,"read":read}),if read{"release marked read · (U)ndo"}else{"release marked unread"});}
                }
            }
            actions::Intent::Undo if self.shows_inbox() => {
                return self.service_action("shows.undo_read",serde_json::json!({}),"release restored to unread");
            }
            actions::Intent::MarkAll => {
                if let Some(s) = self.selected_show() {
                    let (imdb, title) = (s.imdb.clone(), s.title.clone());
                    return self.service_action("shows.mark_show_read",serde_json::json!({"imdb":imdb}),format!("{title}: releases marked read"));
                } else {
                    self.set_status("Shift+M applies to one show. Select a followed show on the left first.", false);
                }
            }
            actions::Intent::Primary => {
                if !self.shows_focus_eps {
                    self.shows_focus_eps = true;
                    if ne > 0 && self.ep_list.borrow().selected().is_none() { self.ep_list.borrow_mut().select(Some(0)); }
                    return Cmd::none();
                }
                let picked = self.selected_episode().map(|(s, e)| (s.imdb.clone(), e.clone()));
                if let Some((imdb, e)) = picked {
                    let token = format!("{imdb}:{}", e.key);
                    if self.service_busy{return Cmd::none();}
                    if !self.show_grabbing.insert(token) { return Cmd::none(); }
                    return self.service_action("shows.grab",serde_json::json!({"imdb":imdb,"key":e.key,"seed":self.cfg.seed_by_default}),format!("added {} {}",self.shows.get(&imdb).map(|s|s.title.as_str()).unwrap_or(&imdb),e.key));
                }
            }
            actions::Intent::Imdb => if let Some(s) = self.release_show() {
                let (name, id) = (s.label(), Some(s.imdb.clone()));
                return self.open_imdb(name, id);
            },
            actions::Intent::Wiki => if let Some(s) = self.release_show() {
                let (name, id) = (s.label(), Some(s.imdb.clone()));
                return self.open_wiki(name, id);
            },
            actions::Intent::Source => if let Some(s) = self.release_show() {
                let _ = std::process::Command::new("xdg-open").arg(format!("https://eztvx.to/search/{}", crate::qbt::urlencode(&s.title))).spawn();
            },
            _ => {}
        }
        Cmd::none()
    }

    #[cfg(test)]
    pub(super) fn mark_selected_release(&mut self) -> Option<String> {
        let target = self.selected_release_target()?;
        let inbox = self.shows_inbox();
        let show = self.shows.get_mut(&target.imdb)?;
        let read = show.toggle_seen(&target.key);
        let label = format!("{} {} marked {}{}", show.title, target.key, if read { "read" } else { "unread" }, if inbox { " · (U)ndo" } else { "" });
        if inbox && read { self.inbox_last_read = Some(target); }
        self.rebuild_inbox();
        Some(label)
    }

    #[cfg(test)]
    pub(super) fn undo_inbox_read(&mut self) -> Option<String> {
        let target = self.inbox_last_read.take()?;
        let show = self.shows.get_mut(&target.imdb)?;
        show.seen.remove(&target.key);
        let label = format!("{} {} restored to unread", show.title, target.key);
        self.rebuild_inbox();
        self.ep_list.borrow_mut().select(self.inbox_rows.iter().position(|row| *row == target));
        Some(label)
    }

    pub(super) fn view_shows(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let tier = layout::Tier::at(area.width);
        let cols = match tier {
            layout::Tier::Narrow => {
                if self.shows_focus_eps {
                    [Rect::default(), area, Rect::default()]
                } else {
                    [area, Rect::default(), Rect::default()]
                }
            }
            layout::Tier::Medium => {
                let c = Flex::horizontal()
                    .constraints([Constraint::Percentage(32.0), Constraint::Fill])
                    .split(area);
                [c[0], c[1], Rect::default()]
            }
            layout::Tier::Wide => {
                let c = Flex::horizontal()
                    .constraints([
                        Constraint::Percentage(22.0),
                        Constraint::Fill,
                        Constraint::Fixed(52),
                    ])
                    .split(area);
                [c[0], c[1], c[2]]
            }
        };
        let card_h = if tier == layout::Tier::Medium && cols[0].height >= 24 {
            11
        } else {
            0
        };
        let left = Flex::vertical()
            .constraints([Constraint::Fill, Constraint::Fixed(card_h)])
            .split(cols[0]);
        let unread = self.shows_unread();
        let title = format!(" Shows › Followed ({}) ", self.shows.shows.len());
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .padding(Sides::all(0))
            .border_style(if self.shows_focus_eps {
                th.border()
            } else {
                th.border_focused()
            })
            .title(&title);
        let inner = block.inner(left[0]);
        block.render(left[0], frame);
        let w = inner.width.saturating_sub(2) as usize;
        let elapsed = self.marquee_elapsed();
        self.view_rows(
            MouseList::Shows,
            inner,
            self.shows.shows.len() + 1,
            |i| {
                if i == 0 {
                    layout::fingerprint(&unread)
                } else {
                    let show = &self.shows.shows[i - 1];
                    layout::fingerprint(&(
                        &show.title,
                        show.year,
                        self.show_unread(show),
                        self.shows_refreshing
                            .contains(&show.imdb)
                            .then_some(self.tick % 4),
                        &show.error,
                        show.catalogue_complete,
                    ))
                }
            },
            |i| {
                if i == 0 {
                    return Line::from_spans([
                        Span::styled(
                            cell("All unread", w.saturating_sub(dw(&unread.to_string()) + 1)),
                            th.text().bold(),
                        ),
                        Span::styled(format!(" {unread}"), th.accented()),
                    ]);
                }
                let show = &self.shows.shows[i - 1];
                let n = self.show_unread(show);
                let busy = self.shows_refreshing.contains(&show.imdb);
                let warning =
                    show.error.is_some() || (show.last_checked > 0 && !show.catalogue_complete);
                let tail = if busy {
                    format!(" {}", SPINNER[self.tick % 4])
                } else if warning {
                    " !".into()
                } else if n > 0 {
                    format!(" {n}")
                } else {
                    String::new()
                };
                Line::from_spans([
                    Span::styled(
                        readable_cell(
                            &show.label(),
                            w.saturating_sub(dw(&tail)),
                            hovered_for(&self.show_list, i, elapsed),
                        ),
                        if n > 0 { th.text().bold() } else { th.text() },
                    ),
                    Span::styled(
                        tail,
                        if warning && !busy {
                            th.err()
                        } else {
                            th.accented()
                        },
                    ),
                ])
            },
            frame,
        );
        if card_h > 0 {
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(th.border())
                .title(" IMDb ");
            let inner = block.inner(left[1]);
            block.render(left[1], frame);
            if let Some(show) = self.release_show() {
                self.view_imdb_card(&show.label(), pad(inner), frame);
            }
        }

        if !cols[2].is_empty() {
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(th.border())
                .title(" IMDb ");
            let card = block.inner(cols[2]);
            block.render(cols[2], frame);
            if let Some(show) = self.release_show() {
                self.view_imdb_card(&show.label(), card, frame);
            }
        }
        self.shows_rects.set([inner, Rect::default()]);
        if cols[1].is_empty() {
            return;
        }
        let inbox = self.shows_inbox();
        let title = if inbox {
            if self.show_controls.read==ReadFilter::Unread && self.show_controls.quality.is_none() {format!(" Shows › All unread · {unread} releases · {} {} ",self.release_sort.label(),if self.release_descending {"↓"}else{"↑"})}
            else {format!(" Shows › {} {} · {} releases · {} {} ",self.show_controls.read.label(),self.show_controls.quality.as_deref().unwrap_or("qualities"),self.inbox_rows.len(),self.release_sort.label(),if self.release_descending {"↓"}else{"↑"})}
        } else if let Some(show) = self.selected_show() {
            format!(
                " Shows › {} · {} releases ",
                show.label(),
                show.episodes.len()
            )
        } else {
            " Releases ".into()
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .padding(Sides::all(0))
            .border_style(if self.shows_focus_eps {
                th.border_focused()
            } else {
                th.border()
            })
            .title(&title);
        let right = block.inner(cols[1]);
        block.render(cols[1], frame);
        let summary = Rect::new(right.x, right.y, right.width, right.height.min(1));
        let releases_inner = Rect::new(
            right.x,
            right.y.saturating_add(summary.height),
            right.width,
            right.height.saturating_sub(summary.height),
        );
        let warning = if inbox {
            let warnings: Vec<String> = self
                .shows
                .shows
                .iter()
                .filter_map(|show| {
                    if let Some(error) = &show.error {
                        Some(format!("{}: {error}", show.title))
                    } else if show.last_checked > 0 && !show.catalogue_complete {
                        Some(format!("{}: history incomplete", show.title))
                    } else {
                        None
                    }
                })
                .collect();
            (!warnings.is_empty())
                .then(|| format!("Source warnings · {} · (R) retry", warnings.join("; ")))
        } else {
            self.selected_show()
                .and_then(|show| {
                    show.error.clone().or_else(|| {
                        (show.last_checked > 0 && !show.catalogue_complete)
                            .then(|| "History incomplete".into())
                    })
                })
                .map(|error| format!("Check: {error} · (Shift+R) retry"))
        };
        let summary_text = if inbox && self.show_controls.inflight.is_some() {"Loading inbox filter…".into()} else if inbox {
            let checking = self.shows_refreshing.len();
            let unchecked = self
                .shows
                .shows
                .iter()
                .filter(|show| show.last_checked == 0)
                .count();
            if checking > 0 {
                format!("Checking {checking} show(s)… · cached releases")
            } else if unchecked > 0 {
                format!("{unchecked} show(s) unchecked · (R) Check all")
            } else {
                format!("Cached releases · (R) Check all · (←) Browse shows")
            }
        } else if let Some(show) = self.selected_show() {
            if self.shows_refreshing.contains(&show.imdb) {
                "Checking EZTV… · cached releases".into()
            } else {
                let checked = if show.last_checked > 0 {
                    format!("checked {}", ago(show.last_checked))
                } else {
                    "never checked".into()
                };
                format!(
                    "{checked} · prefers {}",
                    show.quality.as_deref().unwrap_or(&self.cfg.show_quality)
                )
            }
        } else {
            String::new()
        };
        Paragraph::new(summary_text)
            .style(th.dimmed())
            .wrap(WrapMode::None)
            .render(summary, frame);
        let rows = episode_rows(releases_inner, warning.is_some());
        self.shows_rects.set([inner, rows[1]]);
        if let Some(warning) = &warning {
            Paragraph::new(warning.clone())
                .style(th.err())
                .wrap(WrapMode::Word)
                .render(rows[2], frame);
        }
        if self.episode_count() == 0 {
            let message = if self.shows.shows.is_empty() {
                "No shows followed yet.\n\n(N) Follow a title or IMDb id\n(F)ollow a Search or RSS item\n(Shift+F) Follow a torrent\n\nChecks find releases. Enter downloads your choice."
            } else if inbox {
                if !self.shows_refreshing.is_empty() {
                    "Checking for releases…\n\nNo unread cached releases."
                } else if self.shows.shows.iter().any(|show| show.last_checked == 0) {
                    "Not checked yet.\n\n(R) Check all shows."
                } else if warning.is_some() {
                    "No unread cached releases.\n\nSource warnings remain below.\n(←) Browse a show's full release history."
                } else {
                    "All caught up.\n\n(←) Browse a show's full release history.\n(R) Check for releases."
                }
            } else if self
                .selected_show()
                .is_some_and(|show| show.last_checked == 0)
            {
                "Not checked yet.\n(Shift+R) Check this show"
            } else {
                "No cached releases for this show.\n(Shift+R) Check · (2) Search · (Shift+O) EZTV"
            };
            Paragraph::new(message)
                .style(th.dimmed())
                .wrap(WrapMode::Word)
                .render(pad(rows[1]), frame);
            return;
        }
        let width = rows[1].width.saturating_sub(2) as usize;
        let metadata = if width >= 60 {
            episode_metadata_width(width)
        } else {
            0
        };
        let header = format!(
            "{} {:<7} {}{}",
            cell("STATE", 9),
            "RELEASE",
            cell(
                if inbox { "SHOW" } else { "TITLE" },
                width.saturating_sub(18 + metadata)
            ),
            if metadata > 0 {
                episode_metadata(None, width)
            } else {
                String::new()
            }
        );
        Paragraph::new(header)
            .style(th.dimmed())
            .wrap(WrapMode::None)
            .render(
                Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1),
                frame,
            );
        self.view_rows(
            MouseList::Episodes,
            rows[1],
            self.episode_count(),
            |i| {
                let (show, episode) = self.episode_at(i).expect("displayed release exists");
                layout::fingerprint(&(
                    episode,
                    &show.title,
                    show.year,
                    show.is_new(&episode.key),
                    show.grabbed.contains_key(&episode.key),
                    self.torrent_revision,
                ))
            },
            |i| {
                let (show, episode) = self.episode_at(i).expect("displayed release exists");
                let torrent = self.torrents.iter().find(|t| {
                    let r = crate::release::parse(&t.name);
                    r.title.eq_ignore_ascii_case(&show.title)
                        && r.episode.as_deref() == Some(&episode.key)
                });
                let unread = show.is_new(&episode.key);
                let state = if let Some(t) = torrent {
                    format::state_cell(t, self.store.torrents.get(&t.hash))
                } else if show.grabbed.contains_key(&episode.key) {
                    "+ Added  "
                } else if unread {
                    "● Unread "
                } else {
                    "✓ Read   "
                };
                let name = if inbox {
                    format!("{} · {}", show.label(), episode.title)
                } else {
                    episode.title.clone()
                };
                let key = format!("{:<7} ", episode.key);
                let style = if unread {
                    th.text().bold()
                } else {
                    th.dimmed()
                };
                Line::from_spans([
                    Span::styled(state, style),
                    Span::raw(" "),
                    Span::styled(key.clone(), style),
                    Span::styled(
                        readable_cell(
                            &name,
                            width.saturating_sub(10 + dw(&key) + metadata),
                            hovered_for(&self.ep_list, i, elapsed),
                        ),
                        style,
                    ),
                    Span::styled(
                        if metadata > 0 {
                            self.release_metadata(show,episode,width)
                        } else {
                            String::new()
                        },
                        th.dimmed(),
                    ),
                ])
            },
            frame,
        );
    }
}

pub(super) fn episode_metadata_width(width: usize) -> usize {
    if width >= 80 { 1 + 5 + 2 + 9 + 2 + 5 + 2 + 10 }
    else if width >= 60 { 1 + 5 + 2 + 9 + 2 + 5 }
    else { 1 + 5 + 2 + 5 }
}

pub(super) fn episode_metadata(episode: Option<&Episode>, width: usize) -> String {
    let (quality, size, seeds, date) = match episode {
        Some(e) => (e.quality.clone(), human_bytes(e.size), e.seeds.to_string(), if e.released > 0 { ymd(e.released) } else { String::new() }),
        None => ("QUAL".into(), "SIZE".into(), "SEEDS".into(), "DATE".into()),
    };
    if width >= 80 { format!(" {quality:>5}  {size:>9}  {seeds:>5}  {date:>10}") }
    else if width >= 60 { format!(" {quality:>5}  {size:>9}  {seeds:>5}") }
    else { format!(" {quality:>5}  {seeds:>5}") }
}
