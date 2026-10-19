//! The turtle indicator follows confirmed summary state; requests never edit it.
use super::*;
impl TuiApp {
    pub(super) fn turtle_on(&self) -> Option<bool> {
        self.store.summary.as_ref().map(|s| s.alt)
    }
    pub(super) fn toggle_alt(&mut self) -> Cmd<Msg> {
        let Some(on) = self.turtle_on() else {
            return Cmd::none();
        };
        let connection = self.presence.clone();
        self.alt_busy = true;
        Cmd::task(move || {
            Msg::AltDone(
                (|| -> Result<String> {
                    connection
                        .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?
                        .request("transfer.set_alt", serde_json::json!({"on":!on}))?;
                    Ok(format!("turtle limits {}", if on { "off" } else { "on" }))
                })()
                .map_err(|error| error.to_string()),
            )
        })
    }
}
