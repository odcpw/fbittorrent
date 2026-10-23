//! Open actions follow visible torrent/file identity rather than a stale list row.
use super::*;
#[path="fixture.rs"]
mod fixture;

#[test]
fn file_and_folder_keys_have_distinct_catalogue_actions_and_help() {
    for (surface, ids) in [
        ("torrents", ["torrents.open", "torrents.open_folder"]),
        ("details", ["details.open", "details.open_folder"]),
        ("files", ["files.open", "files.open_folder"]),
    ] {
        let mut app=fixture::app(surface);
        for (id, key, label) in [(ids[0], 'o', "Open file"), (ids[1], 'O', "Open folder")] {
            let action=actions::all().find(|action|action.id==id).unwrap();
            assert!((action.enabled)(&app));
            assert_eq!((action.label)(&app).as_deref(),Some(label));
            assert!(app.footer_labels().iter().any(|(_,text)|text.contains(&format!("({key}) {label}"))));
            assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char(key))),Cmd::Task(..)));
            assert!(app.prompt.is_none(), "opening never asks to change the torrent");
        }
    }
    let help=actions::help_text();
    assert!(help.contains("(o) Open file") && help.contains("(O) Open folder"));
    for surface in ["tui", "dashboard", "popup"] {
        let keys=actions::keymap(surface);
        let action=keys["actions"].as_array().unwrap().iter()
            .find(|action|action["id"]=="torrents.open_folder").unwrap();
        assert_eq!(action["keys"][surface],serde_json::json!(["O"]));
    }
}

#[test]
fn file_modal_and_details_keep_their_hash_when_list_selection_changes() {
    for surface in ["files", "details"] {
        let mut app=fixture::app(surface);
        let hash=app.selected_torrent().unwrap().hash.clone();
        app.tor_list.borrow_mut().select(Some(1));
        assert_eq!(app.selected_torrent().unwrap().hash,hash);
        app.torrents.reverse();
        assert_eq!(app.selected_torrent().unwrap().hash,hash);
        assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('O'))),Cmd::Task(..)));
        app.torrents.retain(|torrent|torrent.hash!=hash);
        assert!(app.selected_torrent().is_none(), "removed modal rows must never open a different torrent");
        for key in ['o','O'] {
            assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char(key))),Cmd::None));
        }
    }
}

#[test]
fn files_open_requires_a_valid_loaded_selection_but_folder_does_not() {
    let mut app=fixture::app("files");
    let ready=app.footer_signature(120);
    app.files.as_mut().unwrap().loading=true;
    assert_ne!(ready,app.footer_signature(120));
    assert!(!app.can_open_file());
    assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('o'))),Cmd::None));
    assert!(matches!(app.on_key(KeyEvent::new(KeyCode::Char('O'))),Cmd::Task(..)));
    app.files.as_mut().unwrap().loading=false;
    assert_eq!(ready,app.footer_signature(120));
    app.files.as_mut().unwrap().error="file list unavailable".into();
    assert_ne!(ready,app.footer_signature(120));
    assert!(!app.can_open_file());
    app.files.as_mut().unwrap().error.clear();
    app.files.as_ref().unwrap().list.borrow_mut().select(Some(999));
    assert!(!app.can_open_file());
    app.files.as_ref().unwrap().list.borrow_mut().select(Some(1));
    assert!(app.can_open_file());
    app.files.as_mut().unwrap().files[1].index=-1;
    assert!(!app.can_open_file());
}
