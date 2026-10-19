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
    sent: Option<Value>,
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
struct Sample {
    epoch: u64,
    state: String,
    rows: Vec<Torrent>,
    completions: Vec<Value>,
    logs: Value,
    searches: BTreeMap<u64, Value>,
    files: BTreeMap<String, Value>,
    network: Value,
    settings: Value,
    disk: Disk,
    shows:Value,
    show_releases:BTreeMap<String,Value>,
    rss_feeds: Value,
    rss_items: BTreeMap<String,Value>,
    daemon: Value,
}
impl Sample {
    fn value(&self, topic: &Topic) -> Value {
        match topic {
            Topic::Shows=>self.shows.clone(),Topic::ShowReleases(id)=>self.show_releases.get(id).cloned().unwrap_or(json!([])),
            Topic::RssFeeds=>self.rss_feeds.clone(),
            Topic::RssItems(id)=>self.rss_items.get(id).cloned().unwrap_or(json!([])),
            Topic::Torrents => serde_json::to_value(&self.rows).unwrap(),
            Topic::Files(h) => self.files.get(&h.to_string()).cloned().unwrap_or(json!([])),
            Topic::Network => self.network.clone(),
            Topic::Settings => self.settings.clone(),
            Topic::Summary => self.summary(),
            Topic::Search(id) => self.searches.get(id).cloned().unwrap_or(json!([])),
            Topic::Events => unreachable!(),
            Topic::Log => self.logs.clone(),
        }
    }
    fn summary(&self) -> Value {
        let counts = summary_counts(&self.rows);
        let top: Vec<_> = self
            .rows
            .iter()
            .filter(|row| !row.staged)
            .take(5)
            .map(|r| {
                let mut v = json!({"hash":r.hash,"name":r.name,"state":r.state});
                for (key, value) in [
                    ("progress", json!(r.progress)),
                    ("dl_bps", json!(r.dl_bps)),
                    ("eta_s", json!(r.eta_s)),
                ] {
                    if !value.is_null() {
                        v[key] = value;
                    }
                }
                v
            })
            .collect();
        let mut result = json!({"counts":counts,"alt":self.settings["alt_limits_on"].as_bool().unwrap_or(false),"limits":ops::transfer::limits_from_settings(&self.settings),"network":{"state":if self.network["ready"]==true {"ready"}else{"waiting"},"message":self.network["message"]},"daemon":self.daemon,"top":top,"disk":self.disk});
        if let Some(v) = self.network.get("interface") {
            result["network"]["interface"] = v.clone();
        }
        if self.state == "running" {
            result["dl_bps"] = json!(self.rows.iter().filter_map(|r| r.dl_bps).sum::<u64>());
            result["ul_bps"] = json!(self.rows.iter().filter_map(|r| r.ul_bps).sum::<u64>());
        }
        result
    }
}

pub(crate) struct Bus {
    daemon: Arc<Daemon>,
    registry: Mutex<Registry>,
    wake: Arc<Wake>,
    stopping: AtomicBool,
    initial_hashes: BTreeSet<String>,
}
impl Bus {
    pub(crate) fn refresh(&self) { self.wake.signal(); }
    pub fn new(daemon: Arc<Daemon>) -> Arc<Self> {
        Arc::new(Self {
            daemon,
            registry: Mutex::default(),
            wake: Arc::default(),
            stopping: AtomicBool::new(false),
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
        let wake = self.wake.clone();
        self.daemon.logs.set_waker(Arc::new(move || wake.signal()));
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
        let sample = self.sample(&args.topics)?;
        for topic in &args.topics {
            if let Topic::ShowReleases(id)=topic {if id!="inbox"{ops::shows::require(&self.daemon,id)?;}}
            if let Topic::RssItems(id)=topic {ops::rss::feed(&self.daemon,id)?;}
            if let Topic::Files(hash) = topic {
                if !sample.rows.iter().any(|r| &r.hash == hash) {
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
                            Topic::Network | Topic::Settings | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_) => {
                                Duration::from_secs(86400)
                            }
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
    fn sample(&self, topics: &[Topic]) -> ApiResult<Sample> {
        let files: Vec<_> = topics
            .iter()
            .filter_map(|t| {
                if let Topic::Files(h) = t {
                    Some(h.to_string())
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let slot = self.daemon.slot.read().unwrap_or_else(|e| e.into_inner());
        let epoch = slot.engine_epoch;
        let state = slot.slot.state().name().to_owned();
        let (rows, details, restoring) = match &slot.slot {
            Slot::Running(app) => app
                .dispatcher()
                .bus_sample(&files)
                .map_err(ops::engine_error)?,
            Slot::Parked(ledger) => {
                // Parked operations are serialized internally. A generation
                // check retries if a mutation crossed the collection reads.
                loop {
                    let g = ledger.ledger_generation();
                    let before = g.current();
                    let EngineResult::Torrents(rows) =
                        ledger.execute(EngineOp::Info).map_err(ops::engine_error)?
                    else {
                        unreachable!()
                    };
                    let mut details = std::collections::HashMap::new();
                    for h in &files {
                        if let Ok(EngineResult::Files(rows)) =
                            ledger.execute(EngineOp::Files { hash: h.clone() })
                        {
                            details.insert(h.clone(), rows);
                        }
                    }
                    if before == g.current() {
                        break (rows, details, false);
                    }
                }
            }
            Slot::Starting | Slot::Stopping => (Vec::new(), std::collections::HashMap::new(), false),
        };
        let mut completions = match &slot.slot {
            Slot::Running(app) => app.dispatcher().take_completions(),
            _ => Vec::new(),
        };
        if let Slot::Running(app)=&slot.slot {
            let mut watch=self.daemon.watch.lock().unwrap();
            if watch.due() {
                completions.extend(watch.sample(app.dispatcher().watch_samples().map_err(ops::engine_error)?));
            }
        } else {
            let mut watch=self.daemon.watch.lock().unwrap();
            if watch.due() {watch.sample(Vec::new());}
        }
        self.daemon.disk.lock().unwrap().observe(&rows);
        if !self.daemon.is_stopping() { self.daemon.inhibitor.observe(&rows); }
        let mut rows: Vec<Torrent> = rows
            .into_iter()
            .map(ops::torrents::torrent)
            .collect::<ApiResult<Vec<_>>>()?;
        self.daemon.watch.lock().unwrap().decorate(&mut rows);
        let files = details
            .into_iter()
            .map(|(hash, files)| {
                let playable=rows.iter().find(|r|r.hash.as_str()==hash).and_then(|r|r.playable.clone());
                (
                    hash,
                    json!(files
                        .into_iter()
                        .map(|f| File {
                            index: f.index as u64,
                            path: f.name.clone(),
                            size: f.size.max(0) as u64,
                            progress: f.progress,
                            included: f.priority != 0,
                            sort_key: ops::torrents::sort_key(&f.name),
                            ready: playable.as_ref().filter(|p|p.file==f.index as u64).map(|p|p.ready)
                        })
                        .collect::<Vec<_>>()),
                )
            })
            .collect();
        let cfg = crate::config::Config::load().map_err(ops::engine_error)?;
        let activity = crate::daemon::policy::Transfers::from_torrents(&rows, &state, restoring);
        let mut searches = BTreeMap::new();
        for id in topics.iter().filter_map(|t| if let Topic::Search(id) = t { Some(*id) } else { None }).collect::<BTreeSet<_>>() {
            let mut items = self.daemon.search.items(id).unwrap_or_default();
            let hashes: BTreeSet<_> = rows.iter().map(|r| r.hash.clone()).collect();
            for item in &mut items {
                item.added = Hash::new(&item.key).ok().filter(|h| hashes.contains(h));
            }
            searches.insert(id, json!(items));
        }
        let mut daemon = {
            let mut policy = self.daemon.policy.lock().unwrap();
            let now = Instant::now();
            policy.configure(cfg.idle_exit_secs, now);
            policy.searches(self.daemon.search.count(), now);
            policy.transfers(activity, now);
            serde_json::to_value(policy.snapshot(now)).unwrap()
        };
        daemon["state"] = json!(state);
        daemon["upgrade_pending"] = json!(self.daemon.upgrade_pending.load(Ordering::Acquire));
        self.daemon.notifier.enabled(cfg.notify && !self.daemon.no_notify);
        let network = serde_json::to_value(self.daemon.network()).unwrap();
        let mut effective = cfg;
        effective.idle_exit_secs = self.daemon.idle_exit_secs();
        effective.notify &= !self.daemon.no_notify;
        let settings = serde_json::to_value(ops::settings::view(&effective)).unwrap();
        let mut show_releases=BTreeMap::new();
        for id in topics.iter().filter_map(|t|if let Topic::ShowReleases(id)=t{Some(id)}else{None}) {
            show_releases.insert(id.clone(),serde_json::to_value(self.daemon.shows.releases((id!="inbox").then_some(id.as_str()),id=="inbox",None,&rows).map_err(ops::shows::failure)?).unwrap());
        }
        Ok(Sample {
            shows:if topics.contains(&Topic::Shows){serde_json::to_value(self.daemon.shows.list().map_err(ops::shows::failure)?).unwrap()}else{json!([])},
            show_releases,

            rss_feeds: if topics.contains(&Topic::RssFeeds){serde_json::to_value(self.daemon.rss.feeds().map_err(ops::rss::failure)?).unwrap()}else{json!([])},
            rss_items: topics.iter().filter_map(|t|if let Topic::RssItems(id)=t{Some(id)}else{None}).map(|id|Ok((id.clone(),serde_json::to_value(self.daemon.rss.snapshot().map_err(ops::rss::failure)?.items.get(id).cloned().unwrap_or_default()).unwrap()))).collect::<ApiResult<_>>()?,
            epoch,
            state,
            rows,
            completions,
            logs: serde_json::to_value(self.daemon.logs.all()).unwrap(),
            searches,
            files,
            network,
            settings,
            disk: self.daemon.disk.lock().unwrap().summary.clone(),
            daemon,
        })
    }
    fn run(&self) {
        let mut observer: Option<rqbit_runtime::task::JoinHandle<()>> = None;
        let mut generation = None;
        let mut seen_generation = 0;
        let mut identity = None;
        let mut previous_rows: BTreeMap<String, Torrent> = BTreeMap::new();
        let mut previous_engine = None;
        let mut ready_seen = false;
        let mut dirty = true;
        let mut generation_threads = Vec::new();
        let mut generation_stop: Option<Arc<AtomicBool>> = None;
        let mut config_stamp = None;
        let mut rss_stamp = None;
        let mut shows_stamp=None;
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
            let due = self
                .registry
                .lock()
                .unwrap()
                .subscriptions
                .values()
                .filter(|s| s.active)
                .flat_map(|s| s.topics.iter())
                .any(|t| t.due <= Instant::now());
            if self.daemon.watch.lock().unwrap().due() { dirty = true; }
            if !dirty && !due {
                dirty = self.wake.wait(Duration::from_millis(100));
                continue;
            }
            dirty = false;
            let topics: Vec<_> = self
                .registry
                .lock()
                .unwrap()
                .subscriptions
                .values()
                .flat_map(|s| s.topics.iter().map(|t| t.topic.clone()))
                .collect();
            match self.sample(&topics) {
                Ok(sample) => {
                    let mut r = self.registry.lock().unwrap();
                    let engine = (sample.epoch, sample.state.clone());
                    if ready_seen && previous_engine.as_ref().is_some_and(|p| p != &engine) {
                        let event=r.events.append(json!({"ev":"engine.changed","epoch":sample.epoch,"state":sample.state,"reason":"engine_restart"}));
                        let mut connections = BTreeSet::new();
                        for s in r.subscriptions.values() {
                            if connections.insert(s.sink.id()) {
                                let _ = s.sink.send(serde_json::from_value(event.clone()).unwrap());
                            }
                        }
                    }
                    let restored_baseline = !ready_seen;
                    previous_engine = Some(engine);
                    if !matches!(sample.state.as_str(), "starting" | "stopping") {
                        ready_seen = true;
                        for row in &sample.rows {
                            let old = previous_rows.get(&row.hash.to_string());
                            let event = if old.is_none()
                                && (!restored_baseline
                                    || !self.initial_hashes.contains(&row.hash.to_string()))
                            {
                                Some(
                                    json!({"ev":"torrent.added","hash":row.hash,"name":row.name,"origin":row.origin.as_deref().unwrap_or("manual")}),
                                )
                            } else if old.is_some_and(|o| o.error != row.error)
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
                        for event in &sample.completions {
                            self.daemon.notifier.event(event);
                            r.events.append(event.clone());
                        }
                        previous_rows = sample
                            .rows
                            .iter()
                            .cloned()
                            .map(|row| (row.hash.to_string(), row))
                            .collect();
                    }
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
            // Ledger mutations wake immediately via its Condvar. The short
            // bound observes slot swaps and also bounds shutdown when parked;
            // handle state transitions wake the bus independently below.
            let deadline = self
                .registry
                .lock()
                .unwrap()
                .subscriptions
                .values()
                .flat_map(|s| s.topics.iter())
                .map(|t| t.due)
                .min()
                .unwrap_or(Instant::now() + Duration::from_secs(1));
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            // No async mutex is acquired by this generation wait.
            if let Some(g) = &generation {
                if g.wait_after(seen_generation, Duration::ZERO).is_some() {
                    continue;
                }
            }
            dirty = self.wake.wait(timeout.max(Duration::from_millis(1)));
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
impl Subscription {
    fn update(&mut self, sample: &Sample, events: &[Value], oldest: u64) -> ApiResult<()> {
        // A slot without a usable ledger is transitional, not an empty saved
        // history. Keep detail subscriptions until the replacement is ready.
        if matches!(sample.state.as_str(), "starting" | "stopping") {
            // An initial snapshot stays pending, but must not leave a past-due
            // deadline spinning the sampler while the engine opens/drains.
            for topic in &mut self.topics {
                topic.due = Instant::now() + Duration::from_millis(100);
            }
            return Ok(());
        }
        let engine = (sample.epoch, sample.state.clone());
        let restart = self.epoch.as_ref().is_some_and(|e| e != &engine);
        // State topics precede keyed snapshots, matching the contract fixture.
        let mut order: Vec<_> = (0..self.topics.len()).collect();
        order.sort_by_key(|i| {
            matches!(
                self.topics[*i].topic,
                Topic::Torrents | Topic::Files(_) | Topic::Search(_) | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_)
            )
        });
        for index in order {
            let t = &mut self.topics[index];
            let mut value = if t.topic == Topic::Events {
                json!(events
                    .iter()
                    .filter(|e| self.since.is_none_or(|s| e["seq"].as_u64().unwrap() > s))
                    .collect::<Vec<_>>())
            } else {
                sample.value(&t.topic)
            };
            let keyed = matches!(t.topic, Topic::Torrents | Topic::Files(_) | Topic::Search(_) | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_));
            let reset =
                restart && matches!(t.topic, Topic::Summary | Topic::Torrents | Topic::Files(_));
            if t.sent.is_none() || reset {
                if keyed {
                    let resync = t.topic == Topic::Events
                        && self.since.is_some_and(|s| s.saturating_add(1) < oldest);
                    if resync {
                        value = json!(events);
                    }
                    let items = value
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned()
                        .map(serde_json::from_value)
                        .collect::<Result<Vec<Item>, _>>()
                        .map_err(|_| error(ErrorCode::Internal, "invalid snapshot", None))?;
                    self.rev = self.sink.snapshot_resync(
                        self.sub,
                        t.topic.clone(),
                        self.rev,
                        items,
                        if reset {
                            SnapshotReason::EngineRestart
                        } else if resync {
                            SnapshotReason::Resync
                        } else {
                            SnapshotReason::Subscribe
                        },
                        if resync { Some(oldest) } else { None },
                    )?;
                } else {
                    self.rev += 1;
                    self.sink
                        .send(state_event(self.sub, &t.topic, self.rev, value.clone())?)?;
                }
                t.sent = Some(value);
                t.due = Instant::now() + t.interval;
            } else {
                let old = t.sent.as_ref().unwrap();
                let due = Instant::now() >= t.due;
                let next = if due {
                    value
                } else {
                    structural_projection(&t.topic, old, &value)
                };
                if &next != old {
                    if keyed {
                        let (changed, added, removed) = diff(&t.topic, old, &next);
                        if !changed.is_empty() || !added.is_empty() || !removed.is_empty() {
                            // Split additions using EventSink's byte/count bounds.
                            self.rev = self.sink.added(
                                self.sub,
                                t.topic.clone(),
                                self.rev,
                                added
                                    .into_iter()
                                    .map(serde_json::from_value)
                                    .collect::<Result<Vec<Item>, _>>()
                                    .map_err(|_| {
                                        error(ErrorCode::Internal, "invalid delta", None)
                                    })?,
                            )?;
                            if !changed.is_empty() || !removed.is_empty() {
                                self.rev += 1;
                                self.sink.send(Event::Delta {
                                    sub: self.sub,
                                    topic: t.topic.clone(),
                                    rev: self.rev,
                                    changed: (!changed.is_empty()).then_some(changed),
                                    added: None,
                                    removed: (!removed.is_empty()).then_some(removed),
                                })?;
                            }
                        }
                    } else {
                        self.rev += 1;
                        self.sink
                            .send(state_event(self.sub, &t.topic, self.rev, next.clone())?)?;
                    }
                    t.sent = Some(next);
                }
                if due {
                    t.due = Instant::now() + t.interval;
                }
            }
        }
        // An absent detail after restart gets exactly one empty snapshot; its
        // sampling is then released while the owning connection remains open.
        if restart {
            self.topics.retain(
                |t| !matches!(&t.topic,Topic::Files(h) if !sample.rows.iter().any(|r|&r.hash==h)),
            );
        }
        self.epoch = Some(engine);
        Ok(())
    }
}
mod diff;
use diff::{diff, state_event, structural_projection};

#[cfg(test)]
#[path = "bus/tests.rs"]
mod tests;
