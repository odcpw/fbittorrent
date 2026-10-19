//! The authoritative action catalogue, in Appendix B group order.
use super::dispatch::{clipboard, help, page, query_leave, query_submit, refresh};
use super::*;
const T: &[Surface] = &[Surface::Tui];
const TD: &[Surface] = &[Surface::Tui, Surface::Dashboard];
const TDP: &[Surface] = &[Surface::Tui, Surface::Dashboard, Surface::Popup];
const D: &[Surface] = &[Surface::Dashboard];
const P: &[Surface] = &[Surface::Popup];
macro_rules! action {
    ($id:expr,$scope:expr,$keys:expr,$label:expr,$priority:expr,$enabled:expr,$run:expr,$surfaces:expr,$dashboard:expr,$popup:expr,$note:expr) => {
        Action {
            id: $id,
            scope: $scope,
            keys: $keys,
            label: |app| crate::tui::actions::labels::label($id, $label, app),
            doc_label: $label,
            priority: $priority,
            enabled: $enabled,
            run: $run,
            surfaces: $surfaces,
            dashboard_keys: $dashboard,
            popup_keys: $popup,
            surface_note: $note,
        }
    };
}
mod details;
mod log;
mod global;
mod clipboard;
mod modals;
mod prompt;
mod poster;
mod rss;
mod search;
mod shows;
mod torrents;
pub(super) const TABLE: &[&[Action]] = &[
    global::ACTIONS,
    clipboard::ACTIONS,
    global::STOPPED,
    details::ACTIONS,
    log::ACTIONS,
    torrents::ACTIONS,
    search::ACTIONS,
    rss::ACTIONS,
    shows::ACTIONS,
    modals::ACTIONS,
    prompt::ACTIONS,
    poster::ACTIONS,
];
