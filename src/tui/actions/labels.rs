//! Context-sensitive labels; availability remains in each catalogue row.
use super::*;

pub(super) fn label(id: &str, default: &str, app: &TuiApp) -> Option<String> {
    let id=match id { "details.pause"=>"torrents.pause", "details.seed"=>"torrents.seed", _=>id };
    Some(
        match id {
            "torrents.pause" => {
                let torrent = app.selected_torrent()?;
                if app.complete_no_seed(torrent) {
                    return None;
                }
                if app.torrent_paused(torrent) {
                    "Resume"
                } else {
                    "Pause"
                }
            }
            "torrents.seed" => {
                let torrent = app.selected_torrent()?;
                if app.complete_no_seed(torrent) {
                    "Seed"
                } else if app.seed_allowed(torrent) {
                    "Seed off"
                } else {
                    "Seed on"
                }
            }
            "search.submit" if app.search_input.value().trim().is_empty() => "Browse lists",
            "browse.open" => {
                if app
                    .browse_list
                    .borrow()
                    .selected()
                    .and_then(|i| app.browse_entries.get(i))
                    .is_some_and(|entry| matches!(entry.source, browse::Source::Site(_)))
                {
                    "Open site"
                } else {
                    "Load list"
                }
            }
            "rss.primary" => {
                if app.rss_focus_items {
                    "Download"
                } else {
                    "Items"
                }
            }
            "rss.mark" => {
                if !app.rss_focus_items {
                    return None;
                }
                let item = app
                    .selected_feed()
                    .and_then(|feed| app.item_list.borrow().selected().and_then(|i| feed.articles.get(i)))?;
                if app.cfg.native() && item.is_read {
                    "Mark unread"
                } else {
                    "Mark read"
                }
            }
            "shows.primary" => {
                if app.shows_focus_eps {
                    "Download"
                } else {
                    "Releases"
                }
            }
            "shows.mark" => {
                if !app.shows_focus_eps {
                    return None;
                }
                let (show, episode) = app.selected_episode()?;
                if show.is_new(&episode.key) {
                    "Mark read"
                } else {
                    "Mark unread"
                }
            }
            "rss.left" => {
                if !app.rss_focus_items {
                    return None;
                }
                "Feeds"
            }
            "shows.left" => {
                if !app.shows_focus_eps {
                    return None;
                }
                "Shows"
            }
            "rss.right" => {
                if app.rss_focus_items {
                    return None;
                }
                "Items"
            }
            "shows.right" => {
                if app.shows_focus_eps {
                    return None;
                }
                "Releases"
            }
            "shows.refresh" => "Check all",
            "shows.reload" => "Check show",
            "prompt.confirm" if app.prompt.as_ref().is_some_and(|p|p.kind==PromptKind::Shutdown) => "Stop fbittorrent",
        "prompt.submit" => match app.prompt.as_ref()?.kind {
                PromptKind::FollowShow => "Follow",
                PromptKind::AddFeed => "Add feed",
                _ => "Add",
            },
            _ => default,
        }
        .to_owned(),
    )
}
