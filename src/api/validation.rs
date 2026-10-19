//! Shared API bounds and cross-field rules. Serde handles wire shapes; these
//! checks also protect typed producers and contextual reply/fixture decoding.
use super::*;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const MIN_UPLOAD_BPS: u64 = 16_384;
pub const MIN_INTERVAL_MS: u64 = 250;
pub const MAX_INTERVAL_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation(pub String);
impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Violation {}
pub type Checked = Result<(), Violation>;
pub trait Validate {
    fn validate(&self) -> Checked;
}
fn require(ok: bool, path: &str, expected: &str) -> Checked {
    if ok {
        Ok(())
    } else {
        Err(Violation(format!("{path}: {expected}")))
    }
}
pub fn text(value: &str, max: usize, path: &str) -> Checked {
    require(
        value.chars().count() <= max,
        path,
        &format!("at most {max} characters"),
    )
}
pub fn count(value: usize, max: usize, path: &str) -> Checked {
    require(
        (1..=max).contains(&value),
        path,
        &format!("must contain 1..{max} items"),
    )
}
pub fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn mutation_ref(value: &str) -> Checked {
    require(
        value.is_ascii() && value.len() <= 64,
        "ref",
        "at most 64 ASCII characters",
    )
}
pub fn progress(value: f64) -> Checked {
    require(
        value.is_finite() && (0.0..=1.0).contains(&value),
        "progress",
        "must be finite and in 0..=1",
    )
}
pub fn upload_bps(value: u64) -> Checked {
    require(
        value >= MIN_UPLOAD_BPS,
        "bps",
        "numeric upload limit must be at least 16384; null means unlimited",
    )
}
pub fn frame<T: Serialize>(value: &T, cap: usize) -> Checked {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| Violation(e.to_string()))?
        .len();
    require(
        bytes < cap,
        "frame",
        "serialized frame plus newline exceeds the byte cap",
    )
}
pub fn deserialize_topics<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Topic>, D::Error> {
    let topics = Vec::<Topic>::deserialize(d)?;
    count(topics.len(), MAX_SUBSCRIPTIONS, "topics").map_err(serde::de::Error::custom)?;
    Ok(topics)
}
pub fn deserialize_upload_update<'de, D: Deserializer<'de>>(d: D) -> Result<Update<u64>, D::Error> {
    let update = Update::<u64>::deserialize(d)?;
    if let Update::Set(bps) = update {
        upload_bps(bps).map_err(serde::de::Error::custom)?;
    }
    Ok(update)
}
impl SubscribeArgs {
    /// Preserve the supplied wire value for round trips; use this normalized
    /// interval for sampling. The caller supplies the topic's default (e.g. 5000
    /// for an idle bar summary, 1000 for an open view).
    pub fn effective_interval_ms(&self, default: u64) -> u64 {
        self.interval_ms
            .unwrap_or(default)
            .clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS)
    }
}
impl Validate for SubscribeArgs {
    fn validate(&self) -> Checked {
        count(self.topics.len(), MAX_SUBSCRIPTIONS, "topics")?;
        require(
            self.since_seq.is_none() || self.topics.contains(&Topic::Events),
            "since_seq",
            "requires the events topic",
        )
    }
}
impl Validate for UploadLimitArgs {
    fn validate(&self) -> Checked {
        count(self.hashes.as_slice().len(), MAX_HASHES, "hashes")?;
        if let Update::Set(bps) = self.bps {
            upload_bps(bps)?;
        }
        Ok(())
    }
}
impl Validate for Request {
    fn validate(&self) -> Checked {
        match &self.call {
            Call::AppLog { args: Some(args) } => {
                require(args.limit.is_none_or(|v| v <= 1000), "limit", "at most 1000")?;
                if let Some(source) = &args.source { text(source, 64, "source")?; }
                Ok(())
            },
            Call::TransferSetLimits { args } => {
                for (key, value) in [("download_bps", &args.download_bps), ("upload_bps", &args.upload_bps), ("alt_download_bps", &args.alt_download_bps), ("alt_upload_bps", &args.alt_upload_bps)] {
                    if let Update::Set(bps) = value { require(*bps == 0 || *bps >= 16384, key, "zero, null or at least 16384 bytes/s")?; }
                }
                Ok(())
            },
            Call::Subscribe { args } => args.validate(),
            Call::TorrentsSetUploadLimit { args } => args.validate(),
            Call::Hello { args } => text(args.client.name.as_str(), 64, "client.name"),
            _ => super::search::validate_call(&self.call),
        }
    }
}
impl Validate for Error {
    fn validate(&self) -> Checked {
        text(&self.message, 200, "error.message")
    }
}
fn torrent(value: &Torrent, list_name: bool) -> Checked {
    require(
        value.state != TorrentState::Checking || !value.checked,
        "torrent.checked",
        "must be false while state is checking",
    )?;
    if list_name {
        text(&value.name, 1024, "torrent.name")?;
    }
    if let Some(p) = value.progress {
        progress(p)?;
    }
    if value
        .error
        .as_deref()
        .is_some_and(|message| message.ends_with(" restore failed: missing files"))
    {
        require(
            value.reason.as_deref() == Some("missing_files"),
            "torrent.reason",
            "missing-files restore errors require missing_files",
        )?;
    }
    Ok(())
}
impl Validate for Torrent {
    fn validate(&self) -> Checked {
        torrent(self, true)
    }
}
impl Validate for TorrentDetail {
    fn validate(&self) -> Checked {
        torrent(&self.torrent, false)
    }
}
impl Validate for SearchItem {
    fn validate(&self) -> Checked { SearchItem::validate(self) }
}
impl Validate for File {
    fn validate(&self) -> Checked {
        progress(self.progress)
    }
}
impl Validate for Summary {
    fn validate(&self) -> Checked {
        require(self.top.len() <= 5, "summary.top", "at most 5 entries")?;
        for top in &self.top {
            text(&top.name, 1024, "summary.top.name")?;
            if let Some(p) = top.progress {
                progress(p)?;
            }
        }
        Ok(())
    }
}
impl Validate for Reply {
    fn validate(&self) -> Checked {
        match self {
            Self::AppLog(result) => {
                require(result.entries.len() <= 1000, "entries", "at most 1000")?;
                for entry in &result.entries { entry.validate()?; }
                Ok(())
            }
            Self::TorrentsTrackers(result) => {
                let mut previous=None;
                for row in &result.trackers {
                    text(&row.url,65536,"trackers.url")?;
                    require(previous.is_none_or(|url:&str|url<row.url.as_str()),"trackers","URLs must be sorted and unique")?;
                    previous=Some(row.url.as_str());
                    if let Some(message)=&row.message {text(message,200,"trackers.message")?;}
                }
                Ok(())
            }
            Self::TorrentsList(result) => {
                for value in &result.torrents {
                    value.validate()?;
                }
                Ok(())
            }
            Self::TorrentsGet(result) => result.torrent.validate(),
            Self::TorrentsFiles(result) => {
                for value in &result.files {
                    value.validate()?;
                }
                Ok(())
            }
            Self::SearchStart(result) if result.search_id == 0 => Err(Violation("search_id: must be positive".into())),
            Self::SearchBrowse(result) => {
                require(result.items.len() <= 5000, "items", "search result cap is 5000")?;
                for value in &result.items { value.validate()?; } Ok(())
            }
            Self::Subscribe(result) => count(result.topics.len(), MAX_SUBSCRIPTIONS, "topics"),
            _ => Ok(()), // Pending refs are required by their serde variants.
        }
    }
}

/// Validate the fixture transcript's repeated list/get values and file totals
/// per hash, checking either arrival order. Omitted values remain unknown.
#[derive(Debug, Default)]
pub struct TorrentFileSizes {
    by_hash: BTreeMap<Hash, FileSizeObservations>,
}
#[derive(Debug, Default)]
struct FileSizeObservations {
    torrent_sizes: BTreeSet<u64>,
    completed_bytes: BTreeSet<u64>,
    file_totals: BTreeSet<u64>,
}
impl TorrentFileSizes {
    /// A confirmed removal ends this torrent instance's measured completion.
    /// The info hash's static geometry still applies after re-admission.
    #[cfg(test)]
    pub(crate) fn removed(&mut self, hash: &Hash) {
        if let Some(observed) = self.by_hash.get_mut(hash) {
            observed.completed_bytes.clear();
        }
    }

    fn torrent(&mut self, value: &Torrent, path: &str) -> Checked {
        let observed = self.by_hash.entry(value.hash.clone()).or_default();
        if let Some(size) = value.size {
            require(
                observed
                    .torrent_sizes
                    .iter()
                    .all(|earlier| *earlier == size),
                &format!("{path}/size"),
                "must agree with earlier list/get replies for the same hash",
            )?;
            require(
                observed.file_totals.iter().all(|total| *total == size),
                &format!("{path}/size"),
                "must equal the sum of file lengths for the same hash",
            )?;
            observed.torrent_sizes.insert(size);
        }
        if let Some(completed) = value.completed_bytes {
            require(
                observed
                    .completed_bytes
                    .iter()
                    .all(|earlier| *earlier == completed),
                &format!("{path}/completed_bytes"),
                "must agree with earlier list/get replies for the same hash",
            )?;
            observed.completed_bytes.insert(completed);
        }
        Ok(())
    }

    pub fn reply(&mut self, call: &Call, reply: &Reply) -> Checked {
        match (call, reply) {
            (_, Reply::TorrentsList(result)) => {
                for (index, value) in result.torrents.iter().enumerate() {
                    self.torrent(value, &format!("/frame/result/torrents/{index}"))?;
                }
                Ok(())
            }
            (_, Reply::TorrentsGet(result)) => {
                self.torrent(&result.torrent.torrent, "/frame/result/torrent")
            }
            (Call::TorrentsFiles { args }, Reply::TorrentsFiles(result)) => {
                let mut total = 0u64;
                for (index, file) in result.files.iter().enumerate() {
                    total = total.checked_add(file.size).ok_or_else(|| {
                        Violation(format!(
                            "/frame/result/files/{index}/size: sum of file lengths exceeds u64"
                        ))
                    })?;
                }
                let observed = self.by_hash.entry(args.hash.clone()).or_default();
                require(
                    observed.torrent_sizes.iter().all(|size| *size == total),
                    "/frame/result/files",
                    "sum of file lengths must equal the torrent size for the same hash",
                )?;
                observed.file_totals.insert(total);
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Output rules that require the connection's hello capabilities, in addition
/// to the context-free shape/bounds checks above. Playable fields and events
/// require watch in a successful hello.
#[derive(Debug, Default)]
pub struct NegotiatedCaps {
    watch: bool,
    log: bool,
}
impl NegotiatedCaps {
    pub fn from_hello(hello: &HelloResult) -> Self {
        Self {
            watch: hello.caps.iter().any(|cap| cap == "watch"),
            log: hello.caps.iter().any(|cap| cap == "log"),
        }
    }
    fn playable(&self, present: bool, path: &str) -> Checked {
        require(
            !present || self.watch,
            path,
            "requires the watch capability",
        )
    }
    pub fn torrent(&self, value: &Torrent) -> Checked {
        self.playable(value.playable.is_some(), "torrent.playable")
    }
    pub fn summary(&self, value: &Summary) -> Checked {
        for (index, top) in value.top.iter().enumerate() {
            self.playable(
                top.playable.is_some(),
                &format!("summary.top[{index}].playable"),
            )?;
        }
        Ok(())
    }
    pub fn reply(&self, value: &Reply) -> Checked {
        match value {
            Reply::AppLog(_) | Reply::AppStats(_) => require(self.log, "result", "requires the log capability"),
            Reply::TorrentsList(result) => {
                for torrent in &result.torrents {
                    self.torrent(torrent)?;
                }
                Ok(())
            }
            Reply::TorrentsGet(result) => self.torrent(&result.torrent.torrent),
            _ => Ok(()),
        }
    }
    fn items(&self, values: &[Item], field: &str) -> Checked {
        for (index, value) in values.iter().enumerate() {
            let result = match value {
                Item::Torrent(torrent) => self.torrent(torrent),
                Item::Event(event) => self.event(event),
                Item::File(_) | Item::Search(_) | Item::Log(_) | Item::Feed(_) | Item::FeedItem(_) | Item::Show(_) | Item::Release(_) => Ok(()),
            };
            result.map_err(|error| Violation(format!("{field}[{index}].{error}")))?;
        }
        Ok(())
    }
    pub fn event(&self, value: &Event) -> Checked {
        match value {
            Event::SnapshotBegin { topic: Topic::Log, .. } | Event::SnapshotChunk { topic: Topic::Log, .. }
            | Event::SnapshotEnd { topic: Topic::Log, .. } | Event::Delta { topic: Topic::Log, .. } => {
                require(self.log, "topic", "requires the log capability")?;
            }
            _ => {}
        }
        match value {
            Event::TorrentPlayable { .. } => require(
                self.watch,
                "ev",
                "torrent.playable requires the watch capability",
            ),
            Event::State {
                data: StateData::Summary(summary),
                ..
            } => self.summary(summary),
            Event::SnapshotChunk { items, .. } => self.items(items, "items"),
            Event::Delta {
                topic,
                added,
                changed,
                ..
            } => {
                if let Some(items) = added {
                    self.items(items, "added")?;
                }
                if *topic == Topic::Torrents {
                    if let Some(changed) = changed {
                        for (hash, fields) in changed {
                            // Null removes an optional field; it does not
                            // advertise playable data without the capability.
                            self.playable(
                                fields.get("playable").is_some_and(|v| !v.is_null()),
                                &format!("changed.{hash}.playable"),
                            )?;
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn items(topic: &Topic, values: &[Item]) -> Checked {
    for value in values {
        match (topic, value) {
            (Topic::Shows,Item::Show(row))=>row.validate()?,(Topic::ShowReleases(_),Item::Release(row))=>row.validate()?,
            (Topic::RssFeeds, Item::Feed(_)) | (Topic::RssItems(_), Item::FeedItem(_))=>{},
            (Topic::Log, Item::Log(entry)) => entry.validate()?,
            (Topic::Torrents, Item::Torrent(t)) => t.validate()?,
            (Topic::Files(_), Item::File(f)) => f.validate()?,
            (Topic::Search(_), Item::Search(s)) => s.validate()?,
            (Topic::Events, Item::Event(e))
                if e.subscription_revision().is_none()
                    && !matches!(
                        e.as_ref(),
                        Event::Heartbeat { .. } | Event::Bye { .. } | Event::Error { .. }
                    ) =>
            {
                e.validate()?
            }
            _ => return Err(Violation("items: payload does not match topic".into())),
        }
    }
    Ok(())
}
fn delta_value(key: &str, value: &Value) -> Checked {
    if value.is_null() {
        return Ok(());
    } // Explicit deletion of an optional field.
    match key {
        "progress" => progress(
            value
                .as_f64()
                .ok_or_else(|| Violation("progress: expected a number".into()))?,
        ),
        "name" => text(
            value
                .as_str()
                .ok_or_else(|| Violation("name: expected a string".into()))?,
            1024,
            "name",
        ),
        "hash" => require(
            value.as_str().is_some_and(|h| lower_hex(h, 40)),
            "hash",
            "40 lowercase hex characters",
        ),
        _ => Ok(()), // Optional future output fields remain additive.
    }
}
impl Validate for Event {
    fn validate(&self) -> Checked {
        if let Some((_, rev)) = self.subscription_revision() {
            require(rev > 0, "rev", "starts at 1")?;
        }
        match self {
            Self::SnapshotBegin {
                topic,
                reason,
                oldest_seq,
                ..
            } => require(
                oldest_seq.is_none()
                    || (*reason == SnapshotReason::Resync && *topic == Topic::Events),
                "oldest_seq",
                "only valid for an events resync",
            ),
            Self::SnapshotChunk {
                topic,
                items: values,
                ..
            } => {
                require(
                    values.len() <= MAX_CHUNK_ITEMS,
                    "items",
                    "snapshot chunk exceeds 256 items",
                )?;
                items(topic, values)
            }
            Self::Delta {
                topic,
                changed,
                added,
                removed,
                ..
            } => {
                if let Some(values) = added {
                    items(topic, values)?;
                }
                if let Some(keys) = removed {
                    for key in keys {
                        require(
                            match topic {
                                Topic::Torrents => key.as_str().is_some_and(|h| lower_hex(h, 40)),
                                Topic::RssFeeds | Topic::RssItems(_) => key.is_string(),
                                Topic::Shows | Topic::ShowReleases(_) => key.is_string(),
                                Topic::Files(_) | Topic::Events | Topic::Log => key.as_u64().is_some(),
                                Topic::Search(_) => key.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 8192),
                                _ => false,
                            },
                            "removed",
                            "key does not match topic",
                        )?;
                    }
                }
                if let Some(changed) = changed {
                    require(
                        !matches!(topic, Topic::Events | Topic::Log) || changed.is_empty(),
                        "changed",
                        "events topic is append-only",
                    )?;
                    for (key, fields) in changed {
                        require(
                            match topic {
                                Topic::Torrents => lower_hex(key, 40),
                                Topic::RssFeeds | Topic::RssItems(_) => !key.is_empty(),
                                Topic::Shows | Topic::ShowReleases(_) => !key.is_empty(),
                                Topic::Files(_) => key.parse::<u64>().is_ok(),
                                Topic::Search(_) => !key.is_empty() && key.len() <= 8192,
                                _ => false,
                            },
                            "changed",
                            "key does not match topic",
                        )?;
                        for (field, value) in fields {
                            delta_value(field, value)?;
                        }
                    }
                }
                Ok(())
            }
            Self::State { topic, data, .. } => match (topic, data) {
                (Topic::Summary, StateData::Summary(value)) => value.validate(),
                (Topic::Network, StateData::Network(_))
                | (Topic::Settings, StateData::Settings(_)) => Ok(()),
                _ => Err(Violation("state: data does not match topic".into())),
            },
            Self::MutationResult {
                op,
                outcome,
                error,
                result,
                ..
            } => {
                require(
                    !(*outcome == MutationOutcome::Done && error.is_some()),
                    "error",
                    "done mutation cannot carry an error",
                )?;
                require(
                    !(*outcome == MutationOutcome::Failed && error.is_none()),
                    "error",
                    "failed mutation requires an error",
                )?;
                if let Some(error) = error {
                    error.validate()?;
                }
                if let Some(result) = result {
                    if ops::TABLE.iter().any(|row| row.name == op) {
                        Reply::decode(op, Value::Object(result.clone()))
                            .map_err(|e| Violation(e.to_string()))?;
                    }
                }
                Ok(())
            }
            Self::Error { error } => error.validate(),
            _ => Ok(()),
        }
    }
}

/// Per-subscription revisions share a counter across topics. Rejected frames
/// leave the last accepted revision intact, so clients can recover from a gap.
#[derive(Debug, Default)]
pub struct Revisions(BTreeMap<u64, u64>);
impl Revisions {
    pub fn last(&self, sub: u64) -> u64 {
        self.0.get(&sub).copied().unwrap_or(0)
    }
    pub fn observe(&mut self, sub: u64, rev: u64) -> Checked {
        require(
            self.last(sub).checked_add(1) == Some(rev),
            "rev",
            "must increase by exactly one per subscription",
        )?;
        self.0.insert(sub, rev);
        Ok(())
    }
    pub fn unsubscribe(&mut self, sub: u64) {
        self.0.remove(&sub);
    }
}

#[cfg(test)]
mod tests;

impl Validate for LogEntry {
    fn validate(&self) -> Checked {
        require(self.seq > 0, "seq", "starts at one")?;
        text(&self.source, 64, "source")?;
        text(&self.message, 8192, "message")
    }
}
