use super::*;
pub(super) const ACTIONS: &[Action] = &[
    action!(
        "poster.toggle",
        Scope::Global,
        &[KeyChord::plain(KeyCode::Char('v'))],
        "Card",
        20,
        |a| a.hovered_title().is_some(),
        Some(TuiApp::toggle_poster),
        T,
        None,
        None,
        None
    ),
    action!(
        "poster.close",
        Scope::Modal(ModalKind::Poster),
        &[
            KeyChord::plain(KeyCode::Escape),
            KeyChord::plain(KeyCode::Char('v'))
        ],
        "Back",
        10,
        |_| true,
        Some(TuiApp::close_poster),
        T,
        None,
        None,
        None
    ),
    action!(
        "poster.imdb",
        Scope::Modal(ModalKind::Poster),
        &[KeyChord::plain(KeyCode::Char('i'))],
        "IMDb",
        20,
        |_| true,
        Some(TuiApp::poster_imdb),
        T,
        None,
        None,
        None
    ),
    action!(
        "poster.wiki",
        Scope::Modal(ModalKind::Poster),
        &[KeyChord::plain(KeyCode::Char('w'))],
        "Wikipedia",
        20,
        |_| true,
        Some(TuiApp::poster_wiki),
        T,
        None,
        None,
        None
    ),
];
