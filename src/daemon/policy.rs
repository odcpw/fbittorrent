//! Owner idle net. All inputs update before the countdown is observed; no client
//! owns daemon lifetime. A glance freezes remaining time rather than resetting it.
use serde::Serialize;
use std::{collections::BTreeMap, sync::{Arc, Mutex}, time::{Duration, Instant}};

#[derive(Clone, Copy, Default)]
pub(crate) struct Transfers {
    pub downloading: bool,
    pub seeding: bool,
    pub parked_waiting: bool,
    pub restoring: bool,
    pub starting: bool,
}
impl Transfers {
    pub fn from_torrents(rows: &[crate::api::Torrent], state: &str, restoring: bool) -> Self {
        use crate::api::{Intent, TorrentState};
        let mut activity = Self { starting: matches!(state, "starting" | "stopping"),
            restoring, ..Self::default() };
        for row in rows {
            if row.intent != Intent::Run || row.staged { continue; }
            activity.downloading |= matches!(row.state, TorrentState::Downloading
                | TorrentState::Stalled | TorrentState::Metadata | TorrentState::Checking
                | TorrentState::Queued);
            activity.seeding |= row.seed && row.state == TorrentState::Seeding;
            activity.parked_waiting |= state == "parked"
                && row.state == TorrentState::WaitingVpn && row.completed_at.is_none();
        }
        activity
    }
}
#[derive(Clone, Copy)]
struct Presence { interactive: bool, glance: bool }
#[derive(Debug, Serialize)]
pub(crate) struct Snapshot {
    pub busy_reasons: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_exit_in_s: Option<u64>,
}

pub(crate) struct Policy {
    seconds: u64,
    override_seconds: Option<u64>,
    remaining: Duration,
    last: Instant,
    transfers: Transfers,
    clients: BTreeMap<u64, Presence>,
    searches: usize,
    mutations: usize,
}
impl Policy {
    pub fn new(configured: u64, override_seconds: Option<u64>, now: Instant) -> Self {
        let seconds = override_seconds.unwrap_or(configured);
        Self { seconds, override_seconds, remaining: Duration::from_secs(seconds), last: now,
            transfers: Transfers { starting: true, ..Transfers::default() },
            clients: BTreeMap::new(), searches: 0, mutations: 0 }
    }
    pub fn seconds(&self) -> u64 { self.seconds }
    pub fn replace_delay(&mut self, seconds: u64, now: Instant) {
        self.override_seconds = None;
        self.configure(seconds, now);
    }
    fn busy(&self) -> bool {
        let t = self.transfers;
        t.downloading || t.seeding || t.parked_waiting || t.restoring || t.starting
            || self.mutations != 0 || self.searches != 0
            || self.clients.values().any(|p| p.interactive)
    }
    fn glance(&self) -> bool { self.clients.values().any(|p| p.glance) }
    fn advance(&mut self, now: Instant) {
        if self.busy() {
            self.remaining = Duration::from_secs(self.seconds);
        } else if !self.glance() && self.seconds != 0 {
            self.remaining = self.remaining.saturating_sub(now.saturating_duration_since(self.last));
        }
        self.last = now;
    }
    fn changed(&mut self) {
        if self.busy() { self.remaining = Duration::from_secs(self.seconds); }
    }
    pub fn configure(&mut self, configured: u64, now: Instant) {
        self.advance(now);
        let seconds = self.override_seconds.unwrap_or(configured);
        if self.seconds != seconds {
            self.seconds = seconds;
            self.remaining = Duration::from_secs(seconds);
        }
    }
    pub fn transfers(&mut self, value: Transfers, now: Instant) {
        self.advance(now); self.transfers = value; self.changed();
    }
    pub fn client(&mut self, id: u64, interactive: bool, glance: bool, now: Instant) {
        self.advance(now);
        self.clients.insert(id, Presence { interactive, glance });
        self.changed();
    }
    pub fn disconnect(&mut self, id: u64, now: Instant) {
        self.advance(now); self.clients.remove(&id); self.changed();
    }
    /// Search-session ownership (including its disconnect grace) belongs to the
    /// session manager. The idle net consumes its current number of live sessions.
    pub fn searches(&mut self, count: usize, now: Instant) {
        self.advance(now); self.searches = count; self.changed();
    }
    pub fn snapshot(&mut self, now: Instant) -> Snapshot {
        self.advance(now);
        let t = self.transfers;
        let mut reasons = Vec::with_capacity(6);
        for (active, reason) in [(t.downloading, "downloading"), (t.seeding, "seeding"),
            (t.parked_waiting, "parked_waiting"),
            (self.clients.values().any(|p| p.interactive), "interactive"),
            (self.searches != 0, "search"),
            (self.mutations != 0 || t.restoring || t.starting, "mutation")] {
            if active { reasons.push(reason); }
        }
        Snapshot { idle_exit_in_s: (self.seconds != 0 && reasons.is_empty()).then(||
            self.remaining.as_secs() + u64::from(self.remaining.subsec_nanos() != 0)), busy_reasons: reasons }
    }
    pub fn expired(&mut self, now: Instant) -> bool {
        self.advance(now);
        self.seconds != 0 && !self.busy() && !self.glance() && self.remaining.is_zero()
    }
    pub fn wait(&mut self, now: Instant) -> Duration {
        self.advance(now);
        if self.seconds == 0 || self.busy() || self.glance() {
            Duration::from_secs(5)
        } else { self.remaining.min(Duration::from_secs(5)) }
    }
}

/// A queued or executing mutation keeps the idle net cancelled, including after
/// its response times out or its originating connection has disappeared.
pub(crate) struct Mutation(Arc<Mutex<Policy>>);
impl Mutation {
    pub fn start(policy: Arc<Mutex<Policy>>) -> Self {
        {
            let mut p = policy.lock().unwrap_or_else(|e| e.into_inner());
            p.advance(Instant::now()); p.mutations += 1; p.changed();
        }
        Self(policy)
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        let mut p = self.0.lock().unwrap_or_else(|e| e.into_inner());
        p.advance(Instant::now()); p.mutations -= 1; p.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn idle(seconds: u64, at: Instant) -> Policy {
        let mut p=Policy::new(seconds,None,at); p.transfers(Transfers::default(),at); p
    }
    #[test]
    fn countdown_cancel() {
        let at=Instant::now(); let mut p=idle(30,at);
        assert_eq!(p.snapshot(at+Duration::from_secs(10)).idle_exit_in_s,Some(20));
        p.transfers(Transfers{downloading:true,..Transfers::default()},at+Duration::from_secs(10));
        assert_eq!(p.snapshot(at+Duration::from_secs(100)).busy_reasons,vec!["downloading"]);
        assert!(!p.expired(at+Duration::from_secs(100)));
        p.transfers(Transfers::default(),at+Duration::from_secs(100));
        assert_eq!(p.snapshot(at+Duration::from_secs(100)).idle_exit_in_s,Some(30));
        assert!(p.expired(at+Duration::from_secs(130)));
    }
    #[test]
    fn countdown_pause_resume() {
        let at=Instant::now(); let mut p=idle(30,at);
        p.client(1,false,true,at+Duration::from_secs(10));
        assert_eq!(p.snapshot(at+Duration::from_secs(15)).idle_exit_in_s,Some(20));
        p.disconnect(1,at+Duration::from_secs(15));
        assert_eq!(p.snapshot(at+Duration::from_secs(15)).idle_exit_in_s,Some(20));
        assert!(!p.expired(at+Duration::from_secs(34)));
        assert!(p.expired(at+Duration::from_secs(35)));
        println!("glance pauses at 20 seconds; disconnect resumes all 20 seconds");
    }
    #[test]
    fn zero_disables() {
        let at=Instant::now(); let mut p=idle(0,at);
        assert_eq!(p.snapshot(at+Duration::from_secs(100000)).idle_exit_in_s,None);
        assert!(!p.expired(at+Duration::from_secs(100000)));
    }
    #[test]
    fn every_busy_reason_and_override() {
        let at=Instant::now(); let mut p=Policy::new(300,Some(2),at);
        p.configure(600,at); assert_eq!(p.seconds(),2);
        for t in [Transfers{seeding:true,..Transfers::default()},Transfers{parked_waiting:true,..Transfers::default()},Transfers{restoring:true,..Transfers::default()}] {
            p.transfers(t,at); assert!(!p.snapshot(at).busy_reasons.is_empty());
            assert!(!p.expired(at+Duration::from_secs(100)));
        }
        p.transfers(Transfers::default(),at+Duration::from_secs(100));
        p.client(1,true,false,at+Duration::from_secs(100));
        assert_eq!(p.snapshot(at+Duration::from_secs(100)).busy_reasons,vec!["interactive"]);
        p.disconnect(1,at+Duration::from_secs(100));
        p.searches(1,at+Duration::from_secs(100));
        assert_eq!(p.snapshot(at+Duration::from_secs(100)).busy_reasons,vec!["search"]);
    }
    #[test]
    fn transfer_table_excludes_staged_paused_and_empty_parked() {
        use crate::api::{Intent, Torrent, TorrentState};
        let raw = include_str!("../../contracts/ipc/v1/torrents.ndjson").lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find_map(|line| line["frame"]["result"]["torrents"].as_array()?.first().cloned()).unwrap();
        let mut row: Torrent = serde_json::from_value(raw).unwrap();
        row.intent=Intent::Run; row.staged=false;
        for state in [TorrentState::Downloading,TorrentState::Stalled,TorrentState::Metadata,
                      TorrentState::Checking,TorrentState::Queued] {
            row.state=state;
            assert!(Transfers::from_torrents(&[row.clone()],"running",false).downloading);
        }
        row.staged=true;
        assert!(!Transfers::from_torrents(&[row.clone()],"running",false).downloading);
        row.staged=false; row.state=TorrentState::Paused; row.seed=true;
        let inactive=Transfers::from_torrents(&[row.clone()],"running",false);
        assert!(!inactive.downloading && !inactive.seeding);
        row.state=TorrentState::Seeding;
        assert!(Transfers::from_torrents(&[row.clone()],"running",false).seeding);
        row.state=TorrentState::WaitingVpn; row.completed_at=None;
        assert!(Transfers::from_torrents(&[row],"parked",false).parked_waiting);
        assert!(!Transfers::from_torrents(&[],"parked",false).parked_waiting);
    }
    #[test]
    fn queued_mutation_guard_keeps_work_busy_until_dropped() {
        let policy=Arc::new(Mutex::new(idle(2,Instant::now())));
        let mutation=Mutation::start(policy.clone());
        assert_eq!(policy.lock().unwrap().snapshot(Instant::now()).busy_reasons,vec!["mutation"]);
        drop(mutation);
        let snapshot=policy.lock().unwrap().snapshot(Instant::now());
        assert!(snapshot.busy_reasons.is_empty());
        assert_eq!(snapshot.idle_exit_in_s,Some(2));
    }
}

#[cfg(test)]
mod settings_integration_tests {
    use super::*;
    #[test]
    fn explicit_setting_replaces_serve_override_but_other_config_updates_keep_it() {
        let now = Instant::now();
        let mut policy = Policy::new(300, Some(2), now);
        policy.configure(300, now);
        assert_eq!(policy.seconds(), 2);
        policy.replace_delay(300, now);
        assert_eq!(policy.seconds(), 300);
        policy.configure(45, now);
        assert_eq!(policy.seconds(), 45);
    }
}
