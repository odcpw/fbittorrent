use super::*;
#[path = "fixture.rs"]
mod fixture;
#[test]
fn rename_prompt_prefills_cancel_and_empty_submission() {
    for surface in ["torrents", "details"] {
        let mut app = fixture::app(surface);
        let name = app.selected_torrent().unwrap().name.clone();
        app.on_key(KeyEvent::new(KeyCode::Char('N')));
        assert_eq!(actions::focused(&app), actions::Scope::Prompt);
        assert_eq!(app.prompt.as_ref().unwrap().input.value(), name);
        assert!(golden::render(&app, 100, 30).contains("Rename to:"));
        app.on_key(KeyEvent::new(KeyCode::Escape));
        assert!(app.prompt.is_none());
        assert_eq!(app.selected_torrent().unwrap().name, name);
        app.on_key(KeyEvent::new(KeyCode::Char('N')));
        app.prompt.as_mut().unwrap().input.set_value("");
        let cmd = app.on_key(KeyEvent::new(KeyCode::Enter));
        assert!(
            matches!(cmd, Cmd::Task(..)),
            "empty name must dispatch the clear operation"
        );
        assert!(app.prompt.is_none());
        assert_eq!(
            app.selected_torrent().unwrap().name,
            name,
            "no optimistic rename"
        );
    }
}
