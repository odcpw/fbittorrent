//! On-demand tracker reads; explicit reannounce has a bounded follow-up read.
use super::*;
use crate::api;
use std::{sync::atomic::{AtomicU64,Ordering}, time::Instant};

static NEXT_READ: AtomicU64=AtomicU64::new(1);
#[derive(Default)]
pub(super) struct TrackerPanel {
    pub rows: Vec<api::Tracker>,
    pub busy: bool,
    pub error: String,
    pub request: u64,
    pub offset: usize,
    revision: u64,
    cache: RefCell<TrackerTable>,
}
#[derive(Default)]
struct TrackerTable {
    key: Option<(u64,u16,i64)>,
    rows: Vec<(String,bool)>,
    header: String,
}
/// The mirror never retains the private query. Also remove echoed passkeys.
pub(super) fn public_tracker(mut row: api::Tracker) -> api::Tracker {
    if let Some(message)=&mut row.message {
        if let Ok(url)=url::Url::parse(&row.url) {
            for (_,value) in url.query_pairs() {
                if !value.is_empty() {*message=message.replace(value.as_ref(),"…");}
            }
        }
        *message=public_message(message);
    }
    row.url=row.url.split(['?','#']).next().unwrap_or_default().to_owned();
    row
}
pub(super) fn public_message(message: &str) -> String {
    message.split_whitespace().map(|word| {
        if word.contains("://") {word.split(['?','#']).next().unwrap_or_default()} else {word}
    }).collect::<Vec<_>>().join(" ").chars().take(200).collect()
}
fn read(connection: &attach::Presence,hash: &str)->Result<Vec<api::Tracker>> {
    let result:api::TrackersResult=serde_json::from_value(connection.request("torrents.trackers",serde_json::json!({"hash":hash}))?)?;
    anyhow::ensure!(result.trackers.len()<=4096,"tracker list exceeds UI row budget");
    Ok(result.trackers.into_iter().map(public_tracker).collect())
}
impl TuiApp {
    pub(super) fn load_trackers(&mut self, forced: bool) -> Cmd<Msg> {
        let Some(details)=self.details.as_mut().filter(|d|d.tab==DetailsTab::Trackers) else {return Cmd::none();};
        if details.trackers.busy {return Cmd::none();}
        let request=NEXT_READ.fetch_add(1,Ordering::Relaxed);
        details.trackers.request=request;
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
                let deadline=Instant::now()+Duration::from_secs(2);
                loop {
                    let rows=read(&connection,&hash)?;
                    if (rows!=before && rows.iter().all(|r|r.status!=api::TrackerStatus::Updating)) || Instant::now()>=deadline {return Ok(rows);}
                    std::thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())));
                }
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
                panel.rows=rows.into_iter().map(public_tracker).collect();
                panel.rows.sort_by(|a,b|a.url.cmp(&b.url));
                panel.revision=panel.revision.wrapping_add(1);
                if forced {self.set_success("Reannounce requested");}
                else {self.set_status("Trackers refreshed",false);}
            },
            Err(error)=>{panel.error=public_message(&error);self.set_status(public_message(&error),true);},
        }
        Cmd::none()
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
        if !panel.error.is_empty() {
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
            Paragraph::new(row.as_str()).style(if *dim||self.store.stale {self.theme.dimmed()}else{self.theme.text()}).render(Rect::new(area.x,area.y+1+index as u16,area.width,1),frame);
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
