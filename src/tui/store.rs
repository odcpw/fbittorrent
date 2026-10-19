//! The TUI's confirmed API mirror. Mutation acknowledgements never edit rows.
use crate::api::{self, validation::Validate};
use anyhow::{bail, ensure, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, VecDeque};
mod pending;
mod snapshot;
mod sort;
mod search_sort;
pub(super) use search_sort::{SearchSort,search_order,legacy_search_order};
use snapshot::Snapshot;
pub(super) use sort::{TorrentSort,legacy_order};
#[cfg(test)]
mod tests;

const MAX_ITEMS: usize = 100_000;
const MAX_SNAPSHOT_BYTES: usize = 32 * api::LINE_OUT;
const MAX_EVENTS: usize = 1000; // Match the daemon event ring, including reconnect snapshots.
const MAX_LOGS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Subscription {
    pub topic: api::Topic,
    pub rev: u64,
    pub resync: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    Changed,
    Resubscribe { sub: u64, topic: api::Topic },
}

#[derive(Default)]
pub(super) struct Store {
    pub hello: Option<api::HelloResult>,
    pub connected: bool,
    pub stale: bool,
    pub pending: BTreeMap<api::MutationRef, pending::Pending>,
    pub torrents: BTreeMap<String, api::Torrent>,
    pub details: BTreeMap<api::Hash, api::TorrentDetail>,
    pub files: BTreeMap<api::Hash, BTreeMap<String, api::File>>,
    pub searches: BTreeMap<u64, BTreeMap<String, api::SearchItem>>,
    pub peers: BTreeMap<api::Hash, BTreeMap<String, api::Peer>>,
    pub rss_feeds:BTreeMap<String,api::Feed>,
    pub rss_items:BTreeMap<String,BTreeMap<String,api::FeedItem>>,
    pub shows:BTreeMap<String,api::Show>,pub show_releases:BTreeMap<String,api::Release>,
    pub transfer: Option<api::TransferInfo>,
    pub summary: Option<api::Summary>,
    pub network: Option<api::NetworkState>,
    pub settings: Option<api::Settings>,
    pub subscriptions: BTreeMap<u64, Subscription>,
    pub events: VecDeque<api::Event>,
    pub logs: VecDeque<api::LogEntry>,
    snapshots: BTreeMap<(u64, api::Topic), Snapshot>,
    caps: api::validation::NegotiatedCaps,
}
impl Store {
    pub fn hello(&mut self, hello: api::HelloResult) {
        self.stale = true;
        self.lose_pending();
        self.subscriptions.clear();
        self.snapshots.clear();
        // Subscriptions belong to the connection, even if the instance did not change.
        // Keep confirmed rows for the reconnect banner until a fresh snapshot commits.
        self.connected = hello.proto == api::PROTO;
        self.caps = api::validation::NegotiatedCaps::from_hello(&hello);
        self.hello = Some(hello);
    }
    /// Apply only authoritative read replies; mutation acknowledgements carry no row state.
    pub fn reply(&mut self, call: &api::Call, reply: api::Reply) -> Result<()> {
        if let api::Reply::Hello(hello) = reply {
            self.hello(hello);
            return Ok(());
        }
        reply.validate()?;
        self.caps.reply(&reply)?;
        match (call, reply) {
            (_,api::Reply::ShowsList(result))=>{ensure!(result.shows.len()<=MAX_ITEMS,"show list exceeds UI row budget");self.shows=result.shows.into_iter().map(|row|(row.imdb.clone(),row)).collect();},
            (api::Call::ShowsReleases {args},api::Reply::ShowsReleases(result))=>{ensure!(result.releases.len()<=MAX_ITEMS,"show releases exceed UI row budget");self.show_releases=result.releases.into_iter().map(|row|(api::release_key(args.inbox.unwrap_or(false),&row),row)).collect();},
            (_,api::Reply::RssFeeds(result))=>{
                ensure!(result.feeds.len()<=MAX_ITEMS,"feed list exceeds UI row budget");
                self.rss_feeds=result.feeds.into_iter().map(|row|(row.id.clone(),row)).collect();
            },
            (api::Call::RssItems {args},api::Reply::RssItems(result))=>{
                ensure!(result.items.len()<=MAX_ITEMS,"feed items exceed UI row budget");
                self.rss_items.clear(); // Keep only the currently selected feed's rows.
                self.rss_items.insert(args.feed.clone(),result.items.into_iter().map(|row|(row.id.clone(),row)).collect());
            },
            (_, api::Reply::TorrentsList(result)) => {
                ensure!(
                    result.torrents.len() <= MAX_ITEMS,
                    "torrent list exceeds UI row budget"
                );
                self.stale = false;
                self.fresh_rows();
                self.torrents = result
                    .torrents
                    .into_iter()
                    .map(|row| (row.hash.to_string(), row))
                    .collect();
            }
            (_, api::Reply::TorrentsGet(result)) => {
                let hash = result.torrent.torrent.hash.clone();
                self.details.clear(); // Only the currently visible detail is retained.
                self.details.insert(hash, result.torrent);
            }
            (api::Call::TorrentsFiles { args }, api::Reply::TorrentsFiles(result)) => {
                ensure!(
                    result.files.len() <= MAX_ITEMS,
                    "file list exceeds UI row budget"
                );
                self.files.clear(); // Selecting another torrent releases its old file table.
                self.files.insert(
                    args.hash.clone(),
                    result
                        .files
                        .into_iter()
                        .map(|row| (row.index.to_string(), row))
                        .collect(),
                );
            }
            (api::Call::TorrentsPeers {args},api::Reply::TorrentsPeers(result))=>{
                ensure!(result.peers.len()<=MAX_ITEMS,"peer list exceeds UI row budget");
                self.peers.clear();self.peers.insert(args.hash.clone(),result.peers.into_iter().map(|p|(p.addr.clone(),p)).collect());
            }
            (_, api::Reply::AppNetwork(result)) => self.network = Some(result.state),
            (api::Call::Unsubscribe { args }, api::Reply::Unsubscribe(_)) => {
                self.unsubscribe(args.sub)
            }
            _ => {}
        }
        Ok(())
    }
    pub fn disconnected(&mut self) {
        self.connected = false;
        self.stale = true;
        self.lose_pending();
        self.snapshots.clear();
    }
    pub fn unsubscribe(&mut self, sub: u64) {
        self.subscriptions.remove(&sub);
        self.snapshots.retain(|(number, _), _| *number != sub);
    }
    pub fn event(&mut self, event: &api::Event) -> Result<Effect> {
        event.validate()?;
        self.caps.event(event)?;
        if let Some((sub, rev)) = event.subscription_revision() {
            let topic = match event {
                api::Event::SnapshotBegin { topic, .. }
                | api::Event::SnapshotChunk { topic, .. }
                | api::Event::SnapshotEnd { topic, .. }
                | api::Event::Delta { topic, .. }
                | api::Event::State { topic, .. } => topic.clone(),
                _ => unreachable!(),
            };
            ensure!(
                self.subscriptions.contains_key(&sub)
                    || self.subscriptions.len() < api::MAX_SUBSCRIPTIONS,
                "too many subscriptions"
            );
            let subscription = self.subscriptions.entry(sub).or_insert(Subscription {
                topic: topic.clone(),
                rev: 0,
                resync: false,
            });
            if subscription.resync {
                return Ok(Effect::None);
            }
            if subscription.rev.checked_add(1) != Some(rev) {
                subscription.resync = true;
                self.snapshots.retain(|(number, _), _| *number != sub);
                return Ok(Effect::Resubscribe { sub, topic });
            }
            let result = self.apply(event);
            match result {
                Ok(effect) => {
                    let subscription = self.subscriptions.get_mut(&sub).unwrap();
                    subscription.rev = rev;
                    subscription.topic = topic;
                    Ok(effect)
                }
                Err(error) => {
                    self.subscriptions.get_mut(&sub).unwrap().resync = true;
                    self.snapshots.retain(|(number, _), _| *number != sub);
                    Err(error.context(format!("subscription {sub} needs a fresh snapshot")))
                }
            }
        } else {
            match event {
                api::Event::Bye { .. } => self.disconnected(),
                api::Event::Heartbeat { .. } => return Ok(Effect::None),
                api::Event::EngineChanged { epoch, .. } => {
                    self.stale = true;
                    if let Some(hello) = self.hello.as_mut() {
                        hello.engine_epoch = *epoch;
                    }
                    self.push_event(event.clone());
                }
                _ => self.push_event(event.clone()),
            }
            Ok(Effect::Changed)
        }
    }
    fn apply(&mut self, event: &api::Event) -> Result<Effect> {
        match event {
            api::Event::State { data, .. } => match data {
                api::StateData::Summary(data) => self.summary = Some(data.clone()),
                api::StateData::Network(data) => self.network = Some(data.clone()),
                api::StateData::Settings(data) => self.settings = Some(data.clone()),
            },
            api::Event::SnapshotBegin {
                sub, topic, count, ..
            } => {
                ensure!(
                    *count <= MAX_ITEMS as u64,
                    "snapshot exceeds {MAX_ITEMS} UI rows"
                );
                ensure!(
                    !self.snapshots.contains_key(&(*sub, topic.clone())),
                    "nested snapshot"
                );
                ensure!(
                    self.snapshots.len() < api::MAX_SUBSCRIPTIONS,
                    "too many snapshots"
                );
                self.snapshots
                    .insert((*sub, topic.clone()), Snapshot::new(*count as usize));
                return Ok(Effect::None);
            }
            api::Event::SnapshotChunk {
                sub, topic, items, ..
            } => {
                let used: usize = self.snapshots.values().map(|snapshot| snapshot.bytes).sum();
                let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(used);
                let snapshot = self
                    .snapshots
                    .get_mut(&(*sub, topic.clone()))
                    .ok_or_else(|| anyhow::anyhow!("snapshot chunk without begin"))?;
                snapshot.append(topic, items, remaining)?;
                return Ok(Effect::None);
            }
            api::Event::SnapshotEnd { sub, topic, .. } => {
                let snapshot = self
                    .snapshots
                    .remove(&(*sub, topic.clone()))
                    .ok_or_else(|| anyhow::anyhow!("snapshot end without begin"))?;
                self.commit(topic, snapshot)?;
            }
            api::Event::Delta {
                topic,
                changed,
                added,
                removed,
                ..
            } => match topic {
                api::Topic::Shows=>patch(&mut self.shows,changed,added,removed,|item|match item{api::Item::Show(row)=>Some((row.imdb.clone(),row.clone())),_=>None})?,
                api::Topic::ShowReleases(id)=>patch(&mut self.show_releases,changed,added,removed,|item|match item{api::Item::Release(row)=>Some((api::release_key(id=="inbox",row),row.clone())),_=>None})?,
                api::Topic::RssFeeds=>patch(&mut self.rss_feeds,changed,added,removed,|item|match item{api::Item::Feed(row)=>Some((row.id.clone(),row.clone())),_=>None})?,
                api::Topic::RssItems(id)=>patch(self.rss_items.entry(id.clone()).or_default(),changed,added,removed,|item|match item{api::Item::FeedItem(row)=>Some((row.id.clone(),row.clone())),_=>None})?,
                api::Topic::Torrents => patch(
                    &mut self.torrents,
                    changed,
                    added,
                    removed,
                    |item| match item {
                        api::Item::Torrent(row) => Some((row.hash.to_string(), row.clone())),
                        _ => None,
                    },
                )?,
                api::Topic::Peers(hash) => patch(self.peers.entry(hash.clone()).or_default(),changed,added,removed,|item|match item{api::Item::Peer(p)=>Some((p.addr.clone(),p.clone())),_=>None})?,
                api::Topic::Files(hash) => patch(
                    self.files.entry(hash.clone()).or_default(),
                    changed,
                    added,
                    removed,
                    |item| match item {
                        api::Item::File(row) => Some((row.index.to_string(), row.clone())),
                        _ => None,
                    },
                )?,
                api::Topic::Search(id) => patch(
                    self.searches.entry(*id).or_default(), changed, added, removed,
                    |item| match item { api::Item::Search(row) => Some((row.key.clone(),row.clone())), _ => None },
                )?,
                api::Topic::Events => {
                    for item in added.iter().flatten() {
                        let api::Item::Event(event) = item else {
                            bail!("non-event in events delta")
                        };
                        self.push_event(*event.clone());
                    }
                }
                api::Topic::Log => {
                    for item in added.iter().flatten() {
                        let api::Item::Log(entry) = item else {
                            bail!("non-log in log delta")
                        };
                        self.push_log(entry.clone());
                    }
                }
                _ => bail!("delta on a singleton topic"),
            },
            _ => return Ok(Effect::None),
        }
        Ok(Effect::Changed)
    }
    fn push_log(&mut self, entry: api::LogEntry) {
        if self.logs.back().is_none_or(|last| last.seq < entry.seq) {
            if self.logs.len() == MAX_LOGS { self.logs.pop_front(); }
            self.logs.push_back(entry);
        }
    }
    fn push_event(&mut self, event: api::Event) {
        self.settle(&event);
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
    fn commit(&mut self, topic: &api::Topic, snapshot: Snapshot) -> Result<()> {
        ensure!(snapshot.len() == snapshot.count, "incomplete snapshot");
        match topic {
            api::Topic::Shows=>self.shows=snapshot.shows,
            api::Topic::ShowReleases(_)=>self.show_releases=snapshot.releases,
            api::Topic::RssFeeds=>self.rss_feeds=snapshot.feeds,
            api::Topic::RssItems(id)=>{self.rss_items.clear();self.rss_items.insert(id.clone(),snapshot.feed_items);},
            api::Topic::Torrents => {
                self.torrents = snapshot.torrents;
                self.stale = false;
                self.fresh_rows();
            }
            api::Topic::Peers(hash)=>{self.peers.clear();self.peers.insert(hash.clone(),snapshot.peers);},
            api::Topic::Files(hash) => {
                self.files.clear();
                self.files.insert(hash.clone(), snapshot.files);
            }
            api::Topic::Search(id) => {
                self.searches.clear(); // The TUI retains only its current search.
                self.searches.insert(*id, snapshot.searches);
            }
            api::Topic::Events => {
                for event in snapshot.events {
                    self.push_event(event);
                }
            }
            api::Topic::Log => {
                self.logs = snapshot.logs.into_values().collect();
            }
            _ => bail!("snapshot on a singleton topic"),
        }
        Ok(())
    }
}

/// Validate only changed rows before committing a delta; do not clone the table.
fn patch<T: Serialize + DeserializeOwned + Validate>(
    table: &mut BTreeMap<String, T>,
    changed: &Option<BTreeMap<String, BTreeMap<String, Value>>>,
    added: &Option<Vec<api::Item>>,
    removed: &Option<Vec<Value>>,
    extract: impl Fn(&api::Item) -> Option<(String, T)>,
) -> Result<()> {
    let mut updates = Vec::new();
    for (key, fields) in changed.iter().flatten() {
        let old = table
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("delta changes unknown row {key}"))?;
        let mut value = serde_json::to_value(old)?;
        merge(value.as_object_mut().unwrap(), fields);
        let row: T = serde_json::from_value(value)?;
        row.validate()?;
        // A partial update may never change the identity that indexes the row.
        ensure!(
            !fields.contains_key("hash") && !fields.contains_key("index"),
            "delta changes a row identity"
        );
        updates.push((key.clone(), row));
    }
    for item in added.iter().flatten() {
        let (key, row) =
            extract(item).ok_or_else(|| anyhow::anyhow!("item does not match topic"))?;
        row.validate()?;
        updates.push((key, row));
    }
    let removals: Vec<_> = removed
        .iter()
        .flatten()
        .map(|key| {
            key.as_str()
                .map(str::to_owned)
                .or_else(|| key.as_u64().map(|key| key.to_string()))
                .ok_or_else(|| anyhow::anyhow!("invalid removal key"))
        })
        .collect::<Result<_>>()?;
    let new_keys = updates
        .iter()
        .filter(|(key, _)| !table.contains_key(key))
        .count();
    ensure!(
        table.len().saturating_add(new_keys) <= MAX_ITEMS,
        "table exceeds {MAX_ITEMS} UI rows"
    );
    for key in removals {
        table.remove(&key);
    }
    for (key, row) in updates {
        table.insert(key, row);
    }
    Ok(())
}
fn merge(object: &mut Map<String, Value>, fields: &BTreeMap<String, Value>) {
    for (key, value) in fields {
        if value.is_null() {
            object.remove(key);
        } else {
            object.insert(key.clone(), value.clone());
        }
    }
}
