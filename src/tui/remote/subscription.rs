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
    pub(in crate::tui) fn subscribe_ui(&self)->Cmd<Msg>{
        let Some(connection)=self.presence.clone()else{return Cmd::none();};
        Cmd::task(move||{
            let mut topics=vec!["summary","torrents","network"];
            for (topic,cap) in [("stats","history"),("settings","settings"),("rss.feeds","rss"),("shows","shows"),("log","log")] {
                if connection.hello.caps.iter().any(|c|c==cap){topics.push(topic);}
            }
            let result=(||->Result<()> {
                // Each topic has its own sub/revision so a gap can replace only that topic.
                for topic in topics {connection.request("subscribe",json!({"topics":[topic]}))?;}
                Ok(())
            })().map_err(|e|e.to_string());
            Msg::UiSubscribed{connection:connection.id,result}
        })
    }

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
                if event.subscription_revision().is_some_and(|(sub,_)|self.retired_topic(sub)){return Cmd::none();}
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
                if matches!(&event,api::Event::SnapshotEnd{topic:api::Topic::Log,..}|api::Event::Delta{topic:api::Topic::Log,..}){self.feedback.dirty.set(true);}
                let committed=matches!(&event,api::Event::SnapshotEnd{..}|api::Event::Delta{..}|api::Event::State{..});
                if committed {
                    match &event {
                        api::Event::SnapshotEnd{topic:api::Topic::RssFeeds|api::Topic::RssItems(_),..}|api::Event::Delta{topic:api::Topic::RssFeeds|api::Topic::RssItems(_),..}=>self.sync_native_feeds(),
                        api::Event::SnapshotEnd{topic:api::Topic::Shows|api::Topic::ShowReleases(_),..}|api::Event::Delta{topic:api::Topic::Shows|api::Topic::ShowReleases(_),..}=>self.sync_native_shows(),
                        api::Event::State{data:api::StateData::Settings(settings),..}=>{if let Err(e)=settings_fields::native_settings(&mut self.cfg,settings){self.set_status(e.to_string(),true);}},
                        api::Event::State{data:api::StateData::Summary(summary),..}=>{
                            let transfer=api::TransferInfo {session_downloaded:0,session_uploaded:0,dl_bps:summary.dl_bps,ul_bps:summary.ul_bps};
                            self.transfer=format::transfer(&transfer,self.store.network.as_ref());
                        },
                        api::Event::SnapshotEnd{topic:api::Topic::Files(hash),..}|api::Event::Delta{topic:api::Topic::Files(hash),..}=>{
                            if self.files.as_ref().is_some_and(|m|m.hash==hash.as_str()) {
                                let files=self.store.files.get(hash).into_iter().flat_map(|rows|rows.values()).map(format::file).collect();
                                self.update(Msg::Files(hash.to_string(),Ok(files)));
                            }
                        },
                        _=>{}
                    }
                }
                if rows_changed {
                    let rows = self
                        .store
                        .sorted_torrents(self.torrent_sort,self.torrent_descending)
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
        self.sync_topics()
    }
}
pub(in crate::tui) fn verb(op: &str) -> &'static str {
    match op {
        "torrents.add" | "search.grab" | "rss.grab" | "shows.grab" => "Adding",
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
