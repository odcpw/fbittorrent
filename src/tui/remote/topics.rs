//! Replace detail subscriptions serially; rapid selection changes cannot leak slots.
use super::*;
#[derive(Default)]
pub(in crate::tui) struct Selection {
    desired: Vec<api::Topic>,
    active: Vec<(u64, api::Topic)>,
    inflight: bool,
    retired: std::collections::VecDeque<u64>,
}
impl TuiApp {
    pub(in crate::tui) fn sync_topics(&mut self) -> Cmd<Msg> {
        let topics=self.select_topics();let releases=self.refresh_show_filter();Cmd::batch(vec![topics,releases])
    }
    fn select_topics(&mut self) -> Cmd<Msg> {
        let Some(connection) = self.presence.clone().filter(|c| c.alive()) else {
            return Cmd::none();
        };
        let mut desired = Vec::with_capacity(3);
        if connection.hello.caps.iter().any(|c| c == "shows") && !self.full_show_inbox() {
            let id = if self.screen == Screen::Shows {
                self.selected_show().map(|s| s.imdb.clone())
            } else {
                None
            };
            desired.push(api::Topic::ShowReleases(
                id.unwrap_or_else(|| "inbox".into()),
            ));
        }
        if self.screen == Screen::Rss {
            if let Some(id) = self.selected_feed().and_then(|f| self.feed_id(&f.name)) {
                desired.push(api::Topic::RssItems(id));
            }
        }
        if let Some(files) = &self.files {
            if let Ok(hash) = api::Hash::new(files.hash.clone()) {
                desired.push(api::Topic::Files(hash));
            }
        }
        if let Some(topic)=self.tracker_topic(){desired.push(topic);}
        if let Some(topic)=self.peer_topic() {desired.push(topic);}
        self.topic_selection.desired = desired;
        if self.topic_selection.inflight
            || self
                .topic_selection
                .active
                .iter()
                .map(|(_, t)| t)
                .eq(self.topic_selection.desired.iter())
        {
            return Cmd::none();
        }
        self.topic_selection.inflight = true;
        let old = std::mem::take(&mut self.topic_selection.active);
        for (sub, _) in &old {
            self.store.unsubscribe(*sub);
            self.topic_selection.retired.push_back(*sub);
            if self.topic_selection.retired.len() > 64 {
                self.topic_selection.retired.pop_front();
            }
        }
        let desired = self.topic_selection.desired.clone();
        Cmd::task(move || {
            let mut active = Vec::new();
            let result = (|| -> Result<()> {
                for (sub, _) in old {
                    connection.request("unsubscribe", json!({"sub":sub}))?;
                }
                for topic in desired {
                    let reply: api::SubscribeResult =
                        request(&connection, "subscribe", json!({"topics":[topic.as_str()]}))?;
                    active.push((reply.sub, topic));
                }
                Ok(())
            })()
            .map_err(|e| e.to_string());
            Msg::TopicsSelected {
                connection: connection.id,
                active,
                result,
            }
        })
    }
    pub(in crate::tui) fn topics_selected(
        &mut self,
        connection: u64,
        active: Vec<(u64, api::Topic)>,
        result: Result<(), String>,
    ) -> Cmd<Msg> {
        if self.presence.as_ref().is_none_or(|c| c.id != connection) {
            return Cmd::none();
        }
        self.topic_selection.inflight = false;
        self.topic_selection.active = active;
        if let Err(error) = result {
            self.set_status(format!("API selection: {error}"), true);
            return Cmd::none();
        }
        self.sync_topics()
    }
    pub(in crate::tui) fn retired_topic(&self, sub: u64) -> bool {
        self.topic_selection.retired.contains(&sub)
    }
    pub(in crate::tui) fn reset_topics(&mut self) {
        self.topic_selection = Selection::default();
    }
}
