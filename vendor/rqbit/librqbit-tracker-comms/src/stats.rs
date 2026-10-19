//! Tracker observations and forced-announcement wakeups outlive a run region.
use parking_lot::Mutex;
use rqbit_runtime::{Cx, sync::Notify};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use url::Url;
use crate::{redact_url,sanitize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerStatus {
    Ok,
    Error,
    Updating,
    NotContacted,
}
#[derive(Clone)]
pub struct TrackerStat {
    pub url: Url,
    pub status: TrackerStatus,
    pub last_announce_at: Option<SystemTime>,
    pub next_announce_at: Option<SystemTime>,
    pub peers: Option<u32>,
    pub message: Option<String>,
    revision: u64,
}
impl std::fmt::Debug for TrackerStat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackerStat")
            .field("url", &redact_url(&self.url))
            .field("status", &self.status)
            .field("peers", &self.peers)
            .field("message", &self.message)
            .finish()
    }
}
impl TrackerStat {
    pub fn not_contacted(url: Url) -> Self {
        Self {
            url,
            status: TrackerStatus::NotContacted,
            last_announce_at: None,
            next_announce_at: None,
            peers: None,
            message: None,
            revision: 0,
        }
    }
}
#[derive(Default)]
pub struct TrackerStats {
    rows: Mutex<BTreeMap<Url, TrackerStat>>,
    wake: Notify,
    requested: AtomicU64,
    revision: AtomicU64,
    last_forced: Mutex<Option<Instant>>,
}
impl TrackerStats {
    pub fn register(&self, urls: impl IntoIterator<Item = Url>) {
        let mut rows = self.rows.lock();
        for url in urls {
            rows.entry(url.clone())
                .or_insert_with(|| TrackerStat::not_contacted(url));
        }
    }
    /// Configuration changes retire observations immediately. An old request's
    /// revision guard cannot restore a removed row or overwrite a re-added row.
    pub fn remove(&self, url: &Url) {
        self.rows.lock().remove(url);
    }
    pub fn retain(&self, urls: &[Url]) {
        self.rows.lock().retain(|url, _| urls.contains(url));
    }
    pub fn snapshot(&self) -> Vec<TrackerStat> {
        self.rows.lock().values().cloned().collect()
    }
    pub fn not_contacted(&self, url: &Url, message: &str) {
        let mut rows = self.rows.lock();
        if let Some(row) = rows.get_mut(url) {
            row.status = TrackerStatus::NotContacted;
            row.message = Some(sanitize(url, message));
        }
    }
    pub fn announce_now(&self) -> bool {
        let mut last = self.last_forced.lock();
        if last.is_some_and(|value| value.elapsed() < Duration::from_secs(60)) {
            return false;
        }
        *last = Some(Instant::now());
        drop(last);
        self.requested.fetch_add(1, Ordering::AcqRel);
        self.wake.notify_waiters();
        true
    }
    pub fn generation(&self) -> u64 {
        self.requested.load(Ordering::Acquire)
    }
    pub async fn wait(&self, cx: &Cx, seen: u64, delay: Duration) -> bool {
        rqbit_runtime::select! {
            _=self.wake.wait_until(||self.generation()!=seen)=>true,
            _=rqbit_runtime::time::sleep(cx,delay)=>false,
        }
    }
    pub fn updating(self: &Arc<Self>, url: &Url) -> AnnounceGuard {
        let mut rows = self.rows.lock();
        let previous = rows
            .get(url)
            .cloned()
            .unwrap_or_else(|| TrackerStat::not_contacted(url.clone()));
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(row) = rows.get_mut(url) {
            row.revision = revision;
            row.status = TrackerStatus::Updating;
            row.message = None;
            row.next_announce_at = None;
        }
        AnnounceGuard {
            owner: self.clone(),
            url: url.clone(),
            previous,
            revision,
            done: false,
        }
    }
}
pub struct AnnounceGuard {
    owner: Arc<TrackerStats>,
    url: Url,
    previous: TrackerStat,
    revision: u64,
    done: bool,
}
impl AnnounceGuard {
    fn finish(
        &mut self,
        status: TrackerStatus,
        peers: Option<u32>,
        message: Option<String>,
        interval: Duration,
    ) {
        if let Some(row) = self
            .owner
            .rows
            .lock()
            .get_mut(&self.url)
            .filter(|row| row.revision == self.revision)
        {
            row.status = status;
            row.peers = peers;
            row.message = message;
            let now = SystemTime::now();
            row.last_announce_at = Some(now);
            row.next_announce_at = now.checked_add(interval);
        }
        self.done = true;
    }
    pub fn ok(mut self, peers: u32, interval: Duration) {
        self.finish(TrackerStatus::Ok, Some(peers), None, interval);
    }
    pub fn error(mut self, message: &str, interval: Duration) {
        let message = sanitize(&self.url, message);
        self.finish(TrackerStatus::Error, None, Some(message), interval);
    }
}
impl Drop for AnnounceGuard {
    fn drop(&mut self) {
        if !self.done {
            if let Some(row) = self
                .owner
                .rows
                .lock()
                .get_mut(&self.url)
                .filter(|row| row.revision == self.revision)
            {
                *row = self.previous.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_keeps_last_result_on_cancel_and_force_is_rate_limited() {
        let state = Arc::new(TrackerStats::default());
        let url = Url::parse("http://localhost/announce?passkey=secret-value").unwrap();
        state.register([url.clone()]);
        state
            .updating(&url)
            .error("fixture: not yet", Duration::from_secs(10));
        let previous = state.snapshot();
        {
            let _pending = state.updating(&url);
            assert_eq!(state.snapshot()[0].status, TrackerStatus::Updating);
        }
        assert_eq!(state.snapshot()[0].status, previous[0].status);
        assert_eq!(state.snapshot()[0].message, previous[0].message);
        assert!(state.announce_now());
        assert!(!state.announce_now());
        state.updating(&url).ok(1, Duration::from_secs(5));
        assert_eq!(state.snapshot()[0].peers, Some(1));
        assert!(!format!("{:?}", state.snapshot()).contains("secret-value"));
    }
    #[test]
    fn messages_redact_all_urls_and_echoed_values() {
        let url = Url::parse("https://tracker.invalid/announce?passkey=secret-value").unwrap();
        let message = sanitize(
            &url,
            "https://elsewhere.invalid/x?another=token failed; echoed secret-value\n",
        );
        assert!(
            !message.contains("token")
                && !message.contains("secret-value")
                && !message.contains('\n')
        );
        assert!(message.len() <= 200);
        assert_eq!(redact_url(&url), "https://tracker.invalid/announce?…");
    }
}

#[cfg(test)]
#[test]
fn removed_announce_cannot_resurrect_or_overwrite_readded_observation() {
    let url=Url::parse("http://127.0.0.1:1234/announce").unwrap();
    let state=Arc::new(TrackerStats::default());
    let set=crate::TrackerSet::with_stats(vec![url.clone()],state.clone());
    let old=state.updating(&url);
    assert!(set.remove(&url));assert!(state.snapshot().is_empty());
    old.ok(99,Duration::from_secs(600));assert!(state.snapshot().is_empty());
    let late=state.updating(&url);assert!(state.snapshot().is_empty());
    assert!(set.add(url.clone()).unwrap());
    late.error("old request",Duration::from_secs(10));
    let rows=state.snapshot();assert_eq!(rows.len(),1);
    assert_eq!(rows[0].status,TrackerStatus::NotContacted);assert!(rows[0].message.is_none());
    assert!(state.announce_now());assert!(!state.announce_now());
    assert!(set.remove(&url));assert!(set.add(url).unwrap());
    assert!(!state.announce_now(),"set mutation preserves the same throttle owner");
    println!("removed/readded tracker: stale results cannot resurrect rows; throttle owner survives edits");
}

#[cfg(test)]
#[test]
fn userinfo_and_fragments_never_reach_tracker_diagnostics() {
    let url=Url::parse("https://credential-user:credential-pass@tracker.invalid/announce?passkey=private-key#private-fragment").unwrap();
    assert_eq!(redact_url(&url),"https://tracker.invalid/announce?…");
    let value=sanitize(&url,"credential-user credential-pass private-key private-fragment https://other-user:other-pass@other.invalid/error?secret=value#fragment");
    for secret in ["credential-user","credential-pass","private-key","private-fragment","other-user","other-pass","secret=","#fragment"] {assert!(!value.contains(secret),"{value}");}
    assert!(value.contains("https://other.invalid/error?…"));
    let scheme_secret=Url::parse("https://tracker.invalid/?key=https").unwrap();
    let value=sanitize(&scheme_secret,"https://other-user:other-pass@other.invalid/error?secret=value");
    for secret in ["other-user","other-pass","secret=","value"] {assert!(!value.contains(secret),"{value}");}
}
