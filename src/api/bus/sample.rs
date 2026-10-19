//! One shared observation of the owner, then the value of each subscribed
//! topic. The observation alone runs without subscribers: it feeds the
//! owner's disk, inhibitor, idle-policy, watch and statistics side effects.
use super::*;
use std::collections::HashMap;

pub(super) struct Sample {
    pub(super) topics: BTreeSet<Topic>,
    /// Collected topics whose own source failed; they keep their last value.
    pub(super) failed: BTreeSet<Topic>,
    pub(super) history: Vec<u64>,
    pub(super) stats: Value,
    pub(super) epoch: u64,
    pub(super) state: String,
    pub(super) rows: Vec<Torrent>,
    pub(super) completions: Vec<Value>,
    pub(super) logs: Value,
    pub(super) searches: BTreeMap<u64, Value>,
    pub(super) files: BTreeMap<String, Value>,
    pub(super) peers: BTreeMap<String, Value>,
    pub(super) pieces: BTreeMap<String, Value>,
    pub(super) trackers: BTreeMap<String, Value>,
    pub(super) network: Value,
    pub(super) settings: Value,
    pub(super) disk: Disk,
    pub(super) shows: Value,
    pub(super) show_releases: BTreeMap<String, Value>,
    pub(super) rss_feeds: Value,
    pub(super) rss_items: BTreeMap<String, Value>,
    pub(super) daemon: Value,
    /// Each topic value is built once per sample, for every subscription.
    pub(super) memo: Memo,
}
#[derive(Default)]
pub(super) struct Memo(pub(super) std::cell::RefCell<BTreeMap<Topic, Arc<Value>>>);
impl Sample {
    pub(super) fn value(&self, topic: &Topic) -> Arc<Value> {
        if let Some(value) = self.memo.0.borrow().get(topic) {
            return value.clone();
        }
        let value = Arc::new(self.build(topic));
        self.memo.0.borrow_mut().insert(topic.clone(), value.clone());
        value
    }
    fn build(&self, topic: &Topic) -> Value {
        match topic {
            Topic::Stats => self.stats.clone(),
            Topic::Shows => self.shows.clone(),
            Topic::ShowReleases(id) => self.show_releases.get(id).cloned().unwrap_or(json!([])),
            Topic::RssFeeds => self.rss_feeds.clone(),
            Topic::RssItems(id) => self.rss_items.get(id).cloned().unwrap_or(json!([])),
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
        result["history"] = json!(self.history);
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

/// What every observation collects, with or without subscribers.
pub(super) struct Observation {
    pub(super) epoch: u64,
    pub(super) state: String,
    pub(super) rows: Vec<Torrent>,
    pub(super) completions: Vec<Value>,
    /// A transfer the watch and the idle policy observe every second.
    pub(super) active: bool,
    files: BTreeMap<String, Value>,
    pieces: BTreeMap<String, Value>,
    cfg: Arc<crate::config::Config>,
    config_error: Option<String>,
    daemon: Value,
    totals: (SessionTotals, AllTimeTotals),
}

impl Bus {
    /// Rows and the owner's side effects. `topics` only selects the detail
    /// collections (files, pieces) that need the same slot guard.
    pub(super) fn observe(&self, topics: &[Topic]) -> ApiResult<Observation> {
        let files: Vec<_> = topics
            .iter()
            .filter_map(|t| if let Topic::Files(h) = t { Some(h.to_string()) } else { None })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let slot = self.daemon.slot.read().unwrap_or_else(|e| e.into_inner());
        let epoch = slot.engine_epoch;
        let state = slot.slot.state().name().to_owned();
        let (rows, details, restoring) = match &slot.slot {
            Slot::Running(app) => app.dispatcher().bus_sample(&files).map_err(ops::engine_error)?,
            Slot::Parked(ledger) => {
                // Parked operations are serialized internally. A generation
                // check retries if a mutation crossed the collection reads.
                loop {
                    let g = ledger.ledger_generation();
                    let before = g.current();
                    let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info).map_err(ops::engine_error)? else {
                        unreachable!()
                    };
                    let mut details = HashMap::new();
                    for h in &files {
                        if let Ok(EngineResult::Files(rows)) = ledger.execute(EngineOp::Files { hash: h.clone() }) {
                            details.insert(h.clone(), rows);
                        }
                    }
                    if before == g.current() {
                        break (rows, details, false);
                    }
                }
            }
            Slot::Starting | Slot::Stopping => (Vec::new(), HashMap::new(), false),
        };
        let mut completions = match &slot.slot {
            Slot::Running(app) => app.dispatcher().take_completions(),
            _ => Vec::new(),
        };
        self.daemon.disk.lock().unwrap().observe(&rows);
        // Free space, the low-disk warning and ENOSPC retries use these same
        // rows on the monitor's own five-second cadence; the supervisor no
        // longer lists every torrent for them.
        if matches!(&slot.slot, Slot::Running(_) | Slot::Parked(_)) {
            let (retries, low) = self.daemon.disk.lock().unwrap().sample(&rows, Instant::now());
            if low {
                self.daemon.notifier.disk_low();
            }
            for (hash, free) in retries {
                let result = match &slot.slot {
                    Slot::Running(app) => app.dispatcher().retry_disk_full(&hash),
                    _ => Ok(false),
                };
                // Consume this rise even when the attempt fails; the next
                // ENOSPC observation records a fresh baseline for another rise.
                self.daemon.disk.lock().unwrap().restarted(&hash);
                if result.as_ref().is_ok_and(|retry| *retry) {
                    eprintln!("INFO engine disk_full_recovered hash={hash} free_bytes={free}");
                } else if let Err(error) = result {
                    eprintln!("ERROR engine disk_full retry failed hash={hash} message={:?}", format!("{error:#}"));
                }
            }
        }
        if !self.daemon.is_stopping() {
            self.daemon.inhibitor.observe(&rows);
        }
        if let Slot::Running(app) = &slot.slot {
            let mut watch = self.daemon.watch.lock().unwrap();
            if watch.due() {
                completions.extend(watch.sample(app.dispatcher().watch_samples().map_err(ops::engine_error)?));
            }
        } else {
            let mut watch = self.daemon.watch.lock().unwrap();
            if watch.due() {
                watch.sample(Vec::new());
            }
        }
        let mut rows: Vec<Torrent> = rows.into_iter().map(ops::torrents::torrent).collect::<ApiResult<Vec<_>>>()?;
        self.daemon.watch.lock().unwrap().decorate(&mut rows);
        let mut pieces = BTreeMap::new();
        if let Slot::Running(app) = &slot.slot {
            for hash in topics.iter().filter_map(|t| if let Topic::Pieces(h) = t { Some(h) } else { None }).collect::<BTreeSet<_>>() {
                let playable = rows.iter().find(|row| &row.hash == hash).and_then(|row| row.playable.as_ref());
                if let Ok(value) = app.dispatcher().pieces_snapshot(hash.as_str()).map_err(ops::engine_error).and_then(|value| ops::pieces::encode(value, playable)) {
                    pieces.insert(hash.to_string(), json!(value));
                }
            }
        }
        drop(slot);
        let files = details
            .into_iter()
            .map(|(hash, files)| {
                let playable = rows.iter().find(|r| r.hash.as_str() == hash).and_then(|r| r.playable.clone());
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
                            ready: playable.as_ref().filter(|p| p.file == f.index as u64).map(|p| p.ready),
                        })
                        .collect::<Vec<_>>()),
                )
            })
            .collect();
        let (cfg, config_error) = self.daemon.config.current();
        let activity = crate::daemon::policy::Transfers::from_torrents(&rows, &state, restoring);
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
        // Session/all-time totals accumulate and save on their own cadence.
        let totals = self.daemon.statistics();
        Ok(Observation {
            epoch,
            state,
            rows,
            completions,
            active: activity.downloading || activity.restoring || activity.starting,
            files,
            pieces,
            cfg,
            config_error,
            daemon,
            totals,
        })
    }

    /// One observation plus every requested topic's value, built once; and
    /// whether a transfer is active (see Observation::active).
    pub(super) fn sample(&self, topics: &[Topic]) -> ApiResult<(Sample, bool)> {
        let Observation { epoch, state, rows, completions, active, files, pieces, cfg, config_error, mut daemon, totals } =
            self.observe(topics)?;
        let mut failed = BTreeSet::new();
        let mut searches = BTreeMap::new();
        let search_ids: BTreeSet<_> = topics.iter().filter_map(|t| if let Topic::Search(id) = t { Some(*id) } else { None }).collect();
        if !search_ids.is_empty() {
            let hashes: BTreeSet<_> = rows.iter().map(|r| r.hash.clone()).collect();
            for id in search_ids {
                let mut items = self.daemon.search.items(id).unwrap_or_default();
                for item in &mut items {
                    item.added = Hash::new(&item.key).ok().filter(|h| hashes.contains(h));
                }
                searches.insert(id, json!(items));
            }
        }
        let network = serde_json::to_value(self.daemon.network()).unwrap();
        let mut effective = (*cfg).clone();
        effective.idle_exit_secs = self.daemon.idle_exit_secs();
        effective.notify &= !self.daemon.no_notify;
        let settings = serde_json::to_value(ops::settings::view(&effective)).unwrap();
        let mut show_releases = BTreeMap::new();
        for id in topics.iter().filter_map(|t| if let Topic::ShowReleases(id) = t { Some(id) } else { None }) {
            match self.daemon.shows.releases((id != "inbox").then_some(id.as_str()), id == "inbox", None, &rows) {
                Ok(releases) => {
                    show_releases.insert(id.clone(), serde_json::to_value(releases).unwrap());
                }
                Err(_) => {
                    failed.insert(Topic::ShowReleases(id.clone()));
                }
            }
        }
        self.peers.retain(topics);
        let mut peers = BTreeMap::new();
        for hash in topics.iter().filter_map(|t| if let Topic::Peers(h) = t { Some(h) } else { None }).collect::<BTreeSet<_>>() {
            if rows.iter().any(|r| &r.hash == hash) {
                match self.peers.read(&self.daemon, hash) {
                    Ok(value) => {
                        peers.insert(hash.to_string(), serde_json::to_value(value).unwrap());
                    }
                    Err(_) => {
                        failed.insert(Topic::Peers(hash.clone()));
                    }
                }
            }
        }
        let mut trackers = BTreeMap::new();
        for hash in topics.iter().filter_map(|t| if let Topic::Trackers(h) = t { Some(h) } else { None }).collect::<BTreeSet<_>>() {
            if rows.iter().any(|r| &r.hash == hash) {
                match ops::trackers::read(&self.daemon, hash) {
                    Ok(value) => {
                        trackers.insert(hash.to_string(), serde_json::to_value(value).unwrap());
                    }
                    Err(_) => {
                        failed.insert(Topic::Trackers(hash.clone()));
                    }
                }
            }
        }
        // The engine's own one-second tick (or the loop while no engine runs)
        // records rate history; a sample only reads it.
        let (history, stats) = {
            let ring = self.daemon.history.lock().unwrap();
            let (session, all_time) = totals;
            (
                ring.download(60),
                if topics.contains(&Topic::Stats) {
                    json!(SampledStats { history: ring.snapshot(), session: Some(session), all_time: Some(all_time) })
                } else {
                    Value::Null
                },
            )
        };
        let shows = match topics.contains(&Topic::Shows).then(|| self.daemon.shows.list()) {
            Some(Ok(list)) => serde_json::to_value(list).unwrap(),
            Some(Err(_)) => {
                failed.insert(Topic::Shows);
                json!([])
            }
            None => json!([]),
        };
        let rss_feeds = match topics.contains(&Topic::RssFeeds).then(|| self.daemon.rss.feeds()) {
            Some(Ok(feeds)) => serde_json::to_value(feeds).unwrap(),
            Some(Err(_)) => {
                failed.insert(Topic::RssFeeds);
                json!([])
            }
            None => json!([]),
        };
        let mut rss_items = BTreeMap::new();
        let item_topics: Vec<_> = topics.iter().filter_map(|t| if let Topic::RssItems(id) = t { Some(id) } else { None }).collect();
        if !item_topics.is_empty() {
            match self.daemon.rss.snapshot() {
                Ok(snapshot) => {
                    for id in item_topics {
                        rss_items.insert(id.clone(), serde_json::to_value(snapshot.items.get(id).cloned().unwrap_or_default()).unwrap());
                    }
                }
                Err(_) => failed.extend(item_topics.into_iter().map(|id| Topic::RssItems(id.clone()))),
            }
        }
        // Report each unreadable local file on the always-subscribed summary.
        let warnings: Vec<_> = [("config", config_error), ("shows", self.daemon.shows.problem()), ("rss", self.daemon.rss.problem())]
            .into_iter()
            .filter_map(|(source, message)| message.map(|message| json!({"source":source,"message":message})))
            .collect();
        if !warnings.is_empty() {
            daemon["warnings"] = json!(warnings);
        }
        Ok((Sample {
            topics: topics.iter().cloned().collect(),
            failed,
            history,
            stats,
            peers,
            trackers,
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
            searches,
            files,
            pieces,
            network,
            settings,
            disk: self.daemon.disk.lock().unwrap().summary.clone(),
            daemon,
            memo: Memo::default(),
        }, active))
    }

    /// Hashes of the current slot's records, for detail-topic validation.
    /// Unlike a sample this takes no completions and has no side effects.
    pub(super) fn record_hashes(&self) -> ApiResult<BTreeSet<String>> {
        let slot = self.daemon.slot.read().unwrap_or_else(|e| e.into_inner());
        Ok(match &slot.slot {
            Slot::Running(app) => app.dispatcher().record_hashes().map_err(ops::engine_error)?,
            Slot::Parked(ledger) => {
                let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info).map_err(ops::engine_error)? else {
                    unreachable!()
                };
                rows.into_iter().map(|row| row.hash).collect()
            }
            Slot::Starting | Slot::Stopping => BTreeSet::new(),
        })
    }
}
