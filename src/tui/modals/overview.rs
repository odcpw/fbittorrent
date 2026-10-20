//! Overview intents capture the hash at prompt creation and wait for confirmation.
use super::*;
impl TuiApp {
    pub(super) fn overview_available(&self, cap: &str) -> bool {
        self.online && !self.service_busy
            && self.store.hello.as_ref().is_some_and(|h| h.caps.iter().any(|c| c == cap))
            && self.selected_torrent().is_some_and(|t| self.store.pending_for_key(t.hash.as_str()).is_none())
    }
    pub(super) fn overview_prompt(&mut self, limit: bool) -> Cmd<Msg> {
        let Some(torrent)=self.selected_torrent() else {return Cmd::none()};
        let hash=torrent.hash.to_string();
        let value=if limit {torrent.download_limit_bps.map(|n|n.to_string()).unwrap_or_default()} else {torrent.save_path.clone()};
        let kind=if limit {PromptKind::TorrentDownloadCap {hash}}else{PromptKind::MoveTorrent {hash}};
        self.open_prompt(kind,if limit {"Download cap · B/s, K/M · empty or none = unlimited"}else{"Move to folder · absolute path"},false);
        self.prompt.as_mut().unwrap().input.set_value(value);Cmd::none()
    }
    pub(super) fn apply_overview(&mut self,kind:PromptKind,value:&str)->Cmd<Msg> {
        let parsed=(||->Result<(&'static str,serde_json::Value,&'static str)>{
            match &kind {
                PromptKind::MoveTorrent {hash}=>{
                    let path=std::path::Path::new(value);
                    anyhow::ensure!(path.is_absolute() && !path.components().any(|c|matches!(c,std::path::Component::ParentDir)) && !value.chars().any(char::is_control),"Choose an absolute folder without parent traversal");
                    Ok(("torrents.move",serde_json::json!({"hashes":[hash],"save_path":value}),"Move complete"))
                },
                PromptKind::TorrentDownloadCap {hash}=>{
                    let value=if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("unlimited"){""}else{value};
                    let bps=settings_fields::parse_rate(value)?;
                    Ok(("torrents.set_download_limit",serde_json::json!({"hashes":[hash],"bps":bps}),"Download cap applied"))
                },
                _=>anyhow::bail!("not an Overview prompt"),
            }
        })();
        match parsed {
            Ok((op,args,label))=>self.service_action(op,args,label),
            Err(error)=>{self.open_prompt(kind,"Check the value · Esc cancels",false);self.prompt.as_mut().unwrap().input.set_value(value);self.set_status(error.to_string(),true);Cmd::none()},
        }
    }
    pub(super) fn overview_cap_text(&self,row:&crate::api::Torrent)->String {
        let global=self.store.settings.as_ref().and_then(|s|if self.turtle_on().unwrap_or(s.alt_limits_on){s.alt_download_limit_bps}else{s.download_limit_bps});
        let effective=[row.download_limit_bps,global].into_iter().flatten().filter(|n|*n>0).min();
        let text=|n:Option<u64>|n.map(|n|format!("{} B/s",n)).unwrap_or_else(||"Unlimited".into());
        format!("{} · effective {}",text(row.download_limit_bps),text(effective))
    }
}
