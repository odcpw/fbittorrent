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
    action!("details.rename",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('N'))],"Rename",40,
        |a|a.cfg.native() && a.selected_torrent().is_some(),Some(TuiApp::rename_prompt),T,None,None,None),
    action!("trackers.add",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('+'))],"Add",15,
        TuiApp::can_edit_trackers,Some(TuiApp::prompt_add_tracker),TD,None,None,None),
    action!("trackers.remove",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('x'))],"Remove",16,
        |a|a.can_edit_trackers() && a.details.as_ref().is_some_and(|d|!d.trackers.urls.is_empty()),Some(TuiApp::prompt_remove_tracker),TD,None,None,None),
    action!("trackers.reannounce",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('e'))],"Reannounce",20,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Trackers&&!d.trackers.busy)&&a.online,Some(|a|a.load_trackers(true)),TD,None,None,None),
    action!("trackers.refresh",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('r'))],"Refresh",40,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Trackers&&!d.trackers.busy)&&a.online,Some(|a|a.load_trackers(false)),T,None,None,None),
    action!("trackers.down",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Down),KeyChord::plain(KeyCode::Char('j'))],"Next tracker",90,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Trackers),Some(|a|a.scroll_trackers(true)),T,None,None,None),
    action!("trackers.up",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Up),KeyChord::plain(KeyCode::Char('k'))],"Previous tracker",90,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Trackers),Some(|a|a.scroll_trackers(false)),T,None,None,None),
    action!("peers.down",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Down),KeyChord::plain(KeyCode::Char('j'))],"Next peer",90,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Peers),Some(|a|a.scroll_peers(true)),T,None,None,None),
    action!("peers.up",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Up),KeyChord::plain(KeyCode::Char('k'))],"Previous peer",90,
        |a|a.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Peers),Some(|a|a.scroll_peers(false)),T,None,None,None),
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
    action!("details.move",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('m'))],"Move…",40,
        |a|a.overview_available("move"),Some(|a|a.overview_prompt(false)),TD,None,None,None),
    action!("details.download_cap",Scope::Modal(ModalKind::Details),&[KeyChord::plain(KeyCode::Char('c'))],"Download cap…",40,
        |a|a.overview_available("download_limit"),Some(|a|a.overview_prompt(true)),TD,None,None,None),
    detail!("details.open", 'o', "Open", Open),
    detail!("details.pause", 'p', "Pause/resume", Pause),
    detail!("details.seed", 'u', "Seed", Seed),
    detail!("details.remove", 'd', "Remove", Remove),
    detail!("details.delete", 'D', "Delete files", DeleteFiles),
    detail!("details.imdb", 'i', "IMDb", Imdb),
    detail!("details.wiki", 'w', "Wikipedia", Wiki),
];
