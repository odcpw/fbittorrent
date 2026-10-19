//! Bounded subscription emission through the connection outbox.

use super::*;

impl EventSink {
    pub fn id(&self) -> u64 {
        self.0.number
    }
    fn checked(&self, event: Event) -> ApiResult<()> {
        event
            .validate()
            .map_err(|_| error(ErrorCode::Internal, "invalid server event", None))?;
        { self.0.client.lock().unwrap().caps.event(&event) }.map_err(|_| {
            error(
                ErrorCode::Internal,
                "event requires an unnegotiated capability",
                None,
            )
        })?;
        self.0.send(serde_json::to_value(event).unwrap());
        Ok(())
    }
    pub fn send(&self, event: Event) -> ApiResult<()> {
        let _emission = self.0.emission.lock().unwrap();
        self.checked(event)
    }
    pub fn snapshot(
        &self,
        sub: u64,
        topic: Topic,
        previous: u64,
        items: Vec<Item>,
        reason: SnapshotReason,
    ) -> ApiResult<u64> {
        let _emission = self.0.emission.lock().unwrap();
        let count = items.len() as u64;
        let mut chunks = Vec::new();
        let mut chunk = Vec::new();
        let mut bytes = 0;
        // Topic and numeric envelope overhead is below 1024 bytes. Both the
        // item-count and byte limits apply; UTF-8/escaping can exceed estimates
        // based solely on the 1024-character torrent-name limit.
        for item in items {
            let size = serde_json::to_vec(&item).unwrap().len() + 1;
            if size > LINE_OUT - 1024 {
                return Err(error(
                    ErrorCode::InvalidMessage,
                    "one subscription item exceeds the frame cap",
                    None,
                ));
            }
            if chunk.len() == MAX_CHUNK_ITEMS || bytes + size > LINE_OUT - 1024 {
                chunks.push(std::mem::take(&mut chunk));
                bytes = 0;
            }
            bytes += size;
            chunk.push(item);
        }
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        let final_rev = previous
            .checked_add(chunks.len() as u64 + 2)
            .ok_or_else(|| error(ErrorCode::Internal, "subscription revision exhausted", None))?;
        // Validate every payload before publishing the begin marker.
        for items in &chunks {
            let event = Event::SnapshotChunk {
                sub,
                topic: topic.clone(),
                rev: 1,
                items: items.clone(),
            };
            event.validate().map_err(|_| {
                error(
                    ErrorCode::Internal,
                    "snapshot payload does not match topic",
                    None,
                )
            })?;
            self.0
                .client
                .lock()
                .unwrap()
                .caps
                .event(&event)
                .map_err(|_| {
                    error(
                        ErrorCode::Internal,
                        "snapshot requires an unnegotiated capability",
                        None,
                    )
                })?;
        }
        let mut rev = previous + 1;
        self.checked(Event::SnapshotBegin {
            sub,
            topic: topic.clone(),
            rev,
            count,
            reason,
            oldest_seq: None,
        })?;
        for items in chunks {
            rev += 1;
            self.checked(Event::SnapshotChunk {
                sub,
                topic: topic.clone(),
                rev,
                items,
            })?;
        }
        self.checked(Event::SnapshotEnd {
            sub,
            topic,
            rev: final_rev,
        })?;
        Ok(final_rev)
    }
    pub fn added(&self, sub: u64, topic: Topic, previous: u64, items: Vec<Item>) -> ApiResult<u64> {
        let _emission = self.0.emission.lock().unwrap();
        let mut rev = previous;
        let mut chunk = Vec::new();
        let mut bytes = 0;
        for item in items {
            let size = serde_json::to_vec(&item).unwrap().len() + 1;
            if size > LINE_OUT - 1024 {
                return Err(error(
                    ErrorCode::InvalidMessage,
                    "one delta item exceeds the frame cap",
                    None,
                ));
            }
            if chunk.len() == MAX_CHUNK_ITEMS || bytes + size > LINE_OUT - 1024 {
                rev = rev.checked_add(1).ok_or_else(|| {
                    error(ErrorCode::Internal, "subscription revision exhausted", None)
                })?;
                self.checked(Event::Delta {
                    sub,
                    topic: topic.clone(),
                    rev,
                    changed: None,
                    added: Some(std::mem::take(&mut chunk)),
                    removed: None,
                })?;
                bytes = 0;
            }
            bytes += size;
            chunk.push(item);
        }
        if !chunk.is_empty() {
            rev = rev.checked_add(1).ok_or_else(|| {
                error(ErrorCode::Internal, "subscription revision exhausted", None)
            })?;
            self.checked(Event::Delta {
                sub,
                topic,
                rev,
                changed: None,
                added: Some(chunk),
                removed: None,
            })?;
        }
        Ok(rev)
    }
}
