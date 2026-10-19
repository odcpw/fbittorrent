//! One priority slot above the legend; the selection strip has its own row.
use super::*;
use std::borrow::Cow;
impl TuiApp {
    pub(in crate::tui) fn torrent_reason(&self, torrent: &Torrent) -> String {
        let Some(row) = self.store.torrents.get(&torrent.hash) else {
            return format::state_word(torrent, None).to_lowercase();
        };
        let Some(code) = row.reason.as_deref() else {
            return format::state_word(torrent, Some(row)).to_lowercase();
        };
        let interface = self
            .store
            .network
            .as_ref()
            .and_then(|n| n.interface.as_deref())
            .unwrap_or("the VPN");
        format::reason(code)
            .replace("<interface>", interface)
            .replace("<error>", row.error.as_deref().unwrap_or("unknown error"))
    }
    pub(in crate::tui) fn view_selected_strip(&self, area: Rect, frame: &mut Frame) {
        let Some(t) = self.selected_torrent() else {
            return;
        };
        let row = self.store.torrents.get(&t.hash);
        let queue = row
            .and_then(|r| r.queue_pos)
            .map(|p| format!(" · queue {p}"))
            .unwrap_or_default();
        let metrics = format::Metrics {
            legacy: t,
            api: row,
        };
        let text = format!(
            "{} · {}{queue} · {} · {}",
            format::state_cell(t, row).trim_end(),
            self.torrent_reason(t),
            metrics.upload(),
            t.save_path
        );
        Paragraph::new(short(&text, area.width as usize))
            .style(self.theme.dimmed())
            .wrap(WrapMode::None)
            .render(area, frame);
    }
    pub(in crate::tui) fn priority_banner(&self) -> Option<Cow<'_, str>> {
        if let Some(summary) = &self.store.summary {
            if summary.disk.low {
                let needed = human_bytes(summary.disk.needed_bytes.min(i64::MAX as u64) as i64);
                let free = summary
                    .disk
                    .free_bytes
                    .map(|v| format!("{} free",human_bytes(v.min(i64::MAX as u64) as i64)))
                    .unwrap_or_else(|| "free space unknown".into());
                return Some(
                    format!(
                        "Not enough space for the running downloads: {needed} needed, {free}"
                    )
                    .into(),
                );
            }
        }
        if let Some(t) = self
            .store
            .torrents
            .values()
            .find(|t| t.reason.as_deref() == Some("disk_full"))
        {
            return Some(
                t.error
                    .as_deref()
                    .unwrap_or("Disk full; free space to resume downloads")
                    .into(),
            );
        }
        if let Some(network) = self.store.network.as_ref().filter(|n| !n.ready) {
            return Some(network.message.as_str().into());
        }
        if let Some(notice) = self.connection_banner() {
            return Some(notice);
        }
        if self.daemon_stopped {
            return Some("fbittorrent stopped · Enter starts it again".into());
        }
        if self
            .store
            .summary
            .as_ref()
            .is_some_and(|s| s.daemon.upgrade_pending)
        {
            return Some("Update installed; restarts when idle".into());
        }
        None
    }
}
