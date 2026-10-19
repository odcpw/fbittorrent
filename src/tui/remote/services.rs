//! Service screens are projections of confirmed daemon data, never store writers.
use super::*;
use serde_json::Value;

impl TuiApp {
    pub(in crate::tui) fn feed_id(&self, name: &str) -> Option<String> {
        self.feeds
            .iter()
            .find(|f| f.name == name || f.uid == name)
            .filter(|f| !f.uid.is_empty())
            .map(|f| f.uid.clone())
            .or_else(|| {
                self.store
                    .rss_feeds
                    .values()
                    .find(|f| f.name == name || f.id == name)
                    .map(|f| f.id.clone())
            })
    }
    pub(in crate::tui) fn service_action(
        &mut self,
        op: &'static str,
        args: Value,
        label: impl Into<String>,
    ) -> Cmd<Msg> {
        if self.service_busy {
            return Cmd::none();
        }
        self.service_busy = true;
        if op.starts_with("rss.") {
            self.rss_busy = true;
        }
        let connection = self.presence.clone();
        let identity = connection.as_ref().map(|c| c.id);
        let label = label.into();
        self.set_status(format!("{}…", mutation_verb(op)), false);
        Cmd::task(move || {
            let result = connection
                .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))
                .and_then(|c| c.request_confirmed(op, args.clone()))
                .map_err(|e| e.to_string());
            Msg::ServiceDone {
                connection: identity,
                op,
                args,
                label,
                result,
            }
        })
    }
    pub(in crate::tui) fn service_done(
        &mut self,
        connection: Option<u64>,
        op: &str,
        args: Value,
        label: String,
        result: Result<Value, String>,
    ) -> Cmd<Msg> {
        if self.presence.as_ref().map(|c| c.id) != connection {
            return Cmd::none();
        }
        self.service_busy = false;
        self.rss_busy = false;
        self.show_grabbing.clear();
        if op.starts_with("shows."){self.show_controls.dirty=true;}
        match result {
            Ok(value) => {
                if op == "shows.set_read" && args["read"] == true {
                    if let (Some(imdb), Some(key)) = (args["imdb"].as_str(), args["key"].as_str()) {
                        self.inbox_last_read = Some(ShowReleaseTarget {
                            imdb: imdb.into(),
                            key: key.into(),
                        });
                    }
                }
                if op == "shows.undo_read" {
                    self.inbox_last_read = None;
                }
                if op == "shows.follow" {
                    if let Some(id) = value["show"]["imdb"].as_str() {
                        self.follow_target = Some(id.into());
                        self.sync_native_shows();
                    }
                }
                self.set_success(label);
            }
            Err(error) => self.set_status(error, true),
        }
        self.sync_topics()
    }
    pub(in crate::tui) fn native_feed_action(
        &mut self,
        op: &'static str,
        name: &str,
        item: Option<&str>,
        read: Option<bool>,
        label: impl Into<String>,
    ) -> Cmd<Msg> {
        let Some(id) = self.feed_id(name) else {
            self.set_status("Feed is no longer available", true);
            return Cmd::none();
        };
        let mut args = json!({"feed":id});
        if let Some(item) = item {
            args["item"] = json!(item);
        }
        if let Some(read) = read {
            args["read"] = json!(read);
        }
        if op == "rss.grab" {
            args["seed"] = json!(self.cfg.seed_by_default);
        }
        self.service_action(op, args, label)
    }
    pub(in crate::tui) fn sync_native_feeds(&mut self) {
        let selected = self.selected_feed().map(|f| f.name.clone());
        let item = self
            .item_list
            .borrow()
            .selected()
            .and_then(|i| self.selected_feed().and_then(|f| f.articles.get(i)))
            .map(|a| a.id.clone());
        self.feeds = self
            .store
            .rss_feeds
            .values()
            .map(|f| RssFeed {
                name: f.name.clone(),
                title: f.title.clone().unwrap_or_else(|| f.name.clone()),
                url: f.url.clone(),
                uid: f.id.clone(),
                is_loading: f.loading,
                has_error: f.error.is_some(),
                articles: self
                    .store
                    .rss_items
                    .get(&f.id)
                    .into_iter()
                    .flat_map(|rows| rows.values())
                    .map(|a| crate::qbt::RssArticle {
                        id: a.id.clone(),
                        title: a.title.clone(),
                        description: a.description.clone().unwrap_or_default(),
                        date: a.published_at.map(|n| ymd(n as i64)).unwrap_or_default(),
                        link: a.link.clone().unwrap_or_default(),
                        torrent_url: a
                            .magnet
                            .clone()
                            .or(a.torrent_url.clone())
                            .unwrap_or_default(),
                        is_read: a.read,
                        size: a.size.and_then(|n| n.try_into().ok()),
                        ts: a.published_at.unwrap_or(0) as i64,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        self.feeds.sort_by(|a, b| a.name.cmp(&b.name));
        for feed in &mut self.feeds {
            feed.articles
                .sort_by(|a, b| b.ts.cmp(&a.ts).then(a.id.cmp(&b.id)));
        }
        self.feed_list.borrow_mut().select(
            selected
                .and_then(|name| self.feeds.iter().position(|f| f.name == name))
                .or_else(|| (!self.feeds.is_empty()).then_some(0)),
        );
        let index = item
            .and_then(|id| {
                self.selected_feed()
                    .and_then(|f| f.articles.iter().position(|a| a.id == id))
            })
            .or_else(|| {
                self.selected_feed()
                    .filter(|f| !f.articles.is_empty())
                    .map(|_| 0)
            });
        self.item_list.borrow_mut().select(index);
        self.rss_loading = false;
    }
    pub(in crate::tui) fn feed_counts(&self, feed: &RssFeed) -> (usize, Option<usize>) {
        self.store.rss_feeds.get(&feed.uid).map_or_else(
            || {
                (
                    feed.articles.iter().filter(|a| !a.is_read).count(),
                    Some(feed.articles.len()),
                )
            },
            |f| {
                (
                    f.unread as usize,
                    self.store.rss_items.get(&f.id).map(|items| items.len()),
                )
            },
        )
    }
    pub(in crate::tui) fn release_metadata(
        &self,
        show: &Show,
        episode: &Episode,
        width: usize,
    ) -> String {
        let key = if self.store.releases_topic.as_deref() == Some("inbox") {
            format!("{}/{}", show.imdb, episode.key)
        } else {
            episode.key.clone()
        };
        if let Some(row) = self
            .store
            .show_releases
            .get(&key)
            .filter(|r| r.show_imdb == show.imdb)
        {
            let quality = row.quality.as_deref().unwrap_or("—");
            let size = row
                .size
                .map(|n| human_bytes(n as i64))
                .unwrap_or_else(|| "—".into());
            let seeds = row
                .seeds
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into());
            let date = row
                .published_at
                .map(|n| ymd(n as i64))
                .unwrap_or_else(|| "—".into());
            if width >= 80 {
                return format!(" {quality:>5}  {size:>9}  {seeds:>5}  {date:>10}");
            }
            if width >= 60 {
                return format!(" {quality:>5}  {size:>9}  {seeds:>5}");
            }
            return format!(" {quality:>5}  {seeds:>5}");
        }
        screens_shows::episode_metadata(Some(episode), width)
    }
    pub(in crate::tui) fn shows_unread(&self) -> usize {
        if self.store.hello.is_some() {
            self.store.shows.values().map(|s| s.unread as usize).sum()
        } else {
            self.shows.new_total()
        }
    }
    pub(in crate::tui) fn show_unread(&self, show: &Show) -> usize {
        self.store
            .shows
            .get(&show.imdb)
            .map_or_else(|| show.new_count(), |s| s.unread as usize)
    }
    pub(in crate::tui) fn sync_native_shows(&mut self) {
        let parent = self.selected_show().map(|s| s.imdb.clone());
        let selected = self.selected_release_target();
        let old = std::mem::take(&mut self.shows.shows)
            .into_iter()
            .map(|s| (s.imdb.clone(), s))
            .collect::<std::collections::BTreeMap<_, _>>();
        self.shows_refreshing = self
            .store
            .shows
            .values()
            .filter(|s| s.checking)
            .map(|s| s.imdb.clone())
            .collect();
        let full=self.show_controls.rows.as_ref().filter(|_|self.full_show_inbox());
        let release_topic=if full.is_some(){Some("inbox")}else{self.store.releases_topic.as_deref()};
        self.shows.shows = self
            .store
            .shows
            .values()
            .map(|s| {
                let mut show = old.get(&s.imdb).cloned().unwrap_or_default();
                show.imdb = s.imdb.clone();
                show.title = s.title.clone();
                show.year = s.year.and_then(|n| n.try_into().ok());
                show.quality = s.quality.clone();
                show.last_checked = s.last_check_at.unwrap_or(0) as i64;
                show.catalogue_complete = s.catalogue_complete;
                show.error = s.warning.clone();
                if let Some(topic) = release_topic {
                    if topic == "inbox" || topic == s.imdb.as_str() {
                        show.episodes.clear();
                        show.seen.clear();
                        show.grabbed.clear();
                        for r in self.store.show_releases.values().filter(|_|full.is_none())
                            .chain(full.into_iter().flat_map(|rows|rows.iter()))
                            .filter(|r| r.show_imdb == s.imdb)
                        {
                            show.episodes.push(Episode {
                                key: r.key.clone(),
                                title: r.title.clone(),
                                magnet: r.magnet.clone().unwrap_or_default(),
                                season: r.season.and_then(|n| n.try_into().ok()).unwrap_or(0),
                                episode: r.episode.and_then(|n| n.try_into().ok()).unwrap_or(0),
                                quality: r.quality.clone().unwrap_or_default(),
                                size: r.size.and_then(|n| n.try_into().ok()).unwrap_or(-1),
                                seeds: r.seeds.and_then(|n| n.try_into().ok()).unwrap_or(-1),
                                released: r.published_at.unwrap_or(0) as i64,
                                candidates: r.candidates.unwrap_or(0) as usize,
                            });
                            if r.read {
                                show.seen.insert(r.key.clone());
                            }
                            if r.transfer != api::ReleaseTransfer::None {
                                show.grabbed
                                    .insert(r.key.clone(), r.magnet.clone().unwrap_or_default());
                            }
                        }
                        show.episodes.sort_by(|a, b| {
                            b.released.cmp(&a.released).then_with(|| b.key.cmp(&a.key))
                        });
                    }
                }
                show
            })
            .collect();
        self.shows
            .shows
            .sort_by(|a, b| a.title.cmp(&b.title).then(a.imdb.cmp(&b.imdb)));
        let target = self.follow_target.clone().or(parent);
        let index = target
            .and_then(|id| self.shows.shows.iter().position(|s| s.imdb == id))
            .map(|n| n + 1)
            .unwrap_or(0);
        self.show_list.borrow_mut().select(Some(index));
        if self.follow_target.is_some() && index > 0 {
            self.follow_target = None;
            self.shows_focus_eps = false;
        }
        self.rebuild_inbox();
        if let Some(target) = selected {
            if !self.shows_inbox() {
                let pos = self
                    .selected_show()
                    .filter(|s| s.imdb == target.imdb)
                    .and_then(|s| s.episodes.iter().position(|e| e.key == target.key));
                self.ep_list.borrow_mut().select(pos);
            }
        }
    }
}
