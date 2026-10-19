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
struct Sample {
    topics: BTreeSet<Topic>,
    /// Collected topics whose own source failed; they keep their last value.
    failed: BTreeSet<Topic>,
    history:Vec<u64>,stats:Value,
    epoch: u64,
    state: String,
    rows: Vec<Torrent>,
    completions: Vec<Value>,
    logs: Value,
    searches: BTreeMap<u64, Value>,
    files: BTreeMap<String, Value>,
    peers: BTreeMap<String, Value>,
    pieces: BTreeMap<String, Value>,
    trackers: BTreeMap<String, Value>,
    network: Value,
    settings: Value,
    disk: Disk,
    shows:Value,
    show_releases:BTreeMap<String,Value>,
    rss_feeds: Value,
    rss_items: BTreeMap<String,Value>,
    daemon: Value,
    /// Each topic value is built once per sample, for every subscription.
    memo: Memo,
}
#[derive(Default)]
struct Memo(std::cell::RefCell<BTreeMap<Topic, Arc<Value>>>);
impl Sample {
    fn value(&self, topic: &Topic) -> Arc<Value> {
        if let Some(value) = self.memo.0.borrow().get(topic) {
            return value.clone();
        }
        let value = Arc::new(self.build(topic));
        self.memo.0.borrow_mut().insert(topic.clone(), value.clone());
        value
    }
    fn build(&self, topic: &Topic) -> Value {
        match topic {
            Topic::Stats=>self.stats.clone(),
            Topic::Shows=>self.shows.clone(),Topic::ShowReleases(id)=>self.show_releases.get(id).cloned().unwrap_or(json!([])),
            Topic::RssFeeds=>self.rss_feeds.clone(),
            Topic::RssItems(id)=>self.rss_items.get(id).cloned().unwrap_or(json!([])),
            Topic::Torrents => serde_json::to_value(&self.rows).unwrap(),
            Topic::Trackers(h) => self.trackers.get(h.as_str()).cloned().unwrap_or(json!([])),
            Topic::Peers(h) => self.peers.get(h.as_str()).cloned().unwrap_or(json!([])),
            Topic::Pieces(h) => self.pieces.get(h.as_str()).cloned().unwrap_or(Value::Null),
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
        let top = summary::top(&self.rows);
        let mut result = json!({"counts":counts,"alt":self.settings["alt_limits_on"].as_bool().unwrap_or(false),"limits":ops::transfer::limits_from_settings(&self.settings),"network":{"state":if self.network["ready"]==true {"ready"}else{"waiting"},"message":self.network["message"]},"daemon":self.daemon,"top":top,"disk":self.disk});
        result["history"]=json!(self.history);
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
    peers: peers::Sampler,
    wake: Arc<Wake>,
    stopping: AtomicBool,
    initial_hashes: BTreeSet<String>,
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
            // An unreadable store is not an unknown id: the topic waits for a
            // readable file instead of refusing the whole subscription.
            let known=|result:ApiResult<()>|match result {Err(e) if e.code==ErrorCode::NotFound=>Err(e),_=>Ok(())};
            if let Topic::ShowReleases(id)=topic {if id!="inbox"{known(ops::shows::require(&self.daemon,id).map(drop))?;}}
            if let Topic::RssItems(id)=topic {known(ops::rss::feed(&self.daemon,id).map(drop))?;}
            if let Topic::Files(hash) | Topic::Peers(hash) | Topic::Pieces(hash) | Topic::Trackers(hash) = topic {
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
        self.daemon.disk.lock().unwrap().observe(&rows);
        if !self.daemon.is_stopping() { self.daemon.inhibitor.observe(&rows); }
        if let Slot::Running(app)=&slot.slot {
            let mut watch=self.daemon.watch.lock().unwrap();
            if watch.due() {
                completions.extend(watch.sample(app.dispatcher().watch_samples().map_err(ops::engine_error)?));
            }
        } else {
            let mut watch=self.daemon.watch.lock().unwrap();
            if watch.due() {watch.sample(Vec::new());}
        }
        let mut rows: Vec<Torrent> = rows
            .into_iter()
            .map(ops::torrents::torrent)
            .collect::<ApiResult<Vec<_>>>()?;
        self.daemon.watch.lock().unwrap().decorate(&mut rows);
        let mut pieces=BTreeMap::new();
        if let Slot::Running(app)=&slot.slot {
            for hash in topics.iter().filter_map(|t|if let Topic::Pieces(h)=t {Some(h)}else{None}).collect::<BTreeSet<_>>() {
                let playable=rows.iter().find(|row|&row.hash==hash).and_then(|row|row.playable.as_ref());
                if let Ok(value)=app.dispatcher().pieces_snapshot(hash.as_str()).map_err(ops::engine_error).and_then(|value|ops::pieces::encode(value,playable)) {
                    pieces.insert(hash.to_string(),json!(value));
                }
            }
        }
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
        let (cfg, config_error) = self.daemon.config.current();
        let cfg = (*cfg).clone();
        let mut failed = BTreeSet::new();
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
            match self.daemon.shows.releases((id!="inbox").then_some(id.as_str()),id=="inbox",None,&rows) {
                Ok(releases)=>{show_releases.insert(id.clone(),serde_json::to_value(releases).unwrap());}
                Err(_)=>{failed.insert(Topic::ShowReleases(id.clone()));}
            }
        }
        drop(slot);
        self.peers.retain(topics);
        let mut peers = BTreeMap::new();
        for hash in topics.iter().filter_map(|t|if let Topic::Peers(h)=t{Some(h)}else{None}).collect::<BTreeSet<_>>() {
            if rows.iter().any(|r|&r.hash==hash){
                match self.peers.read(&self.daemon,hash) {
                    Ok(value)=>{peers.insert(hash.to_string(),serde_json::to_value(value).unwrap());}
                    Err(_)=>{failed.insert(Topic::Peers(hash.clone()));}
                }
            }
        }
        let mut trackers=BTreeMap::new();
        for hash in topics.iter().filter_map(|t|if let Topic::Trackers(h)=t{Some(h)}else{None}).collect::<BTreeSet<_>>() {
            if rows.iter().any(|r|&r.hash==hash) {
                match ops::trackers::read(&self.daemon,hash) {
                    Ok(value)=>{trackers.insert(hash.to_string(),serde_json::to_value(value).unwrap());}
                    Err(_)=>{failed.insert(Topic::Trackers(hash.clone()));}
                }
            }
        }
        let (session,all_time)=self.daemon.statistics();
        let (history,stats)={let mut ring=self.daemon.history.lock().unwrap();
            ring.record(Instant::now(),rows.iter().filter_map(|r|r.dl_bps).sum(),rows.iter().filter_map(|r|r.ul_bps).sum());
            (ring.download(60),if topics.contains(&Topic::Stats){json!(SampledStats {history:ring.snapshot(),session:Some(session),all_time:Some(all_time)})}else{Value::Null})};
        let shows = match topics.contains(&Topic::Shows).then(|| self.daemon.shows.list()) {
            Some(Ok(list)) => serde_json::to_value(list).unwrap(),
            Some(Err(_)) => {failed.insert(Topic::Shows); json!([])}
            None => json!([]),
        };
        let rss_feeds = match topics.contains(&Topic::RssFeeds).then(|| self.daemon.rss.feeds()) {
            Some(Ok(feeds)) => serde_json::to_value(feeds).unwrap(),
            Some(Err(_)) => {failed.insert(Topic::RssFeeds); json!([])}
            None => json!([]),
        };
        let mut rss_items = BTreeMap::new();
        let item_topics: Vec<_> = topics.iter().filter_map(|t|if let Topic::RssItems(id)=t{Some(id)}else{None}).collect();
        if !item_topics.is_empty() {
            match self.daemon.rss.snapshot() {
                Ok(snapshot) => for id in item_topics {
                    rss_items.insert(id.clone(), serde_json::to_value(snapshot.items.get(id).cloned().unwrap_or_default()).unwrap());
                },
                Err(_) => failed.extend(item_topics.into_iter().map(|id| Topic::RssItems(id.clone()))),
            }
        }
        // Report each unreadable local file on the always-subscribed summary.
        let warnings: Vec<_> = [("config", config_error), ("shows", self.daemon.shows.problem()), ("rss", self.daemon.rss.problem())]
            .into_iter().filter_map(|(source, message)| message.map(|message| json!({"source":source,"message":message}))).collect();
        if !warnings.is_empty() { daemon["warnings"] = json!(warnings); }
        Ok(Sample {
            topics: topics.iter().cloned().collect(),
            failed,
            history,stats,
            peers,trackers,
            shows,
            show_releases,
            rss_feeds,
            rss_items,
            epoch,
            state,
            rows,
            completions,
            // The log ring is copied only for a Log subscriber.
            logs: if topics.contains(&Topic::Log) { serde_json::to_value(self.daemon.logs.all()).unwrap() } else { json!([]) },
            memo: Memo::default(),
            searches,
            files,
            pieces,
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
        // Only what the added/error transitions compare: (staged, error).
        let mut previous_rows: BTreeMap<String, (bool, Option<String>)> = BTreeMap::new();
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
            {let registry=self.registry.lock().unwrap();self.peers.retain_if(|hash|registry.subscriptions.values().any(|s|s.topics.iter().any(|t|matches!(&t.topic,Topic::Peers(h) if h.as_str()==hash))));}
            let due = self
                .registry
                .lock()
                .unwrap()
                .subscriptions
                .values()
                .filter(|s| s.active)
                .flat_map(|s| s.topics.iter())
                .any(|t| t.due <= Instant::now());
            if self.daemon.watch.lock().unwrap().due() || self.daemon.history.lock().unwrap().due(Instant::now()) { dirty = true; }
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
                .flat_map(|s| s.topics.iter().filter(|t| !matches!(t.topic,Topic::Pieces(_)) || t.due<=Instant::now()).map(|t| t.topic.clone()))
                .collect();
            match self.sample(&topics) {
                Ok(sample) => {
                    let mut r = self.registry.lock().unwrap();
                    let restored_baseline = !ready_seen;
                    if !matches!(sample.state.as_str(), "starting" | "stopping") {
                        ready_seen = true;
                        for row in &sample.rows {
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
                        for event in &sample.completions {
                            self.daemon.notifier.event(event);
                            r.events.append(event.clone());
                        }
                        previous_rows = sample
                            .rows
                            .iter()
                            .map(|row| (row.hash.to_string(), (row.staged, row.error.clone())))
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
mod update;
mod diff;
use diff::{diff, project, state_event};
#[cfg(test)]
use diff::structural_projection;

#[cfg(test)]
#[path = "bus/tests.rs"]
mod tests;
