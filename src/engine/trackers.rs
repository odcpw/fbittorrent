//! Observations can be read without restoring a dormant or parked torrent.
use super::*;
use ops::EngineResult;

pub(super) fn saved(record: &Record, directory: &Path) -> Result<Vec<librqbit::TrackerStat>> {
    // A corrupt cache must not hide the saved row's restore diagnostic. Its
    // magnet still supplies known tracker URLs; this read never repairs or admits it.
    let mut urls = if let Ok(Some(bytes)) = parked::parked_metadata(record, directory) {
        librqbit::torrent_from_bytes(&bytes)?
            .iter_announce()
            .filter_map(|v| std::str::from_utf8(v.as_ref()).ok()?.parse().ok())
            .collect::<Vec<url::Url>>()
    } else {
        librqbit::Magnet::parse(&record.source)
            .map(|m| {
                m.trackers
                    .into_iter()
                    .filter_map(|s| s.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    urls.extend(record.trackers.iter().filter_map(|v|v.parse().ok()));
    urls.retain(|url|!record.removed_trackers.iter().any(|v|v==url.as_str()));
    urls.sort();
    urls.dedup();
    Ok(urls
        .into_iter()
        .map(librqbit::TrackerStat::not_contacted)
        .collect())
}
impl EngineDispatcher {
    pub(super) async fn trackers(&self, _cx: &Cx, hash: &str) -> Result<EngineResult> {
        let ledger = self.observation_ledger().await?;
        let record = ledger
            .records
            .iter()
            .find(|r| r.hash == hash)
            .context("torrent record not found")?;
        let rows = if let Ok(handle) = self.inner.handle(hash) {
            handle.tracker_stats()
        } else {
            saved(record, &self.inner.directory)?
        };
        Ok(EngineResult::Trackers(rows))
    }
    pub(super) async fn reannounce(&self, cx: &Cx, hashes: &[String]) -> Result<EngineResult> {
        anyhow::ensure!(
            (1..=256).contains(&hashes.len()),
            "hashes must contain 1..256 entries"
        );
        let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
        let mut handles = Vec::with_capacity(hashes.len());
        for hash in hashes {
            anyhow::ensure!(
                ledger.records.iter().any(|r| &r.hash == hash),
                "torrent not found: {hash}"
            );
            let handle = self
                .inner
                .handle(hash)
                .ok()
                .filter(|h| h.live().is_some())
                .ok_or_else(|| InvalidTorrentState {
                    hash: hash.clone(),
                    reason: "not_live",
                })?;
            handles.push(handle);
        }
        for handle in handles {
            handle.announce_now();
        }
        Ok(EngineResult::Acknowledged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_saved_metadata_preserves_tracker_diagnostics() {
        let directory = crate::engine::tests::review_directory("corrupt-trackers");
        std::fs::create_dir_all(&directory).unwrap();
        let mut record: Record = serde_json::from_value(serde_json::json!({
            "hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"corrupt-cache",
            "source":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&tr=http%3A%2F%2F127.0.0.1%3A1%2Fannounce",
            "output_folder":directory,"only_files":null,"upload_limit":32768,"ratio_limit":0.0,
            "category":"","tags":"","added_on":1,"completion_on":0,"paused":false
        })).unwrap();
        let cache = directory.join(format!("{}.torrent", record.hash));
        std::fs::write(&cache, b"corrupt cache").unwrap();
        assert!(parked::parked_metadata(&record, &directory).is_err());
        let rows = saved(&record, &directory).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].url.as_str(), "http://127.0.0.1:1/announce");
        assert_eq!(rows[0].status, librqbit::TrackerStatus::NotContacted);
        assert!(rows[0].last_announce_at.is_none() && rows[0].peers.is_none());
        record.source.clear();
        assert!(saved(&record, &directory).unwrap().is_empty());
        assert_eq!(std::fs::read(&cache).unwrap(), b"corrupt cache");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saved_urls_do_not_restore_metadata_or_contact_trackers() {
        let directory = crate::engine::tests::review_directory("saved-trackers");
        std::fs::create_dir_all(&directory).unwrap();
        let record:Record=serde_json::from_value(serde_json::json!({
            "hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"metadata",
            "source":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&tr=http%3A%2F%2F127.0.0.1%3A1%2Fa&tr=udp%3A%2F%2F127.0.0.1%3A2%2Fa&tr=http%3A%2F%2F127.0.0.1%3A1%2Fa",
            "output_folder":directory,"only_files":null,"upload_limit":32768,"ratio_limit":0.0,
            "category":"","tags":"","added_on":1,"completion_on":0,"paused":true
        })).unwrap();
        let rows = saved(&record, &directory).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].url < rows[1].url);
        assert!(
            rows.iter()
                .all(|r| r.status == librqbit::TrackerStatus::NotContacted
                    && r.last_announce_at.is_none()
                    && r.peers.is_none())
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        std::fs::remove_dir(directory).unwrap();
    }
}

#[cfg(test)]
#[test]
fn saved_tracker_edits_merge_without_restoring_or_contacting() {
    let directory=crate::engine::tests::review_directory("saved-tracker-edits");
    std::fs::create_dir_all(&directory).unwrap();
    let record:Record=serde_json::from_value(serde_json::json!({
        "hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"metadata",
        "source":"magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&tr=http%3A%2F%2F127.0.0.1%3A1%2Foriginal",
        "output_folder":directory,"only_files":null,"upload_limit":32768,"ratio_limit":0.0,
        "category":"","tags":"","added_on":1,"completion_on":0,"paused":true,
        "trackers":["http://127.0.0.1:2/added"],"removed_trackers":["http://127.0.0.1:1/original"]
    })).unwrap();
    let rows=saved(&record,&directory).unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].url.as_str(),"http://127.0.0.1:2/added");
    assert_eq!(rows[0].status,librqbit::TrackerStatus::NotContacted);
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(),0);
    std::fs::remove_dir(directory).unwrap();
}
