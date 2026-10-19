//! Deliver the connection's ordered updates to the model, independent of RPC waiters.
use super::*;
pub(in crate::tui) struct ApiSub {
    pub connection: Arc<attach::Presence>,
}
impl Subscription<Msg> for ApiSub {
    fn id(&self) -> SubId {
        self.connection.id
    }
    fn run(&self, sender: Sender<Msg>, stop: StopSignal) {
        while !stop.is_stopped() {
            let (update, terminal) = match self.connection.next_update() {
                Ok(Some(update)) => (update, false),
                Ok(None) => continue,
                Err(error) => (attach::ApiUpdate::Closed(error.to_string()), true),
            };
            // One message may be awaiting the model. Backpressure never grows ftui's queue.
            let (ack, consumed) = std::sync::mpsc::sync_channel(1);
            if sender
                .send(Msg::Api {
                    connection: self.connection.id,
                    update,
                    ack,
                })
                .is_err()
            {
                return;
            }
            loop {
                if stop.is_stopped() {
                    return;
                }
                match consumed.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return,
                }
            }
            if terminal {
                return;
            }
        }
    }
}
impl TuiApp {
    pub(in crate::tui) fn api_update(
        &mut self,
        connection: u64,
        update: attach::ApiUpdate,
    ) -> Cmd<Msg> {
        if self
            .presence
            .as_ref()
            .is_none_or(|active| active.id != connection)
        {
            return Cmd::none();
        }
        match update {
            attach::ApiUpdate::Event(event) => {
                if let api::Event::Bye {reason}=&event {
                    self.connection_lost(matches!(reason,api::ByeReason::Shutdown|api::ByeReason::Idle));
                }
                let rows_changed = matches!(
                    &event,
                    api::Event::SnapshotEnd {
                        topic: api::Topic::Torrents,
                        ..
                    } | api::Event::Delta {
                        topic: api::Topic::Torrents,
                        ..
                    }
                );
                match self.store.event(&event) {
                    Ok(store::Effect::Resubscribe { sub, topic }) => {
                        self.store.unsubscribe(sub);
                        let connection = self.presence.clone().unwrap();
                        return Cmd::task(move || {
                            let result = connection
                                .request("unsubscribe", json!({"sub":sub}))
                                .and_then(|_| {
                                    connection
                                        .request("subscribe", json!({"topics":[topic.as_str()]}))
                                });
                            Msg::ActionDone(
                                result
                                    .map(|_| "requested a fresh snapshot".into())
                                    .map_err(|error| error.to_string()),
                            )
                        });
                    }
                    Err(error) => {
                        self.set_status(format!("Invalid API update: {error}"), true);
                        return Cmd::none();
                    }
                    _ => {}
                }
                if matches!(&event,api::Event::SnapshotEnd {topic:api::Topic::Search(_)|api::Topic::Events,..}
                    |api::Event::Delta {topic:api::Topic::Search(_)|api::Topic::Events,..}|api::Event::SearchDone {..}) {
                    self.sync_native_search();
                }
                if rows_changed {
                    let rows = self
                        .store
                        .sorted_torrents(store::TorrentSort::Added, true)
                        .into_iter()
                        .map(format::torrent)
                        .collect();
                    return self.update(Msg::Torrents(Ok((rows, self.transfer.clone()))));
                }
            }
            attach::ApiUpdate::Sent {
                reference,
                op,
                hashes,
            } => {
                if let Err(error) = self.store.begin_mutation(reference, op.into(), hashes) {
                    self.set_status(error.to_string(), true);
                }
            }
            attach::ApiUpdate::Acknowledged { reference, pending } => {
                if !pending {
                    self.store.pending.remove(&reference);
                } else if let Some(request) = self.store.pending.get(&reference) {
                    self.set_status(format!("{}…", verb(&request.op)), false);
                }
            }
            attach::ApiUpdate::Failed {
                reference,
                error,
                unknown,
            } => {
                self.store.mutation_failed(&reference, unknown);
                self.set_status(error, true);
            }
            attach::ApiUpdate::Closed(error) => {
                self.backend_error = error;
                self.connection_lost(false);
            }
        }
        Cmd::none()
    }
}
pub(in crate::tui) fn verb(op: &str) -> &'static str {
    match op {
        "torrents.add" | "search.grab" => "Adding",
        "search.start" => "Searching",
        "search.stop" => "Stopping search",
        "search.set_provider" => "Updating provider",
        "torrents.start" => "Starting",
        "torrents.stop" => "Stopping",
        "torrents.remove" => "Removing",
        "torrents.set_files" => "Selecting files",
        _ => "Updating",
    }
}
