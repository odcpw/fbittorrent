//! Confirmed daemon state replaces rates only when they are zero or parked.
use super::*;
impl TuiApp {
    pub(in crate::tui) fn header_status(&self, width: u16) -> String {
        if !self.online {
            return "● offline".into();
        }
        let Some(s) = &self.store.summary else {
            return "Connecting…".into();
        };
        let parked = !self.store.network.as_ref().map_or(
            s.network.state != crate::api::NetworkReadiness::Waiting,
            |n| n.ready,
        );
        let status = if parked {
            if width < 100 {
                "Waiting for VPN".into()
            } else {
                s.network.message.clone()
            }
        } else if s.dl_bps == Some(0) && s.ul_bps == Some(0) {
            if !s.daemon.busy_reasons.is_empty() {
                if s.daemon.busy_reasons.iter().any(|r| r == "interactive")
                    && s.daemon.busy_reasons.len() == 1
                {
                    "Idle · UI open".into()
                } else {
                    format!("Busy · {}", s.daemon.busy_reasons.join(", "))
                }
            } else if let Some(seconds) = s.daemon.idle_exit_in_s {
                if seconds >= 60 {
                    format!("Idle · stops in {} min", seconds.div_ceil(60))
                } else {
                    format!("Idle · stops in {seconds}s")
                }
            } else if self
                .store
                .settings
                .as_ref()
                .is_some_and(|s| s.idle_exit_secs == 0)
            {
                "Idle · exit disabled".into()
            } else {
                "Idle".into()
            }
        } else {
            format!("↓ {}  ↑ {}", format::rate(s.dl_bps), format::rate(s.ul_bps))
        };
        if s.alt {
            format!("󰢃 Turtle {status}")
        } else {
            status
        }
    }
}
