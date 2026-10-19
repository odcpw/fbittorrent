//! The port is an API settings draft, with validation before any mutation.
use super::*;
#[path = "fixture.rs"]
mod fixture;
#[test]
fn peer_port_edit_cancel_range_and_patch() {
    let mut app = fixture::app("settings");
    for _ in 0..settings_fields::SETTINGS_ROWS-settings_fields::PORT_ROW {app.on_key(KeyEvent::new(KeyCode::Up));}
    assert_eq!(
        app.settings.as_ref().unwrap().list.borrow().selected(),
        Some(settings_fields::PORT_ROW)
    );
    app.on_key(KeyEvent::new(KeyCode::Right));
    assert_eq!(actions::focused(&app), actions::Scope::Prompt);
    app.on_key(KeyEvent::new(KeyCode::Char('a')).with_modifiers(Modifiers::CTRL));
    for c in "51413".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(c)));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter));
    assert_eq!(app.settings.as_ref().unwrap().cfg.peer_port, 51413);
    assert_eq!(app.cfg.peer_port, 6882);
    for input in ["65536", "-1", "", "1M", "not a port"] {
        app.apply_peer_port_input(input);
        assert_eq!(app.settings.as_ref().unwrap().cfg.peer_port, 51413);
        assert!(!app.settings.as_ref().unwrap().error.is_empty());
    }
    let modal = app.settings.as_ref().unwrap();
    assert_eq!(
        settings_fields::settings_patch(&modal.original, &modal.cfg),
        serde_json::json!({"peer_port":51413})
    );
    app.edit_peer_port();
    app.prompt.as_mut().unwrap().input.set_value("12345");
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert_eq!(app.settings.as_ref().unwrap().cfg.peer_port, 51413);
    app.apply_peer_port_input("0");
    assert_eq!(app.settings.as_ref().unwrap().cfg.peer_port, 0);
    assert!(super::golden::render(&app, 80, 24).contains("Automatic (0)"));
    app.on_key(KeyEvent::new(KeyCode::Escape));
    assert_eq!(app.cfg.peer_port, 6882);
}
