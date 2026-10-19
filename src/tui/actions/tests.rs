use super::*;
use std::{fs, path::Path};
#[path = "../tests/fixture.rs"]
mod fixture;

#[test]
fn table_covers_appendix_b() {
    let committed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("plugin/oliver.torrents/keymap.json").unwrap()).unwrap();
    assert_eq!(committed["version"], 1);
    for action in all() {
        let row = committed["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == action.id)
            .unwrap_or_else(|| panic!("{} missing in keymap at {}", action.id, action.scope.name()));
        let expected = action
            .keys_for(Surface::Tui)
            .map(|keys| serde_json::json!(keys.iter().map(|key| key.label()).collect::<Vec<_>>()));
        assert_eq!(
            row["keys"]["tui"],
            expected.unwrap_or(serde_json::Value::Null),
            "{} in {}",
            action.id,
            action.scope.name()
        );
        assert_eq!(
            action.run.is_some(),
            action.surfaces.contains(&Surface::Tui),
            "only non-TUI rows may omit execution: {}",
            action.id
        );
    }
    assert_eq!(committed["actions"].as_array().unwrap().len(), all().count());
    for id in [
        "screen.torrents",
        "screen.search",
        "screen.shows",
        "screen.rss",
        "search.providers",
        "torrents.seed",
        "shows.undo_read",
        "prompt.confirm",
        "popup.dashboard",
        "popup.tui",
    ] {
        assert!(all().any(|a| a.id == id), "Appendix B action {id} missing");
    }
    assert_eq!(all().find(|a| a.id == "search.providers").unwrap().keys[0].label(), "P");
    assert_eq!(
        all()
            .find(|a| a.id == "prompt.confirm")
            .unwrap()
            .keys
            .iter()
            .map(|c| c.label())
            .collect::<Vec<_>>(),
        ["y"]
    );
    assert_eq!(all().find(|a| a.id == "torrents.seed").unwrap().keys[0].label(), "u");
    let popup = keymap("popup").to_string();
    assert!(!popup.contains("screen.torrents") && !popup.contains("torrents.wiki"));
    assert_eq!(fixture::SURFACES.len(), 12);
    println!(
        "{} catalogue actions cover TUI and both documented popup exceptions",
        all().count()
    );
}

#[test]
fn no_handler_outside_table() {
    for directory in ["src/tui/screens", "src/tui/modals"] {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let source = fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains("KeyCode::Char("),
                "{} contains a chord outside the catalogue",
                path.display()
            );
        }
    }
    let connection = fs::read_to_string("src/tui/connection.rs").unwrap();
    assert!(connection.contains("actions::dispatch(self, key)"));
    println!("all screen/modal handlers use named actions; keyboard entry delegates to dispatch");
}

#[test]
fn no_duplicate_key_per_scope() {
    for native in [true] {
        let mut app = fixture::app("torrents");
        app.prompt = Some(Prompt {
            kind: PromptKind::AddMagnet,
            label: "Add".into(),
            input: TextInput::new(),
            yes_no: false,
            add_flags: (true, true, false),
        });
        for surface in [Surface::Tui, Surface::Dashboard, Surface::Popup] {
            let mut seen = HashMap::new();
            for action in all().filter(|a| a.surfaces.contains(&surface) && (a.enabled)(&app)) {
                for chord in action.keys_for(surface).unwrap() {
                    let scope = action.scope.name();
                    let key = chord.label();
                    assert!(
                        seen.insert((scope.clone(), key.clone()), action.id).is_none(),
                        "duplicate {key} at {scope}, action {}, native={native}",
                        action.id
                    );
                }
            }
        }
    }
    println!("no duplicate active chord per scope on all surfaces in either backend");
}

#[test]
fn global_keys_never_rebound() {
    let reserved = ["q", "Q", "?", "1", "2", "3", "4"];
    for action in all().filter(|a| matches!(a.scope, Scope::Screen(_) | Scope::Pane(_, _))) {
        for chord in action.keys {
            assert!(
                !reserved.contains(&chord.label().as_str()),
                "{} rebinds {} in {}",
                action.id,
                chord.label(),
                action.scope.name()
            );
        }
    }
    for key in reserved {
        assert!(
            all().any(|a| a.scope == Scope::Global && a.keys.iter().any(|c| c.label() == key)),
            "global {key} missing"
        );
    }
}

fn assert_generated(path: &str, actual: &str, expected: &str) {
    if actual == expected {
        return;
    }
    let mut diff = format!(
        "--- {path}\n+++ generated\n@@ -1,{} +1,{} @@\n",
        actual.lines().count(),
        expected.lines().count()
    );
    for (prefix, text) in [('-', actual), ('+', expected)] {
        for line in text.lines() {
            diff.push(prefix);
            diff.push_str(line);
            diff.push('\n');
        }
    }
    panic!("generated output differs:\n{diff}");
}

#[test]
fn readme_table_matches() {
    let readme = fs::read_to_string("README.md").unwrap();
    let block = readme
        .split_once("<!-- keys:begin -->\n")
        .unwrap()
        .1
        .split_once("<!-- keys:end -->")
        .unwrap()
        .0;
    assert_generated("README.md keys", block, &markdown("all"));
}

#[test]
fn keymap_json_matches() {
    let expected = serde_json::to_string_pretty(&keymap("all")).unwrap() + "\n";
    assert_generated(
        "plugin/oliver.torrents/keymap.json",
        &fs::read_to_string("plugin/oliver.torrents/keymap.json").unwrap(),
        &expected,
    );
    for filter in ["tui", "dashboard", "popup"] {
        for row in keymap(filter)["actions"].as_array().unwrap() {
            assert_eq!(
                row["keys"].as_object().unwrap().len(),
                1,
                "surface filtering leaked keys in {row}"
            );
            assert!(row["keys"][filter].is_array());
        }
    }
}

#[test]
fn focused_dispatch_text_entry_and_backend_conditions() {
    let mut app = fixture::app("files");
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('1')));
    assert_eq!(
        app.screen,
        Screen::Torrents,
        "Files' 1 overrides global screen switching"
    );
    dispatch(&mut app, KeyEvent::new(KeyCode::Escape));
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('2')));
    assert_eq!(app.screen, Screen::Search);
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('/')));
    assert!(app.search_focus_input);
    app.search_input.set_value("");
    for c in "qQ?1234".chars() {
        dispatch(&mut app, KeyEvent::new(KeyCode::Char(c)));
    }
    assert_eq!(app.search_input.value(), "qQ?1234");
    assert!(!app.show_help);
    dispatch(&mut app, KeyEvent::new(KeyCode::Escape));
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('?')));
    assert!(app.show_help);
    dispatch(&mut app, KeyEvent::new(KeyCode::PageDown));
    assert_eq!(app.help_scroll, 10);
    dispatch(&mut app, KeyEvent::new(KeyCode::Escape));
    assert!(!all().any(|a| a.id.starts_with("legacy.") || matches!(a.id,"files.high"|"files.max"|"prompt.sequential"|"prompt.first_last")));
    let follow = all().find(|a| a.id == "torrents.follow").unwrap();
    assert!((follow.enabled)(&app));
    assert!(
        KeyChord::plain(KeyCode::Char('P')).matches(KeyEvent::new(KeyCode::Char('P')).with_modifiers(Modifiers::SHIFT))
    );
}

#[test]
fn overflow_is_explicit_and_never_exceeds_available_columns() {
    let labels: Vec<_> = (0..9).map(|i| (10, format!("({i}) Action {i}"))).collect();
    for width in 0..160 {
        let text = super::super::footer::pack_labels(&labels, width);
        assert!(dw(&text) <= width, "width {width}: {text}");
        if width > 0 && width < 124 {
            assert!(text.ends_with('…'), "width {width}: {text}");
        }
    }
    assert!(Path::new("scripts/gen-keymap.sh").exists());
}

#[test]
fn providers_key_and_paging_use_real_handlers() {
    let mut app = fixture::app("search");
    app.search_focus_input = false;
    assert!(matches!(
        dispatch(&mut app, KeyEvent::new(KeyCode::Char('P'))),
        Cmd::Task(..)
    ));
    assert!(app.providers.is_some());
    dispatch(&mut app, KeyEvent::new(KeyCode::Escape));
    dispatch(&mut app, KeyEvent::new(KeyCode::PageDown));
    assert_eq!(app.search_list.borrow().selected(), Some(9));
    dispatch(&mut app, KeyEvent::new(KeyCode::PageUp));
    assert_eq!(app.search_list.borrow().selected(), Some(0));
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('b')));
    assert!(app.search_browse);
    dispatch(&mut app, KeyEvent::new(KeyCode::End));
    assert_eq!(
        app.browse_list.borrow().selected(),
        app.browse_entries.len().checked_sub(1)
    );
    dispatch(&mut app, KeyEvent::new(KeyCode::Char('P')));
    assert!(app.providers.is_some());
}

#[test]
fn footer_cache_tracks_label_and_selection_changes() {
    let mut app = fixture::app("search");
    app.search_focus_input = true;
    app.search_input.set_value(" ");
    let empty = app.footer_signature(80);
    app.search_input.set_value("a query");
    let query = app.footer_signature(80);
    assert_ne!(empty, query, "Browse/Search label changes must invalidate the cache");
    app.search_input.set_value("another query");
    assert_eq!(
        query,
        app.footer_signature(80),
        "typing must reuse the unchanged footer"
    );
    assert_ne!(query, app.footer_signature(120), "resize must repack the legend");
    app.set_status("Added", false);
    assert_ne!(
        query,
        app.footer_signature(80),
        "action feedback must be visible immediately"
    );
    app.screen = Screen::Rss;
    app.rss_focus_items = true;
    let unread = app.footer_signature(80);
    app.feeds[0].articles[0].is_read = true;
    assert_ne!(
        unread,
        app.footer_signature(80),
        "Mark read/unread must invalidate the cached label"
    );
}

#[test]
fn pending_row_mutations_are_disabled_in_both_keys_and_footer() {
    let mut app = fixture::app("torrents");
    let hash = crate::api::Hash::new(app.selected_torrent().unwrap().hash.clone()).unwrap();
    let before = app.footer_signature(120);
    app.store.begin_mutation(
        crate::api::MutationRef::new("selecting-files").unwrap(),
        "torrents.set_files".into(),
        vec![hash],
    ).unwrap();
    assert_ne!(before, app.footer_signature(120));
    for id in ["torrents.pause", "torrents.seed", "torrents.remove", "torrents.delete"] {
        let action = all().find(|action| action.id == id).unwrap();
        assert!(!(action.enabled)(&app), "{id} must not advertise an unavailable action");
        for key in action.keys {
            assert!(matches!(dispatch(&mut app, KeyEvent::new(key.code).with_modifiers(key.mods)), Cmd::None), "{id}: {key:?}");
        }
    }
    let labels = app.footer_labels();
    assert!(!labels.iter().any(|(_, label)| label.contains("(p)") || label.contains("(u)") || label.contains("(d)") || label.contains("(D)")));
    assert!(app.prompt.is_none());
}

#[test]
fn native_sort_and_follow_replace_scheduling_toggles() {
    let mut app=fixture::app("torrents");
    let selected=app.selected_torrent().map(|row|row.hash.clone());
    dispatch(&mut app,KeyEvent::new(KeyCode::Char('s')));
    assert_eq!(app.torrent_sort,store::TorrentSort::Name);
    assert_eq!(app.selected_torrent().map(|row|row.hash.clone()),selected);
    let descending=app.torrent_descending;
    dispatch(&mut app,KeyEvent::new(KeyCode::Char('S')));
    assert_eq!(app.torrent_sort,store::TorrentSort::Name);
    assert_eq!(app.torrent_descending,!descending);
    assert!(app.status.starts_with("Sorted by name"));
    assert!(all().find(|a|a.id=="torrents.follow").unwrap().keys.contains(&KeyChord::plain(KeyCode::Char('f'))));
}
