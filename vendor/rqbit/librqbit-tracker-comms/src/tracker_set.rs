//! Ordered mutable tracker configuration. A watch cell coalesces edits without a queue.
use parking_lot::Mutex;
use rqbit_runtime::sync::watch;
use std::sync::Arc;
use url::Url;

pub struct TrackerSet {
    urls: Mutex<Vec<Url>>,
    changed: watch::Sender<u64>,
}
impl TrackerSet {
    pub fn new(urls: Vec<Url>) -> Arc<Self> {
        let mut unique = Vec::new();
        for url in urls {
            if !unique.contains(&url) {
                unique.push(url);
            }
        }
        let (changed, _) = watch::channel(0);
        Arc::new(Self {
            urls: Mutex::new(unique),
            changed,
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

// Queries can contain passkeys; diagnostics never include their values.
pub fn redact_url(url: &Url) -> String {
    match url.as_str().find('?') {
        Some(end) => format!("{}?…", &url.as_str()[..end]),
        None => url.to_string(),
    }
}
pub fn sanitize(url: &Url, text: &str) -> String {
    let mut text = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    for (_, value) in url.query_pairs() {
        if !value.is_empty() {
            text = text.replace(value.as_ref(), "…");
        }
    }
    text.split_whitespace()
        .map(|word| {
            if let Some(scheme) = word.find("://") {
                if let Some(query) = word[scheme + 3..].find('?') {
                    return format!("{}?…", &word[..scheme + 3 + query]);
                }
            }
            word.to_owned()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}
