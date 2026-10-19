//! Native search is a view of daemon sessions on the existing UI connection.
use super::*;

pub(in crate::tui) struct Session {
    pub id: u64,
    pub instance: api::Instance,
    pub subscriptions: Vec<u64>,
}
impl Session {
    fn subscribe(connection: &attach::Presence, id: u64) -> Result<Self> {
        let mut subscriptions = Vec::new();
        // Events first: even a provider that already finished has its completion
        // in the retained event snapshot when the model receives the search ID.
        for topic in ["events".to_string(), format!("search:{id}")] {
            let reply: Result<api::SubscribeResult> =
                request(connection, "subscribe", json!({"topics":[topic]}));
            match reply {
                Ok(reply) => subscriptions.push(reply.sub),
                Err(error) => {
                    for sub in subscriptions {
                        let _ = connection.request("unsubscribe", json!({"sub":sub}));
                    }
                    return Err(error);
                }
            }
        }
        Ok(Self {
            id,
            instance: connection.hello.instance.clone(),
            subscriptions,
        })
    }
    fn release(&self, connection: &attach::Presence) {
        if self.instance != connection.hello.instance {
            return;
        }
        for sub in &self.subscriptions {
            let _ = connection.request("unsubscribe", json!({"sub":sub}));
        }
        let _ = connection.request("search.stop", json!({"search_id":self.id}));
    }
}

pub(in crate::tui) fn source(item: &api::SearchItem) -> &str {
    item.magnet.as_deref().or(item.url.as_deref()).unwrap_or("")
}
pub(in crate::tui) fn display(item: &api::SearchItem) -> SearchResult {
    let number = |n: Option<u64>| n.and_then(|n| i64::try_from(n).ok()).unwrap_or(-1);
    SearchResult {
        file_name: item.name.clone(),
        file_url: source(item).into(),
        file_size: number(item.size),
        seeders: number(item.seeds),
        leechers: number(item.leechers),
        engine: item.providers.join(", "),
        site_url: item.provider_url.clone().unwrap_or_default(),
        descr_link: item.page_url.clone().unwrap_or_default(),
        imdb: item.imdb_id.clone().unwrap_or_default(),
        pub_date: item
            .uploaded_at
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(0),
    }
}

impl TuiApp {
    pub(in crate::tui) fn native_search_start(&mut self, query: String) -> Cmd<Msg> {
        let sequence = self.search_seq;
        let previous = self.native_search.take();
        let Some(connection) = self.presence.clone().filter(|c| c.alive()) else {
            self.search_running = false;
            self.search_sub = None;
            self.set_status("fbittorrent stopped; reconnect before searching", true);
            return Cmd::none();
        };
        Cmd::task(move || {
            let result = (|| -> Result<Session> {
                if let Some(previous) = previous {
                    previous.release(&connection);
                }
                let reply: api::SearchStarted = request(
                    &connection,
                    "search.start",
                    json!({"query":query,"limit":5000}),
                )?;
                match Session::subscribe(&connection, reply.search_id) {
                    Ok(session) => Ok(session),
                    Err(error) => {
                        let _ =
                            connection.request("search.stop", json!({"search_id":reply.search_id}));
                        Err(error)
                    }
                }
            })();
            Msg::NativeSearchStarted {
                connection: connection.id,
                sequence,
                result: result.map_err(|e| e.to_string()),
            }
        })
    }
    pub(in crate::tui) fn native_search_resume(&mut self) -> Cmd<Msg> {
        let Some(connection) = self.presence.clone() else {
            return Cmd::none();
        };
        let Some(session) = self.native_search.take() else {
            return Cmd::none();
        };
        if session.instance != connection.hello.instance {
            self.search_running = false;
            self.search_sub = None;
            self.set_status(
                "search session ended when fbittorrent restarted; run the query again",
                true,
            );
            return Cmd::none();
        }
        let sequence = self.search_seq;
        Cmd::task(move || Msg::NativeSearchStarted {
            connection: connection.id,
            sequence,
            result: Session::subscribe(&connection, session.id).map_err(|e| e.to_string()),
        })
    }
    pub(in crate::tui) fn native_search_started(
        &mut self,
        connection: u64,
        sequence: u64,
        result: Result<Session, String>,
    ) -> Cmd<Msg> {
        if self.presence.as_ref().is_none_or(|c| c.id != connection) {
            return Cmd::none();
        }
        if self.search_sub.is_none() || self.search_seq != sequence {
            if let Ok(session) = result {
                let connection = self.presence.clone().unwrap();
                return Cmd::task(move || {
                    session.release(&connection);
                    Msg::SearchDisposed
                });
            }
            return Cmd::none();
        }
        match result {
            Ok(session) => {
                self.native_search = Some(session);
                self.sync_native_search();
            }
            Err(error) => {
                self.search_running = false;
                self.search_sub = None;
                self.set_status(format!("search failed: {error}"), true);
            }
        }
        Cmd::none()
    }
    pub(in crate::tui) fn native_search_cancel(&mut self) -> Cmd<Msg> {
        self.search_sub = None;
        self.search_running = false;
        let session = self.native_search.take();
        let connection = self.presence.clone();
        Cmd::task(move || {
            if let (Some(session), Some(connection)) = (session, connection) {
                session.release(&connection);
            }
            Msg::SearchDisposed
        })
    }
    pub(in crate::tui) fn sync_native_search(&mut self) {
        let Some(session) = &self.native_search else {
            return;
        };
        if let Some(rows) = self.store.searches.get(&session.id) {
            let selected = self
                .selected_result()
                .map(|r| (r.file_name.clone(), r.file_url.clone()));
            let mut rows: Vec<_> = rows.values().collect();
            rows.sort_by(|a, b| {
                b.seeds
                    .cmp(&a.seeds)
                    .then(a.name.cmp(&b.name))
                    .then(a.providers.cmp(&b.providers))
            });
            self.search_wire = rows.into_iter().take(MAX_RESULTS).cloned().collect();
            self.search_results = self.search_wire.iter().map(display).collect();
            let index = selected.and_then(|(name, url)| {
                self.search_results
                    .iter()
                    .position(|r| r.file_name == name && r.file_url == url)
            });
            self.search_list.borrow_mut().select(index.or_else(|| {
                (!self.search_focus_input && !self.search_results.is_empty()).then_some(0)
            }));
        }
        let done = self.store.events.iter().rev().find_map(|event| {
            if let api::Event::SearchDone {
                search_id,
                total,
                warnings,
                ..
            } = event
            {
                (*search_id == session.id).then_some((*total, warnings.clone()))
            } else {
                None
            }
        });
        if let Some((total, warnings)) = done.filter(|_| self.search_running) {
            self.search_running = false;
            let warning = warnings
                .iter()
                .map(|w| format!("{}: {}", w.provider, w.message))
                .collect::<Vec<_>>()
                .join("; ");
            let text = if warnings.is_empty() {
                format!("{total} result{}", if total == 1 { "" } else { "s" })
            } else {
                format!("{total} results · some providers failed: {warning}")
            };
            self.set_status(text, !warnings.is_empty());
        }
    }
    pub(in crate::tui) fn wire_result(&self, result: &SearchResult) -> Option<&api::SearchItem> {
        self.search_wire
            .iter()
            .find(|row| row.name == result.file_name && source(row) == result.file_url)
    }
    pub(in crate::tui) fn native_search_grab(&self, result: &SearchResult) -> Cmd<Msg> {
        let Some(row) = self.wire_result(result).cloned() else {
            return Cmd::none();
        };
        let connection = self.presence.clone();
        let options = self.add_opts();
        let mut args = json!({"seed":options.seed,"stopped":options.stopped});
        if let Some(session) = &self.native_search {
            args["search_id"] = json!(session.id);
            args["key"] = json!(row.key);
        } else {
            args["result"] = json!(row);
        }
        if let Some(path) = options.save_path {
            args["save_path"] = json!(path);
        }
        if let Some(category) = options.category {
            args["category"] = json!(category);
        }
        Cmd::task(move || {
            Msg::ActionDone(
                connection
                    .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))
                    .and_then(|c| c.request_confirmed("search.grab", args))
                    .map(|_| "added search result".into())
                    .map_err(|e| e.to_string()),
            )
        })
    }
    pub(in crate::tui) fn native_browse(&mut self, entry: browse::Entry) -> Cmd<Msg> {
        let retired = self.native_search.take();
        self.search_sub = None;
        self.search_running = false;
        self.search_seq += 1;
        let sequence = self.search_seq;
        let connection = self.presence.clone();
        let identity = connection.as_ref().map(|c| c.id);
        let label = format!("{} · {}", entry.provider, entry.label);
        self.browse_loading = Some(label.clone());
        Cmd::task(move || {
            let result = (|| -> Result<Option<Vec<api::SearchItem>>> {
                let connection =
                    connection.ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?;
                if let Some(session) = retired {
                    session.release(&connection);
                }
                let sources: api::BrowseSourcesResult =
                    request(&connection, "search.browse_sources", json!({}))?;
                let source = sources
                    .sources
                    .into_iter()
                    .find(|s| s.provider == entry.provider && s.label == entry.label)
                    .ok_or_else(|| anyhow::anyhow!("browse source is no longer available"))?;
                if !source.in_app {
                    std::process::Command::new("xdg-open")
                        .arg(
                            source
                                .url
                                .ok_or_else(|| anyhow::anyhow!("source has no URL"))?,
                        )
                        .spawn()?;
                    return Ok(None);
                }
                let reply: api::SearchItemsResult =
                    request(&connection, "search.browse", json!({"source":source.id}))?;
                Ok(Some(reply.items))
            })();
            Msg::NativeBrowse {
                connection: identity,
                sequence,
                label,
                result: result.map_err(|e| e.to_string()),
            }
        })
    }
}

#[cfg(test)]
mod tests;
