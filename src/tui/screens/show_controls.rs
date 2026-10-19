//! Shows preferences are daemon intents; inbox filters only change the view.
use super::*;
#[derive(Default)]
pub(super) struct ShowControls {
    pub read: ReadFilter,
    pub quality: Option<String>,
    pub rows: Option<Vec<crate::api::Release>>,
    pub inflight: Option<(u64, u64)>,
    pub revision: u64,
    pub dirty: bool,
    pub requested: bool,
}
#[derive(Default, PartialEq, Clone, Copy)]
pub(super) enum ReadFilter {
    #[default]
    Unread,
    All,
    Read,
}
impl ReadFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::All => "all",
            Self::Read => "read",
        }
    }
}
fn quality(value: &str, clear: &str) -> Option<Option<String>> {
    if value == clear {
        return Some(None);
    }
    match value {
        "2160p" | "1080p" | "720p" | "480p" => Some(Some(value.into())),
        _ => None,
    }
}
impl TuiApp {
    pub(super) fn open_show_quality(&mut self) -> Cmd<Msg> {
        let Some(show) = self.release_show() else {
            return Cmd::none();
        };
        let (imdb, value) = (
            show.imdb.clone(),
            show.quality.clone().unwrap_or_else(|| "default".into()),
        );
        self.open_prompt(
            PromptKind::ShowQuality { imdb },
            "Show quality: 2160p / 1080p / 720p / 480p / default",
            false,
        );
        self.prompt.as_mut().unwrap().input.set_value(value);
        Cmd::none()
    }
    pub(super) fn apply_show_quality(&mut self, imdb: String, value: &str) -> Cmd<Msg> {
        let Some(quality) = quality(value, "default") else {
            self.set_status("Choose 2160p, 1080p, 720p, 480p or default", true);
            return Cmd::none();
        };
        self.service_action(
            "shows.set_quality",
            serde_json::json!({"imdb":imdb,"quality":quality}),
            "show quality saved",
        )
    }
    pub(super) fn open_show_filters(&mut self) -> Cmd<Msg> {
        let value = format!(
            "{} {}",
            self.show_controls.read.label(),
            self.show_controls.quality.as_deref().unwrap_or("all")
        );
        self.open_prompt(
            PromptKind::ShowFilters,
            "Inbox filter: unread / all / read, then all / 2160p / 1080p / 720p / 480p",
            false,
        );
        self.prompt.as_mut().unwrap().input.set_value(value);
        Cmd::none()
    }
    pub(super) fn apply_show_filters(&mut self, value: &str) -> Cmd<Msg> {
        let parts: Vec<_> = value.split_whitespace().take(3).collect();
        let read = match parts.first().copied() {
            Some("unread") => ReadFilter::Unread,
            Some("all") => ReadFilter::All,
            Some("read") => ReadFilter::Read,
            _ => {
                self.set_status("Read filter must be unread, all or read", true);
                return Cmd::none();
            }
        };
        let Some(quality) =
            quality(parts.get(1).copied().unwrap_or("all"), "all").filter(|_| parts.len() <= 2)
        else {
            self.set_status(
                "Quality filter must be all, 2160p, 1080p, 720p or 480p",
                true,
            );
            return Cmd::none();
        };
        self.show_controls.read = read;
        self.show_controls.quality = quality;
        self.show_controls.rows = None;
        self.show_controls.requested = false;
        self.show_controls.revision += 1;
        self.show_controls.dirty = true;
        self.show_list.borrow_mut().select(Some(0));
        self.shows_focus_eps = true;
        self.rebuild_inbox();
        self.sync_topics()
    }
    pub(super) fn full_show_inbox(&self) -> bool {
        self.screen == Screen::Shows
            && self.shows_inbox()
            && self.show_controls.read != ReadFilter::Unread
    }
    pub(super) fn refresh_show_filter(&mut self) -> Cmd<Msg> {
        if !self.full_show_inbox() {
            self.show_controls.rows = None;
            self.show_controls.requested = false;
            return Cmd::none();
        }
        if self.show_controls.rows.is_none() && !self.show_controls.requested {
            self.show_controls.dirty = true;
        }
        if !self.show_controls.dirty || self.show_controls.inflight.is_some() {
            return Cmd::none();
        }
        let Some(connection) = self.presence.clone().filter(|c| c.alive()) else {
            return Cmd::none();
        };
        let revision = self.show_controls.revision;
        self.show_controls.inflight = Some((connection.id, revision));
        self.show_controls.dirty = false;
        self.show_controls.requested = true;
        Cmd::task(move || {
            let result = connection
                .request(
                    "shows.releases",
                    serde_json::json!({"inbox":true,"unread_only":false}),
                )
                .and_then(|value| Ok(serde_json::from_value(value)?))
                .map_err(|e| e.to_string());
            Msg::ShowFilterRead {
                connection: connection.id,
                revision,
                result,
            }
        })
    }
    pub(super) fn show_filter_read(
        &mut self,
        connection: u64,
        revision: u64,
        result: Result<crate::api::ReleasesResult, String>,
    ) -> Cmd<Msg> {
        if self.presence.as_ref().is_none_or(|c| c.id != connection)
            || self.show_controls.inflight != Some((connection, revision))
        {
            return Cmd::none();
        }
        self.show_controls.inflight = None;
        if revision == self.show_controls.revision && self.full_show_inbox() {
            match result {
                Ok(value) if value.releases.len() <= 100000 => {
                    self.show_controls.rows = Some(value.releases);
                    self.sync_native_shows();
                }
                Ok(_) => self.set_status("Too many releases to display", true),
                Err(error) => {
                    self.set_status(error, true);
                    return Cmd::none();
                }
            }
        }
        self.refresh_show_filter()
    }
}
