//! Visible-only tracker snapshots/deltas, with explicit read/reannounce actions.
use super::*;
use crate::api;
use std::sync::atomic::{AtomicU64,Ordering};

static NEXT_READ: AtomicU64=AtomicU64::new(1);
#[derive(Default)]
pub(super) struct TrackerPanel {
    pub rows: Vec<api::Tracker>,
    pub urls:Vec<String>,
    pub busy: bool,
    pub error: String,
    pub request: u64,
    pub offset: usize,
    revision: u64,
    request_revision:u64,
    cache: RefCell<TrackerTable>,
}
#[derive(Default)]
struct TrackerTable {
    key: Option<(u64,u16,i64)>,
    rows: Vec<(String,bool)>,
    header: String,
}
pub(super) fn public_tracker(row:api::Tracker)->api::Tracker {api::trackers::public_tracker(row)}
pub(super) fn public_message(message:&str)->String {api::trackers::public_message(message)}
fn read(connection: &attach::Presence,hash: &str)->Result<Vec<api::Tracker>> {
    let result:api::TrackersResult=serde_json::from_value(connection.request("torrents.trackers",serde_json::json!({"hash":hash}))?)?;
    anyhow::ensure!(result.trackers.len()<=4096,"tracker list exceeds UI row budget");
    Ok(result.trackers)
}
impl TuiApp {
    pub(super) fn load_trackers(&mut self, forced: bool) -> Cmd<Msg> {
        let Some(details)=self.details.as_mut().filter(|d|d.tab==DetailsTab::Trackers) else {return Cmd::none();};
        if details.trackers.busy {return Cmd::none();}
        let request=NEXT_READ.fetch_add(1,Ordering::Relaxed);
        details.trackers.request=request;
        details.trackers.request_revision=details.trackers.revision;
        details.trackers.busy=true;
        details.trackers.error.clear();
        let hash=details.hash.clone();
        let connection=self.presence.clone();
        let identity=connection.as_ref().map(|c|c.id);
        self.set_status(if forced {"Reannouncing…"}else{"Reading trackers…"},false);
        Cmd::task(move || {
            let result=(||->Result<Vec<api::Tracker>> {
                let connection=connection.ok_or_else(||anyhow::anyhow!("fbittorrent stopped"))?;
                if !forced {return read(&connection,&hash);}
                let before=read(&connection,&hash)?;
                connection.request_confirmed("torrents.reannounce",serde_json::json!({"hashes":[hash]}))?;
                // Subsequent status/counters arrive only through the visible topic.
                Ok(before)
            })().map_err(|e|public_message(&e.to_string()));
            Msg::Trackers {connection:identity,hash,request,forced,result}
        })
    }
    pub(super) fn trackers_loaded(&mut self, connection: Option<u64>, hash: String, request: u64, forced: bool, result: Result<Vec<api::Tracker>,String>) -> Cmd<Msg> {
        if self.presence.as_ref().map(|c|c.id)!=connection {return Cmd::none();}
        let Some(panel)=self.details.as_mut().filter(|d|d.hash==hash&&d.tab==DetailsTab::Trackers).map(|d|&mut d.trackers).filter(|p|p.request==request) else {return Cmd::none();};
        panel.busy=false;
        match result {
            Ok(rows)=>{
                if panel.revision==panel.request_revision {
                let mut rows=rows;rows.sort_by(|a,b|a.url.cmp(&b.url));
                let selected=panel.urls.get(panel.offset).cloned();
                panel.urls=rows.iter().map(|row|row.url.clone()).collect();
                panel.offset=selected.and_then(|url|panel.urls.iter().position(|v|v==&url)).unwrap_or(panel.offset).min(rows.len().saturating_sub(1));
                panel.rows=rows.into_iter().map(public_tracker).collect();
                panel.revision=panel.revision.wrapping_add(1);
                }
                if forced {self.set_success("Reannounce requested");}
                else {self.set_status("Trackers refreshed",false);}
            },
            Err(error)=>{panel.error=public_message(&error);self.set_status(public_message(&error),true);},
        }
        Cmd::none()
    }
    pub(super) fn tracker_topic(&self)->Option<api::Topic> {
        self.details.as_ref().filter(|d|d.tab==DetailsTab::Trackers && self.has_cap("trackers"))
            .and_then(|d|api::Hash::new(&d.hash).ok()).map(api::Topic::Trackers)
    }
    pub(super) fn sync_tracker_panel(&mut self) {
        let Some(details)=self.details.as_mut().filter(|d|d.tab==DetailsTab::Trackers)else{return;};
        let Some(rows)=self.store.trackers.iter().find(|(h,_)|h.as_str()==details.hash).map(|(_,rows)|rows)else{return;};
        let urls=rows.keys().cloned().collect::<Vec<_>>();
        let rows=rows.values().cloned().map(public_tracker).collect::<Vec<_>>();
        if details.trackers.rows!=rows || details.trackers.urls!=urls {
            let selected=details.trackers.urls.get(details.trackers.offset).cloned();
            details.trackers.offset=selected.and_then(|url|urls.iter().position(|v|v==&url)).unwrap_or(details.trackers.offset).min(rows.len().saturating_sub(1));
            details.trackers.rows=rows;details.trackers.urls=urls;
            details.trackers.revision=details.trackers.revision.wrapping_add(1);
            details.trackers.offset=details.trackers.offset.min(details.trackers.rows.len().saturating_sub(1));
        }
        details.trackers.error.clear();
    }
    pub(super) fn scroll_trackers(&mut self, down: bool)->Cmd<Msg> {
        if let Some(d)=self.details.as_mut() {
            d.trackers.offset=if down {d.trackers.offset.saturating_add(1).min(d.trackers.rows.len().saturating_sub(1))} else {d.trackers.offset.saturating_sub(1)};
        }
        Cmd::none()
    }
    pub(super) fn view_trackers(&self, area: Rect, frame: &mut Frame) {
        let Some(details)=&self.details else {return;};
        let panel=&details.trackers;
        if !panel.error.is_empty() && panel.rows.is_empty() {
            Paragraph::new(panel.error.as_str()).style(self.theme.err()).wrap(WrapMode::Word).render(area,frame);return;
        }
        if panel.rows.is_empty() {
            Paragraph::new(if panel.busy {"Loading trackers…"}else{"No trackers recorded."}).style(self.theme.dimmed()).render(area,frame);return;
        }
        let mut cache=panel.cache.borrow_mut();
        let key=(panel.revision,area.width,crate::shows::now()/60);
        if cache.key!=Some(key) {
            let url_width=usize::from(area.width).saturating_sub(33).max(1);
            cache.header=format!("{} {:<14} {:>10} {:>6}",cell("URL",url_width),"STATUS","LAST","PEERS");
            cache.rows.clear();
            for row in &panel.rows {
                let status=format::state::tracker_label(&row.status);
                let last=row.last_announce_at.map_or_else(||"—".into(),|at|ago(at.min(i64::MAX as u64) as i64));
                let peers=row.peers.map_or_else(||"—".into(),|n|n.to_string());
                let line=format!("{} {:<14} {:>10} {:>6}",middle_url(&row.url,url_width),status,last,peers);
                cache.rows.push((line,false));
                if row.status==api::TrackerStatus::Error {
                    if let Some(message)=&row.message {cache.rows.push((message_cell(message,area.width as usize),true));}
                }
            }
            cache.key=Some(key);
        }
        Paragraph::new(cache.header.as_str()).style(self.theme.dimmed().bold()).render(Rect::new(area.x,area.y,area.width,1),frame);
        let offset=panel.rows.iter().take(panel.offset).map(|r|1+usize::from(r.status==api::TrackerStatus::Error&&r.message.is_some())).sum::<usize>();
        for (index,(row,dim)) in cache.rows.iter().skip(offset).take(area.height.saturating_sub(2) as usize).enumerate() {
            Paragraph::new(row.as_str()).style(if *dim||self.store.stale {self.theme.dimmed()}else if index==0 {self.theme.accented().bold()}else{self.theme.text()}).render(Rect::new(area.x,area.y+1+index as u16,area.width,1),frame);
        }
    }
}
pub(super) fn middle_url(url: &str,width: usize)->String {
    let value=clean(url);
    if dw(&value)<=width {return cell(&value,width);}
    if width<3 {return cell(&value,width);}
    let tail_budget=(width-1)/2;
    let mut tail_width=0;
    let tail=value.chars().rev().take_while(|ch| {
        let cells=unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
        if tail_width+cells>tail_budget {return false;}
        tail_width+=cells;true
    }).collect::<Vec<_>>().into_iter().rev().collect::<String>();
    let mut prefix_width=0;
    let prefix=value.chars().take_while(|ch| {
        let cells=unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
        if prefix_width+cells>width-1-tail_width {return false;}
        prefix_width+=cells;true
    }).collect::<String>();
    // A wide character can leave one spare cell at the cut. Keep the tail
    // at the right edge rather than padding after the visible URL suffix.
    format!("{prefix}{}…{tail}"," ".repeat(width.saturating_sub(prefix_width+1+tail_width)))
}

pub(super) fn message_cell(message: &str,width: usize)->String {
    let value=clean(message);
    if dw(&value)<=width {return cell(&value,width);}
    let mut used=0;
    let prefix=value.chars().take_while(|ch| {
        let cells=unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
        if used+cells>width.saturating_sub(1) {return false;}
        used+=cells;true
    }).collect::<String>();
    cell(&format!("{prefix}…"),width)
}
