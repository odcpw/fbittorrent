//! CLI service helpers. Torrent adds use the daemon's API v1.
use crate::{config::Config, qbt::{AddOptions, RssFeed, SearchPlugin, SearchResponse}};
use anyhow::Result;
pub struct Client { cfg: Config }
impl Client {
    pub fn from_config(cfg: &Config) -> Self { Self { cfg: cfg.clone() } }
    pub fn add_url(&self, url: &str, options: &AddOptions) -> Result<()> {
        crate::api::client::ensure_daemon(&self.cfg, !self.cfg.autostart || crate::backend::start_prevented())?;
        crate::cli::local_add::add(&self.cfg, serde_json::json!({"magnet":url}), options, None)
    }
    pub fn rss_add_feed(&self, url: &str, name: Option<&str>) -> Result<()> { crate::rss::add(url,name,self.cfg.timeout_secs) }
    pub fn rss_remove(&self, name: &str) -> Result<()> { crate::rss::remove(name) }
    pub fn rss_refresh(&self, name: &str) -> Result<()> { crate::rss::refresh(name,self.cfg.timeout_secs) }
    pub fn rss_mark_read(&self, name: &str, id: Option<&str>) -> Result<()> { crate::rss::mark_read(name,id) }
    pub fn rss_feeds(&self) -> Result<Vec<RssFeed>> { crate::rss::list() }
    pub fn search_plugins(&self) -> Result<Vec<SearchPlugin>> { crate::search::plugins() }
    pub fn search_enable_all(&self) -> Result<()> { for p in self.search_plugins()? { crate::search::set_enabled(&p.name,true)?; } Ok(()) }
    pub fn search_install_plugins(&self, sources: &[String]) -> Result<()> { crate::search::install(sources,self.cfg.timeout_secs) }
    pub fn search_start(&self, pattern: &str) -> Result<i64> { crate::search::start(pattern) }
    pub fn search_results(&self, id: i64, limit: i64, offset: i64) -> Result<SearchResponse> { crate::search::results(id,limit,offset) }
    pub fn search_stop(&self, id: i64) -> Result<()> { crate::search::stop(id) }
    pub fn search_delete(&self, id: i64) -> Result<()> { crate::search::delete(id) }
}
