use super::*;
pub(super) const ACTIONS: &[Action] = &[
    action!("clipboard.offer_add",Scope::Modal(ModalKind::Clipboard),&[KeyChord::plain(KeyCode::Char('y'))],"Add",10,
        TuiApp::clipboard_visible,Some(TuiApp::clipboard_accept),T,None,None,None),
    action!("clipboard.offer_dismiss",Scope::Modal(ModalKind::Clipboard),&[KeyChord::plain(KeyCode::Escape)],"Dismiss",10,
        TuiApp::clipboard_visible,Some(TuiApp::clipboard_dismiss),T,None,None,None),
];
