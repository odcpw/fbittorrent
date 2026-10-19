use super::*;
/// Subscription producers use the same queue, capability checks and frame cap
/// as replies. One emission guard keeps a snapshot's revision sequence atomic.
#[derive(Clone)]
pub(crate) struct EventSink(pub(super) Arc<Connection>);
impl EventSink {
    #[cfg(test)]
    pub(crate) fn testing() -> Self {
        Self(Arc::new(super::tests::connection().0))
    }
    #[cfg(test)]
    pub(crate) fn take_frames(&self) -> Vec<Value> {
        self.0.out.lock().unwrap().frames.drain(..).collect()
    }
    #[cfg(test)]
    pub(crate) fn terminal(&self) -> Option<Value> {
        self.0.out.lock().unwrap().terminal.clone()
    }
    pub fn id(&self) -> u64 {
        self.0.number
    }
    pub fn summary_interval(&self) -> u64 {
        let state = self.0.client.lock().unwrap();
        if !state.interactive
            && !state.glance
            && state
                .client
                .as_ref()
                .is_some_and(|c| c.kind == ClientKind::Bar)
        {
            5000
        } else {
            1000
        }
    }
    pub fn closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire) || self.0.out.lock().unwrap().ending
    }
    pub fn close(&self) {
        self.0.finish(ByeReason::InvalidMessage);
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
        self.snapshot_resync(sub, topic, previous, items, reason, None)
    }
    pub fn snapshot_resync(
        &self,
        sub: u64,
        topic: Topic,
        previous: u64,
        items: Vec<Item>,
        reason: SnapshotReason,
        oldest_seq: Option<u64>,
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
        if chunks.is_empty() && reason != SnapshotReason::EngineRestart {
            chunks.push(Vec::new());
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
            oldest_seq,
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

#[cfg(test)]
mod defaults_tests {
    use super::*;
    #[test]
    fn summary_default_follows_bar_visibility() {
        let (connection, _peer) = super::super::tests::connection();
        let sink = EventSink(Arc::new(connection));
        sink.0.client.lock().unwrap().client = Some(Client {
            kind: ClientKind::Bar,
            name: ClientName::new("visibility-test").unwrap(),
            version: "test".into(),
        });
        assert_eq!(sink.summary_interval(), 5000, "idle bar default");
        sink.0.client.lock().unwrap().glance = true;
        assert_eq!(sink.summary_interval(), 1000, "visible popup default");
        sink.0.client.lock().unwrap().glance = false;
        sink.0.client.lock().unwrap().interactive = true;
        assert_eq!(sink.summary_interval(), 1000, "interactive client default");
    }
}
