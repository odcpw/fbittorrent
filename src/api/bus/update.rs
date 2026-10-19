//! Per-subscription delivery of one shared sample: snapshots, structural
//! deltas and coalesced rate updates with consecutive revisions.
use super::*;

impl Subscription {
    pub(super) fn update(&mut self, sample: &Sample, events: &[Value], oldest: u64) -> ApiResult<()> {
        // A slot without a usable ledger is transitional, not an empty saved
        // history. Keep detail subscriptions until the replacement is ready.
        // A new stats subscription can arrive while the worker samples an
        // older topic set. Wait for its first actual sample, rather than emit
        // null and tear down the shared UI connection.
        if matches!(sample.state.as_str(), "starting" | "stopping")
            || (sample.stats.is_null() && self.topics.iter().any(|t|t.topic==Topic::Stats)) {
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
                Topic::Torrents | Topic::Peers(_) | Topic::Trackers(_) | Topic::Files(_) | Topic::Search(_) | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_)
            )
        });
        for index in order {
            let t = &mut self.topics[index];
            if restart && matches!(t.topic,Topic::Pieces(_)) {t.sent=None;}
            // subscribe/activate can finish while this sample is in flight.
            // An uncollected topic is unknown, not an empty collection. In
            // particular, an empty Files snapshot would erase a newer RPC read
            // and its keyboard selection. Wait for the next actual sample.
            if !sample.topics.contains(&t.topic) {
                let now=Instant::now();
                if t.sent.is_none() && t.due<=now {t.due=now+Duration::from_millis(100);}
                continue;
            }
            // A broken local store (shows.json, rss.json) or a failed detail
            // read is local to its topic: keep the last delivered value, or the
            // pending first snapshot, and retry at most once a second.
            if sample.failed.contains(&t.topic) {
                let now=Instant::now();
                if t.due<=now {t.due=now+Duration::from_secs(1);}
                continue;
            }
            let mut value = if t.topic == Topic::Events {
                Arc::new(json!(events
                    .iter()
                    .filter(|e| self.since.is_none_or(|s| e["seq"].as_u64().unwrap() > s))
                    .collect::<Vec<_>>()))
            } else {
                sample.value(&t.topic)
            };
            if matches!(t.topic,Topic::Pieces(_)) && value.is_null() {if t.due<=Instant::now() {t.due=Instant::now()+t.interval;}continue;}
            let keyed = matches!(t.topic, Topic::Torrents | Topic::Peers(_) | Topic::Trackers(_) | Topic::Files(_) | Topic::Search(_) | Topic::Events | Topic::Log | Topic::RssFeeds | Topic::RssItems(_) | Topic::Shows | Topic::ShowReleases(_));
            let reset =
                restart && matches!(t.topic, Topic::Summary | Topic::Torrents | Topic::Peers(_) | Topic::Trackers(_) | Topic::Files(_));
            if t.sent.is_none() || reset {
                if keyed {
                    let resync = t.topic == Topic::Events
                        && self.since.is_some_and(|s| s.saturating_add(1) < oldest);
                    if resync {
                        value = Arc::new(json!(events));
                    }
                    let items = value
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(serde::Deserialize::deserialize)
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
                        .send(state_event(self.sub, &t.topic, self.rev, &value)?)?;
                }
                t.sent = Some(value);
                t.due = Instant::now() + t.interval;
            } else {
                let old = t.sent.as_ref().unwrap();
                let due = Instant::now() >= t.due;
                let next = if due {
                    value
                } else {
                    project(&t.topic, old, value)
                };
                if *next != **old {
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
                            .send(state_event(self.sub, &t.topic, self.rev, &next)?)?;
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
                |t| !matches!(&t.topic,Topic::Files(h) | Topic::Peers(h) | Topic::Trackers(h) if !sample.rows.iter().any(|r|&r.hash==h)),
            );
        }
        self.topics.retain(|t|!matches!(&t.topic,Topic::Pieces(h) if !sample.rows.iter().any(|r|&r.hash==h)));
        self.epoch = Some(engine);
        Ok(())
    }
}
