//! One bounded, ephemeral clipboard offer; clipboard contents never enter history.
use super::*;
use crate::api::{self, Event as ApiEvent, Hash};
use serde_json::json;

#[derive(Default)]
pub(in crate::tui) struct ClipboardState {
    pub offer: Option<Offer>,
    last_hash: Option<Hash>,
    last_seq: u64,
}
#[derive(Hash)]
pub(in crate::tui) struct Offer {
    hash: Hash,
    name: String,
}
impl TuiApp {
    pub(in crate::tui) fn clipboard_event(&mut self, event: &ApiEvent) {
        match event {
            ApiEvent::ClipboardMagnet {seq,hash,name,already_added,..} => {
                if *seq <= self.clipboard.last_seq {return;}
                self.clipboard.last_seq=*seq;
                if self.clipboard.last_hash.as_ref()==Some(hash) {return;}
                self.clipboard.last_hash=Some(hash.clone());
                if *already_added || self.store.torrents.contains_key(hash.as_str()) {return;}
                let name=clean(&name.as_deref().unwrap_or("magnet").chars().take(128).collect::<String>());
                self.clipboard.offer=Some(Offer {hash:hash.clone(),name});
            }
            // Historical snapshots never re-offer an already dismissed clipboard.
            ApiEvent::Delta {topic:api::Topic::Events,added:Some(items),..} => {
                for item in items {if let api::Item::Event(event)=item {self.clipboard_event(event);}}
            }
            _=>{}
        }
        if self.clipboard.offer.as_ref().is_some_and(|offer|self.store.torrents.contains_key(offer.hash.as_str())) {
            self.clipboard.offer=None;
        }
    }
    pub(in crate::tui) fn clipboard_visible(&self) -> bool {
        self.clipboard.offer.is_some() && self.online && !self.daemon_stopped && !self.service_busy
            && self.prompt.is_none() && self.settings.is_none() && !self.show_help && !self.log_open
            && self.files.is_none() && self.details.is_none() && self.providers.is_none() && !self.poster_open
            && !(self.screen==Screen::Search && self.search_focus_input)
    }
    pub(in crate::tui) fn clipboard_label(&self, width: usize) -> Option<String> {
        if !self.clipboard_visible() {return None;}
        let offer=self.clipboard.offer.as_ref()?;
        Some(format!("Add '{}'? (y/Esc)",short(&offer.name,width.saturating_sub(dw("Add ''? (y/Esc)")))))
    }
    pub(in crate::tui) fn clipboard_accept(&mut self) -> Cmd<Msg> {
        if !self.clipboard_visible() {return Cmd::none();}
        let offer=self.clipboard.offer.take().unwrap();
        self.service_action("torrents.add",json!({"source":{"magnet":format!("magnet:?xt=urn:btih:{}",offer.hash)},
            "origin":"clipboard","seed":false}),"added from clipboard")
    }
    pub(in crate::tui) fn clipboard_dismiss(&mut self) -> Cmd<Msg> {
        if !self.clipboard_visible() {return Cmd::none();}
        let offer=self.clipboard.offer.take().unwrap();
        self.service_action("app.clipboard.dismiss",json!({"hash":offer.hash}),"clipboard offer dismissed")
    }
}
