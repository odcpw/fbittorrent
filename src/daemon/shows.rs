//! Shows owner: one stale-on-open worker, atomic shared history and one undo.
use crate::{api,shows::{self,Shows}};
use std::{collections::BTreeSet,os::unix::fs::MetadataExt,sync::{Arc,Weak,Mutex,Condvar,atomic::{AtomicBool,Ordering}},thread::{self,JoinHandle}};
use anyhow::Result;
use serde_json::json;
pub(crate) type Stamp=(u64,u64,i64,i64);
#[derive(Default)] struct Cache {stamp:Option<Stamp>,loaded:bool,rows:Arc<Vec<shows::Show>>}
#[derive(Default)] struct State {requested:bool,clients:BTreeSet<u64>,checking:BTreeSet<String>}
#[derive(Default)] struct Shared {state:Mutex<State>,changed:Condvar,check:Mutex<()>,stopped:AtomicBool,bus:Mutex<Weak<api::bus::Bus>>}
pub(crate) struct Service {
    shared:Arc<Shared>,cache:Mutex<Cache>,worker:Mutex<Option<JoinHandle<()>>>,
    pub edit:Mutex<()>,pub undo:Mutex<Option<api::ReadTarget>>,
}
fn wake(shared:&Shared){if let Some(bus)=shared.bus.lock().unwrap().upgrade(){bus.refresh();}}
pub(crate) struct Report {pub new:u64,pub warnings:Vec<api::ShowWarning>,pub checked:usize}
fn check(shared:&Shared,selector:Option<&str>,stale_only:bool)->Result<Report> {
    let _checking=shared.check.lock().unwrap();
    let mut all=Shows::load()?;let mut report=Report {new:0,warnings:Vec::new(),checked:0};
    if let Some(id)=selector {anyhow::ensure!(all.get(id).is_some(),"show not found");}
    let cfg=crate::config::Config::load()?;
    for show in all.shows.iter_mut().filter(|s|selector.is_none_or(|id|id==s.imdb)) {
        if shared.stopped.load(Ordering::Acquire){break;}
        if stale_only && (!show.is_stale(shows::now())||shared.state.lock().unwrap().clients.is_empty()){continue;}
        report.checked+=1;shared.state.lock().unwrap().checking.insert(show.imdb.clone());wake(shared);
        match shows::fetch_updates_cancelled(show,&cfg.show_quality,&shared.stopped) {
            Ok(check)=>{report.new+=show.apply_check(check) as u64;},
            Err(error)=>{let message=format!("{error:#}");show.error=Some(message.clone());report.warnings.push(api::ShowWarning {imdb:show.imdb.clone(),message});},
        }
        shared.state.lock().unwrap().checking.remove(&show.imdb);wake(shared);
    }
    if stale_only&&report.checked==0{return Ok(report);}
    all.save()?;
    if let Some(bus)=shared.bus.lock().unwrap().upgrade(){
        let mut event=json!({"ev":"shows.checked","new":report.new,"warnings":report.warnings});
        if let Some(id)=selector {event["imdb"]=json!(id);}
        bus.publish(event);
    }
    wake(shared);Ok(report)
}
impl Service {
    pub fn new()->std::io::Result<Self>{
        let shared=Arc::new(Shared::default());let owner=shared.clone();
        let worker=thread::Builder::new().name("fbt-shows".into()).spawn(move ||loop {
            let mut state=owner.state.lock().unwrap();
            while !state.requested&&!owner.stopped.load(Ordering::Acquire){state=owner.changed.wait(state).unwrap();}
            if owner.stopped.load(Ordering::Acquire){return;} state.requested=false;drop(state);
            if check(&owner,None,true).is_err(){eprintln!("WARN shows interactive check failed");}
        })?;
        Ok(Self {shared,cache:Mutex::default(),worker:Mutex::new(Some(worker)),edit:Mutex::new(()),undo:Mutex::new(None)})
    }
    pub fn set_bus(&self,bus:Weak<api::bus::Bus>){*self.shared.bus.lock().unwrap()=bus;}
    pub fn presence(&self,id:u64,interactive:bool){
        let mut state=self.shared.state.lock().unwrap();
        if interactive&&state.clients.insert(id){state.requested=true;self.shared.changed.notify_one();}
        else if !interactive{state.clients.remove(&id);}
    }
    pub fn stamp(&self)->Option<Stamp>{std::fs::metadata(shows::path()).ok().map(|m|(m.ino(),m.len(),m.mtime(),m.mtime_nsec()))}
    pub fn snapshot(&self)->Result<Arc<Vec<shows::Show>>>{
        let mut cache=self.cache.lock().unwrap();let stamp=self.stamp();
        if !cache.loaded||cache.stamp!=stamp{cache.rows=Arc::new(Shows::load()?.shows);cache.stamp=stamp;cache.loaded=true;}
        Ok(cache.rows.clone())
    }
    pub fn list(&self)->Result<Vec<api::Show>>{
        let rows=self.snapshot()?;let state=self.shared.state.lock().unwrap();
        let mut result:Vec<_>=rows.iter().map(|s|api::Show {imdb:s.imdb.clone(),title:s.title.clone(),year:s.year.map(u64::from),quality:s.quality.clone(),unread:s.new_count() as u64,
            added_at:s.added.max(0) as u64,counts:api::ShowCounts {episodes:s.episodes.len() as u64,seen:s.seen.len() as u64,grabbed:s.grabbed.len() as u64},
            last_check_at:(s.last_checked>0).then_some(s.last_checked as u64),checking:state.checking.contains(&s.imdb),catalogue_complete:s.catalogue_complete,warning:s.error.clone()}).collect();
        result.sort_by(|a,b|a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.imdb.cmp(&b.imdb)));Ok(result)
    }
    pub fn releases(&self,selector:Option<&str>,unread:bool,quality:Option<&str>,rows:&[api::Torrent])->Result<Vec<api::Release>>{
        let all=self.snapshot()?;let mut releases=Vec::new();
        for show in all.iter().filter(|s|selector.is_none_or(|id|id==s.imdb)){
            for e in show.episodes.iter().filter(|e|(!unread||show.is_new(&e.key))&&quality.is_none_or(|q|q==e.quality)){
                let hash=librqbit::Magnet::parse(show.grabbed.get(&e.key).unwrap_or(&e.magnet)).ok().and_then(|m|m.as_id20()).map(|id|id.as_string());
                let transfer=if let Some(row)=rows.iter().find(|r|!r.staged && Some(r.hash.as_str())==hash.as_deref()){
                    if row.intent==api::Intent::Pause{api::ReleaseTransfer::Paused}else{match row.state {api::TorrentState::Downloading|api::TorrentState::Stalled=>api::ReleaseTransfer::Downloading,
                        api::TorrentState::Paused=>api::ReleaseTransfer::Paused,api::TorrentState::Done|api::TorrentState::Seeding=>api::ReleaseTransfer::Done,
                        api::TorrentState::Error=>api::ReleaseTransfer::AddedBefore,_=>api::ReleaseTransfer::Queued}}
                }else if show.grabbed.contains_key(&e.key){api::ReleaseTransfer::AddedBefore}else{api::ReleaseTransfer::None};
                releases.push(api::Release {key:e.key.clone(),show_imdb:show.imdb.clone(),title:e.title.clone(),season:Some(u64::from(e.season)),episode:(!e.is_season_pack()).then_some(u64::from(e.episode)),season_pack:e.is_season_pack(),
                    quality:(!e.quality.is_empty()).then(||e.quality.clone()),seeds:(e.seeds>0).then_some(e.seeds as u64),size:(e.size>0).then_some(e.size as u64),magnet:(!e.magnet.is_empty()).then(||e.magnet.clone()),
                    candidates:Some(e.candidates as u64),published_at:(e.released>0).then_some(e.released as u64),read:show.seen.contains(&e.key),transfer});
            }
        }
        releases.sort_by(|a,b|b.published_at.cmp(&a.published_at).then(a.key.cmp(&b.key)).then(a.show_imdb.cmp(&b.show_imdb)));Ok(releases)
    }
    pub fn check(&self,id:Option<&str>)->Result<Report>{check(&self.shared,id,false)}
    pub fn stop(&self){self.shared.stopped.store(true,Ordering::Release);self.shared.changed.notify_one();if let Some(worker)=self.worker.lock().unwrap().take(){let _=worker.join();}}
}
impl Drop for Service{fn drop(&mut self){self.stop();}}
