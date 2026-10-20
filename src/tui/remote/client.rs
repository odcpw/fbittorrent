//! Screen commands share the TUI's single API connection.
use super::*;
use serde_json::Value;

pub(in crate::tui) struct ScreenClient {
    connection: Option<Arc<attach::Presence>>,
}
impl ScreenClient {
    pub fn new(connection: Option<Arc<attach::Presence>>) -> Self { Self { connection } }
    fn connection(&self) -> Result<&attach::Presence> {
        self.connection.as_deref().filter(|c| c.alive()).ok_or_else(|| anyhow::anyhow!("fbittorrent stopped"))
    }
    fn mutate(&self, op: &'static str, args: Value) -> Result<Value> {
        self.connection()?.request_confirmed(op, args)
    }
    pub fn start(&self, hashes: &[String]) -> Result<()> {
        for hashes in hashes.chunks(api::MAX_HASHES) {
            let reply: api::StartResult = serde_json::from_value(self.mutate("torrents.start", json!({"hashes":hashes}))?)?;
            if let Some(skipped) = reply.skipped.first() { anyhow::bail!("start skipped {}: {}", skipped.hash, skipped.reason); }
        }
        Ok(())
    }
    pub fn stop(&self, hashes: &[String]) -> Result<()> {
        for hashes in hashes.chunks(api::MAX_HASHES) { self.mutate("torrents.stop", json!({"hashes":hashes}))?; }
        Ok(())
    }
    pub fn open(&self, torrent: &Torrent) -> Result<()> {
        let result: api::OpenTargetResult = request(self.connection()?, "torrents.open_target", json!({"hash":torrent.hash}))?;
        crate::public_text::openable_payload(std::path::Path::new(&result.path))?;
        std::process::Command::new("xdg-open").arg(result.path).spawn()?;
        Ok(())
    }
    pub fn add_url(&self, source: &str, options: &AddOptions) -> Result<()> {
        self.mutate("torrents.add", add_args(source, options)?)?;
        Ok(())
    }
    pub fn search_plugins(&self) -> Result<Vec<SearchPlugin>> {
        let result: api::ProvidersResult = request(self.connection()?, "search.providers", json!({}))?;
        Ok(result.providers.into_iter().map(|p| SearchPlugin {kind:if p.kind==api::ProviderKind::Torznab {"torznab"}else{"nova"}.into(),name:p.name, full_name:p.full_name.unwrap_or_default(), url:p.url.unwrap_or_default(), version:p.version.unwrap_or_default(), enabled:p.enabled}).collect())
    }
    pub fn search_set_plugin(&self, name: &str, on: bool) -> Result<()> {
        self.mutate("search.set_provider", json!({"name":name,"enabled":on}))?;
        Ok(())
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
