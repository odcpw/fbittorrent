//! Tracker edits use private URL identities; prompts and receipts never echo them.
use super::*;
impl TuiApp {
    pub(super) fn can_edit_trackers(&self)->bool {
        self.online && !self.service_busy && self.has_cap("tracker_edit")
            && self.details.as_ref().is_some_and(|d|d.tab==DetailsTab::Trackers && !d.trackers.busy)
    }
    pub(super) fn prompt_add_tracker(&mut self)->Cmd<Msg> {
        let Some(hash)=self.details.as_ref().filter(|_|self.can_edit_trackers()).map(|d|d.hash.clone())else{return Cmd::none();};
        self.open_prompt(PromptKind::AddTracker{hash},"Add tracker · http(s) or udp URL · input hidden",false);
        Cmd::none()
    }
    pub(super) fn prompt_remove_tracker(&mut self)->Cmd<Msg> {
        let Some((hash,url))=self.details.as_ref().filter(|_|self.can_edit_trackers()).and_then(|d|d.trackers.urls.get(d.trackers.offset).map(|url|(d.hash.clone(),url.clone())))else{return Cmd::none();};
        let label=format!("Remove tracker {}?",crate::api::trackers::public_url(&url));
        self.open_prompt(PromptKind::ConfirmRemoveTracker{hash,url},&label,true);
        Cmd::none()
    }
    pub(super) fn edit_tracker(&mut self,hash:&str,value:&str,add:bool)->Cmd<Msg> {
        let valid=value.len()<=(if add {4096}else{8192}) && !value.chars().any(char::is_control)
            && url::Url::parse(value).is_ok_and(|url|url.host_str().is_some()
                && ((!add&&crate::api::trackers::is_alias(&url))||matches!(url.scheme(),"http"|"https"|"udp")) && (url.scheme()!="udp"||url.port().is_some()));
        if !valid {self.set_status("Invalid tracker URL · use http(s) or udp with a port",true);return Cmd::none();}
        self.service_action(if add {"torrents.add_tracker"}else{"torrents.remove_tracker"},
            serde_json::json!({"hash":hash,"url":value}),if add {"Tracker added"}else{"Tracker removed"})
    }
}
