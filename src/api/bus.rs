//! Subscription owner. One sampler serves every connection; snapshots and
//! revisions are serialized here, while socket writes use the server's outbox.
mod summary;
pub(crate) use summary::counts as summary_counts;

use super::{
    ops::{self, error, ApiResult},
    server::EventSink,
    *,
};
use crate::{
    daemon::{slot::Slot, Daemon},
    engine::{
        self,
        ops::{EngineOp, EngineResult},
    },
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Default)]
struct Wake {
    changed: Mutex<bool>,
    ready: Condvar,
}
impl Wake {
    fn signal(&self) {
        *self.changed.lock().unwrap() = true;
        self.ready.notify_all();
    }
    fn wait(&self, timeout: Duration) -> bool {
        let g = self.changed.lock().unwrap();
        let (mut g, _) = self.ready.wait_timeout_while(g, timeout, |v| !*v).unwrap();
        let signaled = *g;
        *g = false;
        signaled
    }
}
struct TopicState {
    topic: Topic,
    interval: Duration,
    due: Instant,
    /// Shared with the sample that produced it; never deep-copied per client.
    sent: Option<Arc<Value>>,
}
struct Subscription {
    sub: u64,
    sink: EventSink,
    rev: u64,
    topics: Vec<TopicState>,
    since: Option<u64>,
    epoch: Option<(u64, String)>,
    active: bool,
}
#[derive(Default)]
struct Registry {
    next: BTreeMap<u64, u64>,
    subscriptions: BTreeMap<(u64, u64), Subscription>,
    events: EventRing,
}
impl Registry {
    fn engine_changed(&mut self,epoch:u64,state:&str) {
        let event=self.events.append(json!({"ev":"engine.changed","epoch":epoch,"state":state,"reason":"engine_restart"}));
        let mut connections=BTreeSet::new();
        for sub in self.subscriptions.values().filter(|sub|sub.active) {
            if connections.insert(sub.sink.id()) {let _=sub.sink.send(serde_json::from_value(event.clone()).unwrap());}
        }
    }
}
#[derive(Default)]
struct EventRing {
    next: u64,
    entries: VecDeque<(Instant, Value)>,
}
impl EventRing {
    fn prune(&mut self) {
        while self.entries.front().is_some_and(|(at, _)| {
            self.entries.len() > 1000 || at.elapsed() >= Duration::from_secs(600)
        }) {
            self.entries.pop_front();
        }
    }
    fn append(&mut self, mut value: Value) -> Value {
        self.next += 1;
        value["seq"] = json!(self.next);
        value["ts"] = json!(now());
        self.entries.push_back((Instant::now(), value.clone()));
        self.prune();
        value
    }
    fn oldest(&mut self) -> u64 {
        self.prune();
        self.entries
            .front()
            .map(|(_, v)| v["seq"].as_u64().unwrap())
            .unwrap_or(self.next + 1)
    }
    fn replay(&mut self, since: Option<u64>) -> (bool, u64, Vec<Value>) {
        let oldest = self.oldest();
        let resync = since.is_some_and(|s| s.saturating_add(1) < oldest);
        (
            resync,
            oldest,
            self.entries
                .iter()
                .filter(|(_, v)| resync || since.is_none_or(|s| v["seq"].as_u64().unwrap() > s))
                .map(|(_, v)| v.clone())
                .collect(),
        )
    }
}
pub(crate) struct Bus {
    daemon: Arc<Daemon>,
    registry: Mutex<Registry>,
    peers: peers::Sampler,
    wake: Arc<Wake>,
    stopping: AtomicBool,
    initial_hashes: BTreeSet<String>,
    /// A log line wakes the sampler only while a Log topic is subscribed.
    log_wanted: Arc<AtomicBool>,
}
impl Bus {
    pub(crate) fn refresh(&self) { self.wake.signal(); }
    pub(crate) fn new_episodes(&self,count:u64,names:&[String]) {self.daemon.notifier.new_episodes(count,names);}
    pub(crate) fn engine_changed(&self,epoch:u64,state:&str) {self.registry.lock().unwrap().engine_changed(epoch,state);self.wake.signal();}
    pub fn new(daemon: Arc<Daemon>) -> Arc<Self> {
        Arc::new(Self {
            daemon,
            registry: Mutex::default(),
            peers: Default::default(),
            wake: Arc::default(),
            stopping: AtomicBool::new(false),
            log_wanted: Arc::default(),
            initial_hashes: std::fs::read(crate::config::data_dir().join("engine/torrents.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| value["records"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|row| row["hash"].as_str().map(str::to_owned))
                .collect(),
        })
    }
    pub fn start(self: &Arc<Self>) -> std::io::Result<JoinHandle<()>> {
        let (wake, wanted) = (self.wake.clone(), self.log_wanted.clone());
        self.daemon.logs.set_waker(Arc::new(move || if wanted.load(Ordering::Acquire) { wake.signal() }));
        let bus = self.clone();
        thread::Builder::new()
            .name("fbt-api-bus".into())
            .spawn(move || bus.run())
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.wake.signal();
    }
    pub fn counts(&self) -> Subscriptions {
        let registry = self.registry.lock().unwrap();
        let mut by_topic = BTreeMap::new();
        for sub in registry.subscriptions.values() {
            for topic in &sub.topics {
                *by_topic.entry(topic.topic.as_str()).or_insert(0) += 1;
            }
        }
        Subscriptions {
            total: registry.subscriptions.len() as u64,
            by_topic,
        }
    }
    pub fn remove_connection(&self, id: u64) {
        let mut r = self.registry.lock().unwrap();
        r.subscriptions
            .retain(|(connection, _), _| *connection != id);
        r.next.remove(&id);
        drop(r);
        self.wake.signal();
    }
    pub fn unsubscribe(&self, id: u64, sub: u64) -> Reply {
        self.registry
            .lock()
            .unwrap()
            .subscriptions
            .remove(&(id, sub));
        self.wake.signal();
        Reply::Unsubscribe(EmptyResult {})
    }
    /// Prepare without emitting: the server must queue the successful response
    /// before activation. Otherwise a fast sampler could precede that response.
    pub fn subscribe(&self, sink: EventSink, args: &SubscribeArgs) -> ApiResult<Reply> {
        for topic in &args.topics {
            if let Topic::Search(id) = topic { self.daemon.search.reclaim(sink.id(), *id)?; }
        }
        // Validation reads record hashes only: no sample, no side effects.
        let details = args.topics.iter().any(|t| matches!(t, Topic::Files(_) | Topic::Peers(_) | Topic::Pieces(_) | Topic::Trackers(_)));
        let records = if details { self.record_hashes()? } else { BTreeSet::new() };
        for topic in &args.topics {
            // An unreadable store is not an unknown id: the topic waits for a
            // readable file instead of refusing the whole subscription.
            let known=|result:ApiResult<()>|match result {Err(e) if e.code==ErrorCode::NotFound=>Err(e),_=>Ok(())};
            if let Topic::ShowReleases(id)=topic {if id!="inbox"{known(ops::shows::require(&self.daemon,id).map(drop))?;}}
            if let Topic::RssItems(id)=topic {known(ops::rss::feed(&self.daemon,id).map(drop))?;}
            if let Topic::Files(hash) | Topic::Peers(hash) | Topic::Pieces(hash) | Topic::Trackers(hash) = topic {
                if !records.contains(hash.as_str()) {
                    let mut e = error(ErrorCode::NotFound, "unknown torrent", None);
                    e.details = Some(serde_json::Map::from_iter([("hash".into(), json!(hash))]));
                    return Err(e);
                }
            }
        }
        let mut r = self.registry.lock().unwrap();
        let total: usize = r
            .subscriptions
            .iter()
            .filter(|((id, _), _)| *id == sink.id())
            .map(|(_, s)| s.topics.len())
            .sum();
        if total + args.topics.len() > MAX_SUBSCRIPTIONS {
            let mut e = error(ErrorCode::Busy, "subscription limit", None);
            e.details = Some(serde_json::Map::from_iter([(
                "limit".into(),
                json!("subs"),
            )]));
            return Err(e);
        }
        let next = r.next.entry(sink.id()).or_insert(0);
        *next += 1;
        let sub = *next;
        let summary_interval = sink.summary_interval();
        r.subscriptions.insert(
            (sink.id(), sub),
            Subscription {
                sub,
                sink,
                rev: 0,
                topics: args
                    .topics
                    .iter()
                    .cloned()
                    .map(|topic| TopicState {
                        interval: match topic {
                            Topic::Stats=>Duration::from_secs(5),
                            Topic::Network | Topic::Settings | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_) => {
                                Duration::from_secs(86400)
                            }
                            Topic::Pieces(_) => Duration::from_millis(args.effective_interval_ms(2000).max(2000)),
                            Topic::Search(_) => Duration::from_millis(args.effective_interval_ms(500)),
                            Topic::Summary => {
                                Duration::from_millis(args.effective_interval_ms(summary_interval))
                            }
                            _ => Duration::from_millis(args.effective_interval_ms(1000)),
                        },
                        topic,
                        due: Instant::now(),
                        sent: None,
                    })
                    .collect(),
                since: args.since_seq,
                epoch: None,
                active: false,
            },
        );
        Ok(Reply::Subscribe(SubscribeResult {
            sub,
            topics: args.topics.clone(),
        }))
    }
    pub fn activate(&self, id: u64, sub: u64) {
        if let Some(s) = self
            .registry
            .lock()
            .unwrap()
            .subscriptions
            .get_mut(&(id, sub))
        {
            s.active = true;
        }
        self.wake.signal();
    }
    pub fn publish(&self, value: Value) -> Value {
        let value = self.registry.lock().unwrap().events.append(value);
        self.wake.signal();
        value
    }
    /// torrent.added/torrent.error transitions and engine completions go to
    /// the notifier and the event ring, with or without subscribers.
    fn transitions(&self, r: &mut Registry, state: &str, rows: &[Torrent], completions: &[Value],
                   ready_seen: &mut bool, previous_rows: &mut BTreeMap<String, (bool, Option<String>)>) {
        let restored_baseline = !*ready_seen;
        if matches!(state, "starting" | "stopping") {
            return;
        }
        *ready_seen = true;
        for row in rows {
            let old = previous_rows.get(&row.hash.to_string());
            let event = if !row.staged && old.is_none_or(|previous| previous.0)
                && (old.is_some() || !restored_baseline
                    || !self.initial_hashes.contains(&row.hash.to_string()))
            {
                Some(
                    json!({"ev":"torrent.added","hash":row.hash,"name":row.name,"origin":row.origin.as_deref().unwrap_or("manual")}),
                )
            } else if old.is_some_and(|o| o.1 != row.error)
                && row.error.is_some()
            {
                Some(
                    json!({"ev":"torrent.error","hash":row.hash,"name":row.name,"message":row.error}),
                )
            } else {
                None
            };
            if let Some(event) = event {
                self.daemon.notifier.event(&event);
                r.events.append(event);
            }
        }
        for event in completions {
            self.daemon.notifier.event(event);
            r.events.append(event.clone());
        }
        *previous_rows = rows
            .iter()
            .map(|row| (row.hash.to_string(), (row.staged, row.error.clone())))
            .collect();
    }
    /// Sleep until the next topic deadline or fallback observation. Search
    /// sessions are polled every 100 ms; otherwise slot swaps, the engine,
    /// ledger, logs and stores wake the loop and it rechecks once a second.
    fn idle_wait(&self, next_observation: Instant) -> Duration {
        let now = Instant::now();
        let cap = if self.daemon.search.count() > 0 { Duration::from_millis(100) } else { Duration::from_secs(1) };
        let deadline = self.registry.lock().unwrap().subscriptions.values()
            .flat_map(|s| s.topics.iter()).map(|t| t.due).min()
            .map_or(next_observation, |due| due.min(next_observation));
        deadline.saturating_duration_since(now).min(cap).max(Duration::from_millis(1))
    }
    fn run(&self) {
        let mut observer: Option<rqbit_runtime::task::JoinHandle<()>> = None;
        let mut generation = None;
        let mut seen_generation = 0;
        let mut identity = None;
        // Only what the added/error transitions compare: (staged, error).
        let mut previous_rows: BTreeMap<String, (bool, Option<String>)> = BTreeMap::new();
        let mut ready_seen = false;
        let mut dirty = true;
        let mut generation_threads = Vec::new();
        let mut generation_stop: Option<Arc<AtomicBool>> = None;
        let mut config_stamp = None;
        let mut rss_stamp = None;
        let mut shows_stamp=None;
        // Without a subscriber the owner is observed on wakes (ledger, engine
        // state, completions, config) and on this fallback: every second while
        // a transfer is active (watch, policy), otherwise every five seconds.
        let mut next_observation = Instant::now();
        let mut running;
        while !self.stopping.load(Ordering::Acquire) {
            self.daemon.search.sweep();
            self.daemon.policy.lock().unwrap().searches(self.daemon.search.count(), Instant::now());
            for event in self.daemon.search.completed() {
                let mut r = self.registry.lock().unwrap();
                let event = r.events.append(event);
                let id = event["search_id"].as_u64().unwrap();
                let mut sent = BTreeSet::new();
                for sub in r.subscriptions.values().filter(|s| s.active && s.topics.iter().any(|t| t.topic == Topic::Search(id))) {
                    if sent.insert(sub.sink.id()) { let _ = sub.sink.send(serde_json::from_value(event.clone()).unwrap()); }
                }
                dirty = true;
            }

            {
                let slot = self.daemon.slot.read().unwrap_or_else(|e| e.into_inner());
                let current = (slot.engine_epoch, slot.slot.state().name().to_owned());
                running = matches!(&slot.slot, Slot::Running(_));
                let next_generation = match &slot.slot {
                    Slot::Running(app) => Some(app.dispatcher().ledger_generation()),
                    Slot::Parked(ledger) => Some(ledger.ledger_generation()),
                    _ => None,
                };
                let changed = identity.as_ref() != Some(&current)
                    || next_generation
                        .as_ref()
                        .is_some_and(|g| g.current() != seen_generation);
                if changed {
                    dirty = true;
                    let new_source = match (&generation, &next_generation) {
                        (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                        (None, None) => false,
                        _ => true,
                    };
                    if new_source {
                        if let Some(stop) = generation_stop.take() {
                            stop.store(true, Ordering::Release);
                        }
                        if let Some(g) = next_generation.clone() {
                            let wake = self.wake.clone();
                            let stop = Arc::new(AtomicBool::new(false));
                            generation_stop = Some(stop.clone());
                            generation_threads.push(thread::spawn(move || {
                                let mut seen = g.current();
                                while !stop.load(Ordering::Acquire) {
                                    if let Some(next) = g.wait_after(seen, Duration::from_secs(1)) {
                                        seen = next;
                                        wake.signal();
                                    }
                                }
                            }));
                        }
                    }
                    generation_threads.retain(|worker: &JoinHandle<()>| !worker.is_finished());
                    if let Some(w) = observer.take() {
                        w.abort();
                    }
                    generation = next_generation;
                    seen_generation = generation.as_ref().map_or(0, |g| g.current());
                    if let Slot::Running(app) = &slot.slot {
                        let wake = self.wake.clone();
                        observer =
                            Some(app.dispatcher().bus_watch(Arc::new(move || wake.signal())));
                    }
                    identity = Some(current);
                }
            }
            let stamp = std::fs::metadata(crate::config::config_path())
                .ok()
                .and_then(|m| m.modified().ok());
            if stamp != config_stamp {
                dirty = true;
                config_stamp = stamp;
            }
            if self.registry.lock().unwrap().subscriptions.values().any(|s|s.topics.iter().any(|t|matches!(t.topic,Topic::RssFeeds|Topic::RssItems(_)))) {
                let stamp=self.daemon.rss.stamp();
                if stamp!=rss_stamp {rss_stamp=stamp;dirty=true;}
            }
            if self.registry.lock().unwrap().subscriptions.values().any(|s|s.topics.iter().any(|t|matches!(t.topic,Topic::Shows|Topic::ShowReleases(_)))) {
                let stamp=self.daemon.shows.stamp();if stamp!=shows_stamp{shows_stamp=stamp;dirty=true;}
            }
            {let registry=self.registry.lock().unwrap();self.peers.retain_if(|hash|registry.subscriptions.values().any(|s|s.topics.iter().any(|t|matches!(&t.topic,Topic::Peers(h) if h.as_str()==hash))));}
            let (due, subscribed) = {
                let registry = self.registry.lock().unwrap();
                self.log_wanted.store(registry.subscriptions.values().any(|s| s.topics.iter().any(|t| t.topic == Topic::Log)), Ordering::Release);
                let now = Instant::now();
                (registry.subscriptions.values().filter(|s| s.active).flat_map(|s| s.topics.iter()).any(|t| t.due <= now),
                 !registry.subscriptions.is_empty())
            };
            // A running engine records rate history from its own one-second
            // tick; without one the ring advances with zero rates here.
            if !running { self.daemon.history.lock().unwrap().record(Instant::now(), 0, 0); }
            if Instant::now() >= next_observation { dirty = true; }
            if !dirty && !due {
                dirty = self.wake.wait(self.idle_wait(next_observation));
                continue;
            }
            dirty = false;
            if !subscribed {
                match self.observe(&[]) {
                    Ok(observed) => {
                        next_observation = Instant::now() + if observed.active { Duration::from_secs(1) } else { Duration::from_secs(5) };
                        let mut r = self.registry.lock().unwrap();
                        self.transitions(&mut r, &observed.state, &observed.rows, &observed.completions, &mut ready_seen, &mut previous_rows);
                    }
                    Err(e) => {
                        eprintln!("ERROR api sample failed message={:?}", e.message);
                        next_observation = Instant::now() + Duration::from_secs(1);
                    }
                }
                continue;
            }
            let topics: Vec<_> = self
                .registry
                .lock()
                .unwrap()
                .subscriptions
                .values()
                .flat_map(|s| s.topics.iter().filter(|t| !matches!(t.topic,Topic::Pieces(_)) || t.due<=Instant::now()).map(|t| t.topic.clone()))
                .collect();
            match self.sample(&topics) {
                Ok((sample, active)) => {
                    next_observation = Instant::now() + if active { Duration::from_secs(1) } else { Duration::from_secs(5) };
                    let mut r = self.registry.lock().unwrap();
                    self.transitions(&mut r, &sample.state, &sample.rows, &sample.completions, &mut ready_seen, &mut previous_rows);
                    let needs_events = r
                        .subscriptions
                        .values()
                        .any(|s| s.topics.iter().any(|t| t.topic == Topic::Events));
                    let oldest = r.events.oldest();
                    let events = if needs_events {
                        r.events.replay(None).2
                    } else {
                        Vec::new()
                    };
                    for subscription in r.subscriptions.values_mut().filter(|s| s.active) {
                        if let Err(failure) = subscription.update(&sample, &events, oldest) {
                            eprintln!(
                                "ERROR api subscription failed message={:?}",
                                failure.message
                            );
                            subscription.sink.close();
                        }
                    }
                    r.subscriptions.retain(|_, s| !s.sink.closed());
                }
                Err(e) => {
                    eprintln!("ERROR api sample failed message={:?}", e.message);
                    let mut r = self.registry.lock().unwrap();
                    for sub in r.subscriptions.values() {
                        let _ = sub.sink.send(Event::Error { error: e.clone() });
                        sub.sink.close();
                    }
                    r.subscriptions.clear();
                }
            }
            // Ledger mutations wake immediately via its Condvar (and the
            // generation thread); see idle_wait for the other wake sources.
            // No async mutex is acquired by this generation wait.
            if let Some(g) = &generation {
                if g.wait_after(seen_generation, Duration::ZERO).is_some() {
                    continue;
                }
            }
            dirty = self.wake.wait(self.idle_wait(next_observation));
        }
        if let Some(w) = observer {
            w.abort();
        }
        if let Some(stop) = generation_stop {
            stop.store(true, Ordering::Release);
        }
        for worker in generation_threads {
            let _ = worker.join();
        }
    }
}
mod sample;
use sample::Sample;
mod update;
mod diff;
use diff::{diff, project, state_event};
#[cfg(test)]
use diff::structural_projection;

#[cfg(test)]
#[path = "bus/tests.rs"]
mod tests;
