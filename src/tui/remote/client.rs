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
    pub fn open(&self, hash: &str, file: Option<u64>, folder: bool) -> Result<()> {
        let mut args = json!({"hash":hash});
        if let Some(file) = file { args["file"] = json!(file); }
        if folder { args["folder"] = json!(true); }
        let result: api::OpenTargetResult = request(self.connection()?, "torrents.open_target", args)?;
        let cfg = crate::config::Config::load()?;
        open_result(&cfg, &result, folder)?;
        Ok(())
    }
    pub fn add_url(&self, source: &str, options: &AddOptions) -> Result<String> {
        let result=self.mutate("torrents.add", add_args(source, options)?)?;
        Ok(if result["duplicate"]==true {"Already in your list. Open the existing torrent; no second download was added.".into()}else{format!("added {}",result["name"].as_str().unwrap_or("torrent"))})
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
fn open_result(cfg: &Config, result: &api::OpenTargetResult, folder: bool) -> Result<()> {
    if folder {
        anyhow::ensure!(result.kind == api::TargetKind::Folder, "The download folder is unavailable");
        cfg.open_folder(&result.path)?;
    } else {
        anyhow::ensure!(result.kind == api::TargetKind::File,
            "No file is available to open. Use O to open its folder, or choose a file in Details.");
        crate::public_text::openable_payload(std::path::Path::new(&result.path))?;
        cfg.open_target(result.stream_url.as_deref().unwrap_or(&result.path))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_open_never_hands_a_folder_or_executable_to_the_player() {
        let cfg=Config::default();
        let mut target=api::OpenTargetResult {
            path:"/fixture/downloads".into(), kind:api::TargetKind::Folder, stream_url:None,
        };
        let error=open_result(&cfg,&target,false).unwrap_err().to_string();
        assert!(error.contains("Use O") && error.contains("choose a file"),"{error}");
        target.kind=api::TargetKind::File;
        target.path="/fixture/launcher.desktop".into();
        assert!(open_result(&cfg,&target,false).unwrap_err().to_string().contains("refusing"));
    }

    #[test]
    fn folder_open_rejects_a_file_result_instead_of_launching_media() {
        let target=api::OpenTargetResult {
            path:"/fixture/movie.mkv".into(), kind:api::TargetKind::File,
            stream_url:Some("http://127.0.0.1/stream".into()),
        };
        assert!(open_result(&Config::default(),&target,true)
            .unwrap_err().to_string().contains("folder is unavailable"));
    }
}
fn add_args(source: &str, options: &AddOptions) -> Result<Value> {
    let source = if source.starts_with("magnet:") {
        json!({"magnet":source})
    } else if source.starts_with("https://") || source.starts_with("http://") {
        json!({"url":source})
    } else {
        json!({"path":std::path::Path::new(source).canonicalize().map_err(|_|anyhow::anyhow!(crate::user_messages::UNREADABLE_TORRENT))?.to_str().ok_or_else(||anyhow::anyhow!("torrent path is not UTF-8"))?})
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
