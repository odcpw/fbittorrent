//! Visible-only peer details. Format rows once per revision/width, not per frame.
use super::*;
use crate::api;

#[derive(Default)]
pub(super) struct PeerTable {
    key: Option<(u64, u64, u16)>,
    rows: Vec<String>,
    header: String,
}
fn compact(value: u64) -> String { format::columns::compact_rate(value) }
fn optional(value: Option<u64>) -> String { value.map_or_else(|| "—".into(),compact) }
fn line(peer: &api::Peer, address_width: usize) -> String {
    let kind=match peer.conn {api::PeerConnection::Tcp=>"tcp",api::PeerConnection::TcpMse=>"MSE"};
    let direction=if peer.incoming {"←"} else {"→"};
    let seed=match peer.seed {Some(true)=>"yes",Some(false)=>"no",None=>"—"};
    format!("{} {} {:<7} {:>6} {:>6} {:>6} {:>6} {:>4}",
        readable_cell(&peer.addr,address_width,None),readable_cell(peer.client.as_deref().unwrap_or("—"),14,None),
        format!("{direction} {kind}"),compact(peer.downloaded),compact(peer.uploaded),optional(peer.dl_bps),optional(peer.ul_bps),seed)
}
impl TuiApp {
    pub(super) fn peer_topic(&self) -> Option<api::Topic> {
        let details=self.details.as_ref().filter(|d|d.tab==DetailsTab::Peers)?;
        if !self.has_cap("peers") {return None;}
        api::Hash::new(details.hash.clone()).ok().map(api::Topic::Peers)
    }
    pub(super) fn scroll_peers(&mut self, down: bool) -> Cmd<Msg> {
        let Some(details)=self.details.as_mut() else {return Cmd::none();};
        let count=api::Hash::new(details.hash.clone()).ok().and_then(|hash|self.store.peers.get(&hash)).map_or(0,|p|p.len());
        details.peer_offset=if down {details.peer_offset.saturating_add(1).min(count.saturating_sub(1))} else {details.peer_offset.saturating_sub(1)};
        Cmd::none()
    }
    pub(super) fn view_peers(&self, area: Rect, frame: &mut Frame) {
        let Some(details)=&self.details else {return;};
        let rows=self.store.peers.iter().find(|(hash,_)|hash.as_str()==details.hash);
        let Some((hash,peers))=rows else {
            Paragraph::new(if self.store.stale {"Peers unavailable · reconnecting…"} else {"Loading peers…"}).style(self.theme.dimmed()).render(area,frame);
            return;
        };
        if peers.is_empty() {
            Paragraph::new("No connected peers.").style(self.theme.dimmed()).render(area,frame);
            return;
        }
        let (sub,rev)=self.store.subscriptions.iter().find(|(_,s)|matches!(&s.topic,api::Topic::Peers(h) if h==hash)).map_or((0,0),|(sub,s)|(*sub,s.rev));
        let mut cache=details.peer_cache.borrow_mut();
        let key=(sub,rev,area.width);
        if cache.key!=Some(key) {
            let address_width=usize::from(area.width).saturating_sub(56).max(1);
            cache.header=format!("{} {:<14} {:<7} {:>6} {:>6} {:>6} {:>6} {:>4}",cell("ADDRESS",address_width),"CLIENT","KIND","DOWN","UP","↓/s","↑/s","SEED");
            cache.rows=peers.values().map(|peer|line(peer,address_width)).collect();
            cache.key=Some(key);
        }
        let th=&self.theme;
        Paragraph::new(cache.header.as_str()).style(th.dimmed().bold()).render(Rect::new(area.x,area.y,area.width,1),frame);
        let offset=details.peer_offset.min(cache.rows.len().saturating_sub(1));
        let visible=area.height.saturating_sub(2) as usize;
        for (index,row) in cache.rows.iter().skip(offset).take(visible).enumerate() {
            Paragraph::new(row.as_str()).style(if self.store.stale {th.dimmed()}else{th.text()}).render(Rect::new(area.x,area.y+1+index as u16,area.width,1),frame);
        }
        if area.height>1 {
            Paragraph::new(format!("{}–{} / {} peers · ↑/↓ scroll",offset+1,(offset+visible).min(cache.rows.len()),cache.rows.len())).style(th.dimmed()).render(Rect::new(area.x,area.bottom()-1,area.width,1),frame);
        }
    }
}
