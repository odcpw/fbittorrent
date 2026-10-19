//! Durable tracker edits publish only after the atomic ledger save succeeds.
use super::*;
impl EngineDispatcher {
    pub(super) async fn edit_tracker(
        &self,
        cx: &Cx,
        hash: &str,
        value: &str,
        add: bool,
    ) -> Result<()> {
        let url = url::Url::parse(value).map_err(|_| anyhow!("invalid tracker URL"))?;
        anyhow::ensure!(
            value.len() <= 4096
                && url.host_str().is_some()
                && matches!(url.scheme(), "http" | "https" | "udp")
                && (url.scheme() != "udp" || url.port().is_some()),
            "invalid tracker URL"
        );
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let index = ledger
            .records
            .iter()
            .position(|r| r.hash == hash)
            .context("torrent record not found")?;
        let handle = self.inner.handle(hash).map_err(|_| InvalidTorrentState {
            hash: hash.into(),
            reason: "tracker editing requires an admitted torrent",
        })?;
        let urls = handle.tracker_urls();
        if urls.contains(&url) == add {
            return Ok(());
        }
        anyhow::ensure!(!add || urls.len() < 256, "tracker set limit reached");
        anyhow::ensure!(
            add || ledger.records[index].removed_trackers.len() < 256,
            "removed tracker set limit reached"
        );
        let before = ledger.records[index].clone();
        let record = &mut ledger.records[index];
        let value = url.to_string();
        if add {
            record.removed_trackers.retain(|v| v != &value);
            if !record.trackers.contains(&value) {
                record.trackers.push(value);
            }
        } else {
            record.trackers.retain(|v| v != &value);
            if !record.removed_trackers.contains(&value) {
                record.removed_trackers.push(value);
            }
        }
        if let Err(error) = self.inner.save(cx, &ledger).await {
            ledger.records[index] = before;
            return Err(error);
        }
        if add {
            handle.add_tracker(url)?;
        } else {
            handle.remove_tracker(&url);
        }
        Ok(())
    }
}
