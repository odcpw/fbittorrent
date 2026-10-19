//! Screen interface while Track B's RSS/Shows services are still local.
//! Native torrent commands use the API connection; legacy and pending service paths stay isolated here.
use super::*;
use serde_json::Value;

pub(in crate::tui) struct ScreenClient {
    cfg: Config,
    connection: Option<Arc<attach::Presence>>,
    local: LegacyClient,
}
impl ScreenClient {
    pub fn from_config(cfg: &Config, connection: Option<Arc<attach::Presence>>) -> Self {
        Self {
            cfg: cfg.clone(),
            connection,
            local: LegacyClient::from_config(cfg),
        }
    }
    pub fn login(&self) -> Result<()> {
        self.local.login()
    }
    fn mutate(&self, op: &'static str, args: Value) -> Result<Value> {
        let reply = self
            .connection
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?
            .request_confirmed(op, args)?;
        Ok(reply)
    }
    pub fn start(&self, hashes: &[String]) -> Result<()> {
        if !self.cfg.native() {
            return self.local.start(hashes);
        }
        for hashes in hashes.chunks(api::MAX_HASHES) {
            let reply = self.mutate("torrents.start", json!({"hashes":hashes}))?;
            let reply: api::StartResult = serde_json::from_value(reply)?;
            if let Some(skipped) = reply.skipped.first() {
                anyhow::bail!("start skipped {}: {}", skipped.hash, skipped.reason);
            }
        }
        Ok(())
    }
    pub fn stop(&self, hashes: &[String]) -> Result<()> {
        if !self.cfg.native() {
            return self.local.stop(hashes);
        }
        for hashes in hashes.chunks(api::MAX_HASHES) {
            self.mutate("torrents.stop", json!({"hashes":hashes}))?;
        }
        Ok(())
    }
    pub fn delete(&self, hashes: &[String], delete_files: bool) -> Result<()> {
        if !self.cfg.native() {
            return self.local.delete(hashes, delete_files);
        }
        for hashes in hashes.chunks(api::MAX_HASHES) {
            self.mutate(
                "torrents.remove",
                json!({"hashes":hashes,"delete_files":delete_files}),
            )?;
        }
        Ok(())
    }
    fn seed(&self, hashes: &[String], on: bool) -> Result<()> {
        for hashes in hashes.chunks(api::MAX_HASHES) {
            self.mutate("torrents.set_seed", json!({"hashes":hashes,"on":on}))?;
        }
        Ok(())
    }
    pub fn allow_seeding(&self, hashes: &[String]) -> Result<()> {
        if self.cfg.native() {
            self.seed(hashes, true)
        } else {
            self.local.allow_seeding(hashes)
        }
    }
    pub fn deny_seeding(&self, hashes: &[String]) -> Result<()> {
        if self.cfg.native() {
            self.seed(hashes, false)
        } else {
            self.local.deny_seeding(hashes)
        }
    }
    pub fn toggle_sequential(&self, hashes: &[String]) -> Result<()> {
        anyhow::ensure!(!self.cfg.native(), "native streaming priority is automatic");
        self.local.toggle_sequential(hashes)
    }
    pub fn toggle_first_last(&self, hashes: &[String]) -> Result<()> {
        anyhow::ensure!(!self.cfg.native(), "native streaming priority is automatic");
        self.local.toggle_first_last(hashes)
    }
    pub fn open(&self, torrent: &Torrent) -> Result<()> {
        let path = if self.cfg.native() {
            let connection = self
                .connection
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?;
            let result: api::OpenTargetResult = request(
                connection,
                "torrents.open_target",
                json!({"hash":torrent.hash}),
            )?;
            result.path
        } else {
            crate::qbt::open_target(torrent)
        };
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
    pub fn add_url(&self, source: &str, options: &AddOptions) -> Result<()> {
        if !self.cfg.native() {
            return self.local.add_url(source, options);
        }
        self.mutate("torrents.add", add_args(source, options)?)?;
        Ok(())
    }
    pub fn add_search_result(&self, result: &SearchResult, options: &AddOptions) -> Result<()> {
        if !self.cfg.native() {
            return self.local.add_search_result(result, options);
        }
        if self.cfg.network_interface.is_some() {
            let source = result.file_url.trim();
            let direct = source.starts_with("magnet:")
                || ((source.starts_with("https://") || source.starts_with("http://"))
                    && source
                        .split(['?', '#'])
                        .next()
                        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".torrent")));
            if direct {
                return self.add_url(source, options);
            }
            let known = crate::search::plugins()?
                .iter()
                .any(|plugin| plugin.name == result.engine);
            if !known && (source.starts_with("https://") || source.starts_with("http://")) {
                return self.add_url(source, options);
            }
            anyhow::bail!("this provider needs an external download helper; use its magnet link or a .torrent URL with VPN binding enabled");
        }
        match crate::search::resolve_download(result, self.cfg.timeout_secs)? {
            crate::search::ResolvedDownload::Url(url) => self.add_url(&url, options),
            crate::search::ResolvedDownload::Torrent { bytes, .. } => {
                let path = super::metadata::publish(&bytes)?;
                let result = self.mutate(
                    "torrents.add",
                    add_args(
                        path.to_str()
                            .ok_or_else(|| anyhow::anyhow!("metadata path is not UTF-8"))?,
                        options,
                    )?,
                );
                if result.as_ref().is_ok_and(|value| value["state"] == "done") {
                    let _ = std::fs::remove_file(path);
                }
                result.map(|_| ())
            }
        }
    }
    // These namespaces remain on their current path until Track B is integrated.
    pub fn rss_feeds(&self) -> Result<Vec<RssFeed>> {
        self.local.rss_feeds()
    }
    pub fn rss_add_feed(&self, url: &str, name: Option<&str>) -> Result<()> {
        self.local.rss_add_feed(url, name)
    }
    pub fn rss_remove(&self, name: &str) -> Result<()> {
        self.local.rss_remove(name)
    }
    pub fn rss_refresh(&self, name: &str) -> Result<()> {
        self.local.rss_refresh(name)
    }
    pub fn rss_mark_read(&self, name: &str, id: Option<&str>) -> Result<()> {
        self.local.rss_mark_read(name, id)
    }
    pub fn rss_set_read(&self, name: &str, id: &str, read: bool) -> Result<()> {
        self.local.rss_set_read(name, id, read)
    }
    pub fn search_plugins(&self) -> Result<Vec<SearchPlugin>> {
        if self.cfg.native() {
            let connection=self.connection.as_ref().ok_or_else(||anyhow::anyhow!("fbittorrent stopped"))?;
            let result:api::ProvidersResult=request(connection,"search.providers",json!({}))?;
            return Ok(result.providers.into_iter().map(|p|SearchPlugin {name:p.name,full_name:p.full_name.unwrap_or_default(),url:p.url.unwrap_or_default(),version:p.version.unwrap_or_default(),enabled:p.enabled}).collect());
        }
        self.local.search_plugins()
    }
    pub fn search_set_plugin(&self, name: &str, on: bool) -> Result<()> {
        if self.cfg.native(){self.mutate("search.set_provider",json!({"name":name,"enabled":on}))?;return Ok(());}
        self.local.search_set_plugin(name, on)
    }
    pub fn search_start(&self, query: &str) -> Result<i64> {
        anyhow::ensure!(!self.cfg.native(),"native search uses pushed API sessions");
        self.local.search_start(query)
    }
    pub fn search_results(
        &self,
        id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<crate::qbt::SearchResponse> {
        anyhow::ensure!(!self.cfg.native(),"native search uses pushed API sessions");
        self.local.search_results(id, limit, offset)
    }
    pub fn search_stop(&self, id: i64) -> Result<()> {
        anyhow::ensure!(!self.cfg.native(),"native search uses pushed API sessions");
        self.local.search_stop(id)
    }
    pub fn search_delete(&self, id: i64) -> Result<()> {
        anyhow::ensure!(!self.cfg.native(),"native search uses pushed API sessions");
        self.local.search_delete(id)
    }
    pub fn torrents(&self) -> Result<Vec<Torrent>> {
        if self.cfg.native() {
            let connection = self
                .connection
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?;
            let rows: api::ListResult = request(connection, "torrents.list", json!({}))?;
            return Ok(rows.torrents.iter().map(format::torrent).collect());
        }
        self.local.torrents()
    }
    pub fn transfer_info(&self) -> Result<TransferInfo> {
        if self.cfg.native() {
            let connection = self
                .connection
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?;
            let transfer: api::TransferInfo = request(connection, "transfer.info", json!({}))?;
            let network: api::NetworkResult = request(connection, "app.network", json!({}))?;
            return Ok(format::transfer(&transfer, Some(&network.state)));
        }
        self.local.transfer_info()
    }
    pub fn files(&self, hash: &str) -> Result<Vec<TorrentFile>> {
        if self.cfg.native() {
            return files(
                self.connection
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?,
                hash,
            );
        }
        self.local.files(hash)
    }
    pub fn set_file_priority(&self, hash: &str, ids: &[i64], priority: i64) -> Result<()> {
        if self.cfg.native() {
            set_files(
                self.connection
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))?,
                hash,
                ids,
                priority,
            )?;
            return Ok(());
        }
        self.local.set_file_priority(hash, ids, priority)
    }
    // P3-05 has not supplied settings operations on this branch yet.
    pub fn set_queue_limit(&self, limit: usize) -> Result<()> {
        self.local.set_queue_limit(limit)
    }
}
fn add_args(source: &str, options: &AddOptions) -> Result<Value> {
    let source = if source.starts_with("magnet:") {
        json!({"magnet":source})
    } else if source.starts_with("https://") || source.starts_with("http://") {
        json!({"url":source})
    } else {
        json!({"path":std::path::Path::new(source).canonicalize()?.to_str().ok_or_else(||anyhow::anyhow!("torrent path is not UTF-8"))?})
    };
    let mut args = json!({"source":source,"seed":options.seed,"stopped":options.stopped});
    if let Some(path) = &options.save_path {
        args["save_path"] = json!(path);
    }
    if let Some(category) = &options.category {
        args["category"] = json!(category);
    }
    Ok(args)
}
