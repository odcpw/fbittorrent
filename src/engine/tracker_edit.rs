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
        let mut url = url::Url::parse(value).map_err(|_| anyhow!("invalid tracker URL"))?;
        let alias=crate::api::trackers::is_alias(&url);
        anyhow::ensure!(
            value.len() <= (if !add&&alias {8192}else{4096})
                && !(add&&alias)
                && url.host_str().is_some()
                && ((!add&&alias)||matches!(url.scheme(), "http" | "https" | "udp"))
                && (url.scheme() != "udp" || url.port().is_some()),
            "invalid tracker URL"
        );
        let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let index = ledger
            .records
            .iter()
            .position(|r| r.hash == hash)
            .context("torrent record not found")?;
        // Configuration edits do not need payload admission. A completed
        // dormant record keeps its zero-handle/zero-check restore guarantee.
        let handle = match self.inner.handle(hash) {
            Ok(handle)=>Some(handle),
            Err(_) if self.inner.dormant(hash)=>None,
            Err(_)=>return Err(InvalidTorrentState{hash:hash.into(),reason:"tracker editing requires an admitted torrent"}.into()),
        };
        let urls = match &handle {
            Some(handle)=>handle.tracker_urls(),
            None=>trackers::saved(&ledger.records[index],&self.inner.directory)?.into_iter().map(|row|row.url).collect(),
        };
        if alias {
            // Resolve only canonical aliases; never fall back to a literal
            // imported URL with the same fragment or announce a display URL.
            let mut selected=None;
            for raw in &urls {
                if crate::api::trackers::alias(raw)?==value {selected=Some(raw.clone());break;}
            }
            let Some(raw)=selected else {return Ok(());};
            url=raw;
        }
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
        if let Some(handle)=handle {
            if add {handle.add_tracker(url)?;}else{handle.remove_tracker(&url);}
        }
        Ok(())
    }
}
