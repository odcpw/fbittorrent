//! Ordered mutable tracker configuration. A watch cell coalesces edits without a queue.
use parking_lot::Mutex;
use rqbit_runtime::sync::watch;
use std::sync::Arc;
use url::Url;

pub struct TrackerSet {
    urls: Mutex<Vec<Url>>,
    changed: watch::Sender<u64>,
    stats: Arc<crate::TrackerStats>,
}
impl TrackerSet {
    pub fn new(urls: Vec<Url>) -> Arc<Self> {
        Self::with_stats(urls, Arc::new(crate::TrackerStats::default()))
    }
    pub fn with_stats(urls: Vec<Url>, stats: Arc<crate::TrackerStats>) -> Arc<Self> {
        let mut unique = Vec::new();
        for url in urls {
            if !unique.contains(&url) {
                unique.push(url);
            }
        }
        stats.retain(&unique);
        stats.register(unique.iter().cloned());
        let (changed, _) = watch::channel(0);
        Arc::new(Self {
            urls: Mutex::new(unique),
            changed,
            stats,
        })
    }
    pub fn snapshot(&self) -> Vec<Url> {
        self.urls.lock().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub fn add(&self, url: Url) -> anyhow::Result<bool> {
        validate_tracker(&url)?;
        let mut urls = self.urls.lock();
        if urls.contains(&url) {
            return Ok(false);
        }
        anyhow::ensure!(urls.len() < 256, "tracker set limit reached");
        self.stats.register([url.clone()]);
        urls.push(url);
        self.changed
            .send_modify(|v| *v = v.wrapping_add(1))
            .expect("owned watch sender");
        Ok(true)
    }
    pub fn remove(&self, url: &Url) -> bool {
        let mut urls = self.urls.lock();
        let before = urls.len();
        urls.retain(|v| v != url);
        if urls.len() == before {
            return false;
        }
        self.stats.remove(url);
        self.changed
            .send_modify(|v| *v = v.wrapping_add(1))
            .expect("owned watch sender");
        true
    }
}
pub fn validate_tracker(url: &Url) -> anyhow::Result<()> {
    anyhow::ensure!(
        url.as_str().len() <= 4096
            && url.host_str().is_some()
            && matches!(url.scheme(), "http" | "https" | "udp")
            && (url.scheme() != "udp" || url.port().is_some()),
        "invalid tracker URL"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{redact_url, sanitize};
    #[test]
    fn tracker_set_mutation() {
        let a = Url::parse("http://127.0.0.1:1234/a?passkey=secret").unwrap();
        let b = Url::parse("udp://127.0.0.1:1235/announce").unwrap();
        let set = TrackerSet::new(vec![a.clone(), a.clone()]);
        let rx = set.subscribe();
        assert!(!set.add(a.clone()).unwrap());
        assert!(!rx.has_changed());
        assert!(set.add(b.clone()).unwrap());
        assert!(rx.has_changed());
        assert_eq!(set.snapshot(), vec![a.clone(), b.clone()]);
        assert!(set.remove(&a));
        assert!(!set.remove(&a));
        assert_eq!(set.snapshot(), vec![b]);
        assert!(set.add(Url::parse("file:///tmp/tracker").unwrap()).is_err());
        assert_eq!(redact_url(&a), "http://127.0.0.1:1234/a?…");
        assert!(
            !sanitize(
                &a,
                "tracker secret http://other/tracker?passkey=secret\nfailed"
            )
            .contains("secret")
        );
        println!(
            "tracker_set_mutation: ordered dedup, idempotent add/remove, wake and scheme validation passed (URL queries redacted)"
        );
    }
}
