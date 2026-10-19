use super::*;
use std::io;

pub(super) struct Snapshot {
    pub count: usize,
    pub bytes: usize,
    pub torrents: BTreeMap<String, api::Torrent>,
    pub files: BTreeMap<String, api::File>,
    pub searches: BTreeMap<String, api::SearchItem>,
    pub events: Vec<api::Event>,
}
impl Snapshot {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            bytes: 0,
            torrents: BTreeMap::new(),
            files: BTreeMap::new(),
            searches: BTreeMap::new(),
            events: Vec::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.torrents.len() + self.files.len() + self.searches.len() + self.events.len()
    }
    pub fn append(
        &mut self,
        topic: &api::Topic,
        items: &[api::Item],
        remaining: usize,
    ) -> Result<()> {
        ensure!(
            self.len().saturating_add(items.len()) <= self.count,
            "snapshot exceeds declared count"
        );
        let bytes = encoded_size(items, remaining)?;
        // Validate the whole chunk before mutating its staging table.
        let mut keys = std::collections::BTreeSet::new();
        for item in items {
            let unique = match (topic, item) {
                (api::Topic::Torrents, api::Item::Torrent(row)) => {
                    keys.insert(row.hash.to_string())
                        && !self.torrents.contains_key(row.hash.as_str())
                }
                (api::Topic::Files(_), api::Item::File(row)) => {
                    keys.insert(row.index.to_string())
                        && !self.files.contains_key(&row.index.to_string())
                }
                (api::Topic::Search(_), api::Item::Search(row)) => {
                    keys.insert(row.key.clone()) && !self.searches.contains_key(&row.key)
                }
                (api::Topic::Events, api::Item::Event(_)) => {
                    self.events.len() + items.len() <= MAX_EVENTS
                }
                _ => bail!("snapshot item does not match topic"),
            };
            ensure!(
                unique,
                "duplicate snapshot identity or oversized event history"
            );
        }
        for item in items {
            match item {
                api::Item::Torrent(row) => {
                    self.torrents.insert(row.hash.to_string(), row.clone());
                }
                api::Item::File(row) => {
                    self.files.insert(row.index.to_string(), row.clone());
                }
                api::Item::Event(event) => self.events.push(*event.clone()),
                api::Item::Search(row) => { self.searches.insert(row.key.clone(),row.clone()); },
            }
        }
        self.bytes += bytes;
        Ok(())
    }
}
/// Count serialized bytes without allocating a second copy of a snapshot chunk.
fn encoded_size<T: Serialize + ?Sized>(value: &T, limit: usize) -> Result<usize> {
    struct Count {
        used: usize,
        limit: usize,
    }
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let Some(next) = self
                .used
                .checked_add(bytes.len())
                .filter(|n| *n <= self.limit)
            else {
                return Err(io::Error::other("snapshot exceeds 32 MiB UI memory budget"));
            };
            self.used = next;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { used: 0, limit };
    serde_json::to_writer(&mut count, value)?;
    Ok(count.used)
}
