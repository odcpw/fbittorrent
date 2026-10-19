use super::*;
macro_rules! log_action {
    ($id:expr,$key:expr,$label:expr,$intent:ident) => {
        action!(
            $id,
            Scope::Modal(ModalKind::Log),
            &[KeyChord::plain($key)],
            $label,
            30,
            |_| true,
            Some(|a| a.on_log_action(Intent::$intent)),
            T,
            None,
            None,
            None
        )
    };
}
pub(super) const ACTIONS: &[Action] = &[
    action!(
        "log.open",
        Scope::Global,
        &[KeyChord::plain(KeyCode::Char('L'))],
        "Log",
        90,
        |_| true,
        Some(TuiApp::toggle_log),
        TD,
        None,
        None,
        None
    ),
    log_action!("log.close", KeyCode::Escape, "Back", Cancel),
    action!(
        "log.toggle",
        Scope::Modal(ModalKind::Log),
        &[KeyChord::plain(KeyCode::Char('L'))],
        "Back",
        90,
        |_| true,
        Some(|a|a.on_log_action(Intent::Cancel)),
        T,
        None,
        None,
        None
    ),
    log_action!("log.level", KeyCode::Char('l'), "Level", Right),
    log_action!("log.source", KeyCode::Char('h'), "Source", Left),
    log_action!("log.up", KeyCode::Up, "Previous", Up),
    log_action!("log.down", KeyCode::Down, "Next", Down),
    log_action!("log.first", KeyCode::Home, "First", First),
    log_action!("log.last", KeyCode::End, "Last", Last),
    action!(
        "log.previous_tab",
        Scope::Modal(ModalKind::Log),
        &[KeyChord::plain(KeyCode::Char('['))],
        "Files",
        90,
        |a| !a.log_open && a.details.is_some(),
        Some(|a| a.details_tab(false)),
        T,
        None,
        None,
        None
    ),
    action!(
        "log.next_tab",
        Scope::Modal(ModalKind::Log),
        &[KeyChord::plain(KeyCode::Char(']'))],
        "Next tab",
        90,
        |a| !a.log_open && a.details.is_some(),
        Some(|a| a.details_tab(true)),
        T,
        None,
        None,
        None
    ),
];
