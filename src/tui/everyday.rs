//! Everyday commands reuse typed sorting and confirmed daemon mutations.
use super::*;

impl TuiApp {
    pub(super) fn keyboard_torrent_sort(&mut self, reverse: bool) -> Cmd<Msg> {
        let columns = [
            Column::Name,
            Column::Size,
            Column::Progress,
            Column::Download,
            Column::State,
            Column::Eta,
            Column::Peers,
            Column::Uploaded,
        ];
        let index = columns
            .iter()
            .position(|c| c.sort_field() == Some(self.torrent_sort));
        let column = if reverse {
            index.map(|i| columns[i]).unwrap_or(Column::Uploaded)
        } else {
            columns[index.map_or(0, |i| (i + 1) % columns.len())]
        };
        self.sort_torrent_column(column)
    }
    pub(super) fn keyboard_search_sort(&mut self, reverse: bool) -> Cmd<Msg> {
        let columns = [
            Column::Name,
            Column::Size,
            Column::Seeds,
            Column::Rating,
            Column::Uploaded,
            Column::Provider,
            Column::Peers,
        ];
        let index = columns
            .iter()
            .position(|c| c.search_field() == Some(self.search_sort))
            .unwrap_or(0);
        self.sort_search_column(
            columns[if reverse {
                index
            } else {
                (index + 1) % columns.len()
            }],
        )
    }
    pub(super) fn copy_search_magnet(&mut self) -> Cmd<Msg> {
        let magnet = self.selected_result().and_then(|r| {
            self.wire_result(r)
                .and_then(|row| row.magnet.clone())
                .or_else(|| {
                    r.file_url
                        .starts_with("magnet:")
                        .then(|| r.file_url.clone())
                })
        });
        self.copy_magnet(magnet)
    }
    pub(super) fn copy_torrent_magnet(&mut self) -> Cmd<Msg> {
        let magnet = self.selected_torrent().map(|t| {
            format!(
                "magnet:?xt=urn:btih:{}&dn={}",
                t.hash,
                crate::qbt::urlencode(&t.name)
            )
        });
        self.copy_magnet(magnet)
    }
    fn copy_magnet(&mut self, magnet: Option<String>) -> Cmd<Msg> {
        let Some(magnet) = magnet else {
            self.set_status(
                "No magnet supplied by this provider; use O for its source page",
                true,
            );
            return Cmd::none();
        };
        Cmd::task(move || {
            Msg::ActionDone(
                write_clipboard(&magnet)
                    .map(|()| "copied magnet".into())
                    .map_err(|e| format!("Copy magnet: {e:#}")),
            )
        })
    }
    pub(super) fn removal_undo_available(&self) -> bool {
        self.online
            && !self.service_busy
            && self
                .removal_undo
                .as_ref()
                .is_some_and(|(_, until)| *until > shows::now().max(0) as u64)
    }
    pub(super) fn copy_episode_magnet(&mut self) -> Cmd<Msg> {
        let magnet = self
            .selected_episode()
            .map(|(_, e)| e.magnet.clone())
            .filter(|s| s.starts_with("magnet:"));
        self.copy_magnet(magnet)
    }
    pub(super) fn undo_removal(&mut self) -> Cmd<Msg> {
        let Some((hash, until)) = self.removal_undo.clone() else {
            return Cmd::none();
        };
        if shows::now().max(0) as u64 >= until {
            self.removal_undo = None;
            self.set_status(
                "Removal undo expired (10 seconds); your files were kept",
                true,
            );
            return Cmd::none();
        }
        self.service_action(
            "torrents.add",
            serde_json::json!({"source":{"restore":hash}}),
            "removal undone",
        )
    }
    pub(super) fn show_quality_prompt(&mut self) -> Cmd<Msg> {
        if let Some(show) = self.release_show() {
            let (imdb, quality) = (
                show.imdb.clone(),
                show.quality.clone().unwrap_or_else(|| "default".into()),
            );
            self.open_prompt(
                PromptKind::ShowQuality { imdb },
                "Show quality · 2160p / 1080p / 720p / 480p / default",
                false,
            );
            self.prompt.as_mut().unwrap().input.set_value(quality);
        }
        Cmd::none()
    }
    pub(super) fn apply_show_quality(&mut self, imdb: String, value: &str) -> Cmd<Msg> {
        let quality = value.to_ascii_lowercase();
        if !shows::QUALITIES.contains(&quality.as_str()) && quality != "default" {
            self.set_status("Choose 2160p, 1080p, 720p, 480p or default", true);
            return Cmd::none();
        }
        self.service_action(
            "shows.set_quality",
            serde_json::json!({"imdb":imdb,
            "quality":(quality!="default").then_some(quality)}),
            "show quality saved",
        )
    }
    /// A known film must never offer Follow; an episode tag is useful before its card arrives.
    pub(super) fn can_follow_title(&self, name: &str) -> bool {
        match self.imdb_state(name) {
            Some(ImdbState::Ready(title, _)) => title
                .kind
                .get(..2)
                .is_some_and(|p| p.eq_ignore_ascii_case("tv")),
            _ => name.as_bytes().windows(4).any(|w| {
                w[0].eq_ignore_ascii_case(&b's')
                    && w[1].is_ascii_digit()
                    && w[2].is_ascii_digit()
                    && (w[3].eq_ignore_ascii_case(&b'e') || !w[3].is_ascii_alphanumeric())
            }),
        }
    }
}
fn write_clipboard(value: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let write = child.stdin.take().unwrap().write_all(value.as_bytes());
    let status = child.wait()?;
    write?;
    anyhow::ensure!(status.success(), "wl-copy exited {status}");
    Ok(())
}
