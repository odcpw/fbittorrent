//! Details actions use the same handlers as the torrent screen.
use super::*;
macro_rules! detail {
    ($id:expr,$key:expr,$label:expr,$intent:ident) => {
        action!(
            $id,
            Scope::Modal(ModalKind::Details),
            &[KeyChord::plain(KeyCode::Char($key))],
            $label,
            30,
            |a| a.selected_torrent().is_some_and(|t| !matches!(
                Intent::$intent,
                Intent::Pause | Intent::Seed | Intent::Remove | Intent::DeleteFiles
            ) || a
                .store
                .pending_for_key(&t.hash)
                .is_none()),
            Some(|a| a.on_torrents_action(Intent::$intent)),
            T,
            None,
            None,
            None
        )
    };
}
pub(super) const ACTIONS: &[Action] = &[
    action!(
        "details.close",
        Scope::Modal(ModalKind::Details),
        &[
            KeyChord::plain(KeyCode::Escape),
            KeyChord::plain(KeyCode::Char('h'))
        ],
        "Back",
        10,
        |_| true,
        Some(TuiApp::close_details),
        T,
        None,
        None,
        None
    ),
    action!(
        "details.next",
        Scope::Modal(ModalKind::Details),
        &[KeyChord::plain(KeyCode::Char(']'))],
        "Next tab",
        90,
        |_| true,
        Some(|a| a.details_tab(true)),
        T,
        None,
        None,
        None
    ),
    action!(
        "details.previous",
        Scope::Modal(ModalKind::Details),
        &[KeyChord::plain(KeyCode::Char('['))],
        "Previous tab",
        90,
        |_| true,
        Some(|a| a.details_tab(false)),
        T,
        None,
        None,
        None
    ),
    action!(
        "files.next_tab",
        Scope::Modal(ModalKind::Files),
        &[KeyChord::plain(KeyCode::Char(']'))],
        "Next tab",
        90,
        |a| a.details.is_some(),
        Some(|a| a.details_tab(true)),
        T,
        None,
        None,
        None
    ),
    action!(
        "files.previous_tab",
        Scope::Modal(ModalKind::Files),
        &[KeyChord::plain(KeyCode::Char('['))],
        "Overview",
        90,
        |a| a.details.is_some(),
        Some(|a| a.details_tab(false)),
        T,
        None,
        None,
        None
    ),
    detail!("details.open", 'o', "Open", Open),
    detail!("details.pause", 'p', "Pause/resume", Pause),
    detail!("details.seed", 'u', "Seed", Seed),
    detail!("details.remove", 'd', "Remove", Remove),
    detail!("details.delete", 'D', "Delete files", DeleteFiles),
    detail!("details.imdb", 'i', "IMDb", Imdb),
    detail!("details.wiki", 'w', "Wikipedia", Wiki),
];
