//! Resolve keyboard input without allocating or duplicating chord definitions.
use super::*;

pub(super) fn pane(app: &TuiApp, screen: Screen) -> Scope {
    match screen {
        Screen::Torrents => Scope::Screen(screen),
        Screen::Search => Scope::Pane(
            screen,
            if app.search_focus_input {
                Pane::Query
            } else if app.search_browse {
                Pane::Browse
            } else {
                Pane::Results
            },
        ),
        Screen::Rss => Scope::Pane(screen, if app.rss_focus_items { Pane::Items } else { Pane::Feeds }),
        Screen::Shows => Scope::Pane(
            screen,
            if app.shows_focus_eps {
                Pane::Releases
            } else {
                Pane::Shows
            },
        ),
    }
}
pub(in crate::tui) fn focused(app: &TuiApp) -> Scope {
    if app.clipboard_visible() {return Scope::Modal(ModalKind::Clipboard);}
    if app.prompt.is_some() {
        Scope::Prompt
    } else if app.settings.is_some() {
        Scope::Modal(ModalKind::Settings)
    } else if app.show_help {
        Scope::Modal(ModalKind::Help)
    } else if app.log_open || app.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Log) {
        Scope::Modal(ModalKind::Log)
    } else if app.poster_open {
        Scope::Modal(ModalKind::Poster)
    } else if app.files.is_some() {
        Scope::Modal(ModalKind::Files)
    } else if app.details.is_some() {
        Scope::Modal(ModalKind::Details)
    } else if app.providers.is_some() {
        Scope::Modal(ModalKind::Providers)
    } else if app.daemon_stopped {
        Scope::Modal(ModalKind::Stopped)
    } else {
        pane(app, app.screen)
    }
}
pub(in crate::tui) fn scopes(scope: Scope) -> [Option<Scope>; 3] {
    [
        Some(scope),
        if let Scope::Pane(screen, _) = scope {
            Some(Scope::Screen(screen))
        } else {
            None
        },
        Some(Scope::Global),
    ]
}
pub(in crate::tui) fn dispatch(app: &mut TuiApp, key: KeyEvent) -> Cmd<Msg> {
    if key.kind == KeyEventKind::Release {
        return Cmd::none();
    }
    app.clear_pointer_hover();
    let scope = focused(app);
    dispatch_scope(app, scope, key)
}
pub(in crate::tui) fn dispatch_scope(app: &mut TuiApp, requested: Scope, key: KeyEvent) -> Cmd<Msg> {
    if key.kind == KeyEventKind::Release {
        return Cmd::none();
    }
    let scope = if let Scope::Screen(screen) = requested {
        pane(app, screen)
    } else {
        requested
    };
    if scope == Scope::Prompt && app.prompt.as_ref().is_some_and(|p| p.yes_no) {
        let confirm = all().find(|a| a.id == "prompt.confirm").unwrap();
        // A search batch is a download, not a destructive choice: Enter confirms it.
        let batch = app.prompt.as_ref().is_some_and(|p| matches!(p.kind, PromptKind::ConfirmSearchBatch { .. }));
        let intent = if confirm.keys.iter().any(|chord| chord.matches(key)) {
            Intent::Confirm
        } else if batch && KeyChord::plain(KeyCode::Enter).matches(key) {
            Intent::Primary
        } else {
            Intent::Cancel
        };
        return app.on_prompt_action(intent);
    }
    let editing = scope == Scope::Pane(Screen::Search, Pane::Query)
        || (scope == Scope::Prompt && app.prompt.as_ref().is_some_and(|p| !p.yes_no));
    // Widgets own text editing, including printable q/Q/?/1-4 inside an input.
    // Its explicit Enter/Esc/Down/F-key commands still come from the catalogue.
    if editing {
        if let Some(action) =
            all().find(|a| a.scope == scope && a.keys.iter().any(|c| c.matches(key)) && (a.enabled)(app))
        {
            return invoke(action,app,key);
        }
        if !matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            if let Some(action) = all().find(|a| {
                a.id == "ui.close"
                    && a.keys
                        .iter()
                        .any(|c| c.mods.contains(Modifiers::CTRL) && c.matches(key))
            }) {
                return invoke(action,app,key);
            }
            if scope == Scope::Prompt {
                if let Some(prompt) = app.prompt.as_mut() {
                    prompt.input.handle_event(&Event::Key(key));
                }
            } else {
                app.search_input.handle_event(&Event::Key(key));
            }
            return Cmd::none();
        }
    }
    for candidate in scopes(scope).into_iter().flatten() {
        let mut matched = false;
        for action in all().filter(|a| a.scope == candidate && a.surfaces.contains(&Surface::Tui)) {
            if !action.keys.iter().any(|chord| chord.matches(key)) {
                continue;
            }
            matched = true;
            if (action.enabled)(app) {
                return invoke(action,app,key);
            }
        }
        if matched && scope != Scope::Prompt {
            return Cmd::none();
        }
    }
    if scope == Scope::Prompt && app.prompt.as_ref().is_some_and(|p| p.yes_no) {
        return app.on_prompt_action(Intent::Cancel);
    }
    Cmd::none()
}
pub(super) fn clipboard(app: &mut TuiApp) -> Cmd<Msg> {
    let options = app.add_opts();
    app.action_result(move |client| {
        let out = std::process::Command::new("wl-paste").arg("--no-newline").output()?;
        let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !(value.starts_with("magnet:") || value.starts_with("http")) {
            anyhow::bail!("clipboard holds no magnet link or URL");
        }
        client.add_url(&value, &options)
    })
}
pub(super) fn refresh(app: &mut TuiApp) -> Cmd<Msg> {
    if app.online {
        return app.poll();
    }
    if app.presence.as_ref().is_some_and(|connection|connection.alive()) {app.poll()} else {app.start_daemon()}
}

pub(super) fn query_submit(app: &mut TuiApp) -> Cmd<Msg> {
    if !app.search_input.value().trim().is_empty() {
        return app.start_search();
    }
    app.search_focus_input = false;
    app.search_input.set_focused(false);
    app.search_browse = true;
    if app.browse_list.borrow().selected().is_none() {
        app.browse_list.borrow_mut().select(Some(0));
    }
    Cmd::none()
}
pub(super) fn query_leave(app: &mut TuiApp) -> Cmd<Msg> {
    app.search_focus_input = false;
    app.search_input.set_focused(false);
    if app.search_list.borrow().selected().is_none() && !app.search_results.is_empty() {
        app.search_list.borrow_mut().select(Some(0));
    }
    Cmd::none()
}
pub(super) fn page(app: &mut TuiApp, forward: bool) -> Cmd<Msg> {
    let (state, count) = match app.screen {
        Screen::Torrents => (&app.tor_list, app.torrents.len()),
        Screen::Search if app.search_browse => (&app.browse_list, app.browse_entries.len()),
        Screen::Search => (&app.search_list, app.search_results.len()),
        Screen::Rss if app.rss_focus_items => (&app.item_list, app.selected_feed().map_or(0, |f| f.articles.len())),
        Screen::Rss => (&app.feed_list, app.feeds.len()),
        Screen::Shows if app.shows_focus_eps => (&app.ep_list, app.episode_count()),
        Screen::Shows => (&app.show_list, app.shows.shows.len() + 1),
    };
    let current = state.borrow().selected().unwrap_or(0);
    let next = if forward {
        current.saturating_add(10).min(count.saturating_sub(1))
    } else {
        current.saturating_sub(10)
    };
    state.borrow_mut().select((count > 0).then_some(next));
    if app.screen == Screen::Shows && !app.shows_focus_eps {
        app.select_show_parent(next);
    }
    Cmd::none()
}
pub(super) fn help(app: &mut TuiApp, intent: Intent) -> Cmd<Msg> {
    let max = app.help_max_scroll();
    match intent {
        Intent::Down => app.help_scroll = app.help_scroll.saturating_add(1).min(max),
        Intent::Up => app.help_scroll = app.help_scroll.saturating_sub(1),
        Intent::Right => app.help_scroll = app.help_scroll.saturating_add(10).min(max),
        Intent::Left => app.help_scroll = app.help_scroll.saturating_sub(10),
        Intent::First => app.help_scroll = 0,
        Intent::Last => app.help_scroll = max,
        _ => app.show_help = false,
    }
    Cmd::none()
}

fn invoke(action:&Action,app:&mut TuiApp,key:KeyEvent)->Cmd<Msg> {
    let command=action.run.map_or_else(Cmd::none,|run|run(app));
    if action.id=="ui.close" && key.modifiers.contains(Modifiers::CTRL) { app.quit_notice=false; }
    command
}
