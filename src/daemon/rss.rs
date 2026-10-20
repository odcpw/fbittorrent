//! One refresh scheduler, shared cached views, and the existing atomic RSS store.
use crate::{api::{Feed, FeedItem}, rss::Store};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::MetadataExt,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};
type Wake = Arc<dyn Fn()+Send+Sync>;
#[derive(Default)]
pub(crate) struct Snapshot { pub feeds:Vec<Feed>, pub items:BTreeMap<String,Vec<FeedItem>> }
type Stamp = (u64, u64, i64, i64);
#[derive(Default)]
// Failed loads are cached by stamp, like successful ones; see shows::Cache.
struct Cache { stamp:Option<Stamp>, snapshot:Arc<Snapshot>, loaded:bool, failure:Option<(Option<Stamp>,String)> }
#[derive(Default)]
struct State { stopped:bool, requested:bool, clients:BTreeSet<u64>, loading:BTreeSet<String> }
#[derive(Default)]
struct Shared { state:Mutex<State>, changed:Condvar, refresh:Mutex<()>, wake:Mutex<Option<Wake>> }
pub(crate) struct Service { shared:Arc<Shared>, cache:Mutex<Cache>, worker:Mutex<Option<JoinHandle<()>>> }
fn nonempty(value:&str)->Option<String> { (!value.is_empty()).then(||value.into()) }
fn now()->i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
fn stamp()->Option<Stamp> {
    std::fs::metadata(crate::rss::path()).ok().map(|m|(m.ino(),m.len(),m.mtime(),m.mtime_nsec()))
}
fn wake(shared:&Shared) { if let Some(wake)=shared.wake.lock().unwrap().as_ref() { wake(); } }
fn refresh(shared:&Shared, feed:&str, timeout:u64)->Result<()> {
    let _transaction=shared.refresh.lock().unwrap();
    { let mut state=shared.state.lock().unwrap(); if state.stopped { anyhow::bail!("RSS owner stopping"); } state.loading.insert(feed.into()); }
    wake(shared);
    let result=Store::default().refresh(feed,timeout);
    shared.state.lock().unwrap().loading.remove(feed); wake(shared);
    result
}
impl Service {
    pub fn new()->std::io::Result<Self> {
        let shared=Arc::new(Shared::default()); let owner=shared.clone();
        let worker=thread::Builder::new().name("fbt-rss".into()).spawn(move || {
            loop {
                let mut state=owner.state.lock().unwrap();
                while !state.requested && !state.stopped { state=owner.changed.wait(state).unwrap(); }
                if state.stopped { return; } state.requested=false; drop(state);
                if let Ok(rows)=Store::default().snapshot() {
                    for row in rows {
                        {let state=owner.state.lock().unwrap();if state.stopped {return;} if state.clients.is_empty(){break;}}
                        if now().saturating_sub(row.last_checked) < 1800 { continue; }
                        // Re-check after another explicit refresh obtained the
                        // store transaction; two opens must not fetch twice.
                        let _transaction=owner.refresh.lock().unwrap();
                        {let state=owner.state.lock().unwrap();if state.stopped {return;} if state.clients.is_empty(){break;}}
                        let stale=Store::default().snapshot().ok().is_some_and(|rows|rows.iter().any(|r|r.feed.uid==row.feed.uid && now().saturating_sub(r.last_checked)>=1800));
                        if !stale { continue; }
                        owner.state.lock().unwrap().loading.insert(row.feed.name.clone()); wake(&owner);
                        if Store::default().refresh(&row.feed.name,10).is_err() { eprintln!("WARN rss interactive refresh failed"); }
                        owner.state.lock().unwrap().loading.remove(&row.feed.name); wake(&owner);
                    }
                }
            }
        })?;
        Ok(Self { shared, cache:Mutex::default(), worker:Mutex::new(Some(worker)) })
    }
    pub fn set_waker(&self, waker:Wake) { *self.shared.wake.lock().unwrap()=Some(waker); }
    pub fn presence(&self, id:u64, interactive:bool) {
        let mut state=self.shared.state.lock().unwrap();
        if interactive && state.clients.insert(id) { state.requested=true; self.shared.changed.notify_one(); }
        else if !interactive { state.clients.remove(&id); }
    }
    pub fn snapshot(&self)->Result<Arc<Snapshot>> {
        let mut cache=self.cache.lock().unwrap();
        let next=stamp();
        // Load between two metadata observations; an atomic CLI rename that
        // crosses the read will be observed on the next sample, never cached
        // under the newer inode's stamp with older contents.
        if let Some((failed,message))=&cache.failure { if *failed==next { anyhow::bail!("{message}"); } }
        if next != cache.stamp || !cache.loaded {
            let rows=match Store::default().snapshot() {
                Ok(rows)=>rows,
                Err(error)=>{
                    let message=format!("{error:#}");
                    eprintln!("WARN rss store unreadable message={message:?}");
                    cache.failure=Some((next,message)); return Err(error);
                }
            };
            cache.failure=None;
            let mut snapshot=Snapshot::default();
            for row in rows {
                let f=row.feed; let id=if f.uid.is_empty() {crate::rss::stable_id(&f.url)}else{f.uid};
                let items:Vec<_>=f.articles.iter().map(|a| {
                    let source=if !a.torrent_url.is_empty(){&a.torrent_url}else{&a.link};
                    FeedItem { id:a.id.clone(), title:a.title.clone(), description:nonempty(&a.description), published_at:(a.ts>0).then_some(a.ts as u64),
                        size:a.size.and_then(|v|u64::try_from(v).ok()), link:nonempty(&a.link),
                        magnet:source.starts_with("magnet:?").then(||source.clone()), torrent_url:(!a.torrent_url.is_empty()&&(source.starts_with("http://")||source.starts_with("https://"))).then(||source.clone()),
                        read:a.is_read, grabbed:row.grabbed.contains(&a.id) }
                }).collect();
                snapshot.feeds.push(Feed { id:id.clone(),name:f.name,url:f.url,title:nonempty(&f.title),unread:items.iter().filter(|a|!a.read).count() as u64,
                    last_refresh_at:(row.last_checked>0).then_some(row.last_checked as u64),loading:false,error:nonempty(&row.error) });
                snapshot.items.insert(id,items);
            }
            snapshot.feeds.sort_by(|a,b|a.id.cmp(&b.id));
            cache.snapshot=Arc::new(snapshot); cache.stamp=next; cache.loaded=true;
        }
        Ok(cache.snapshot.clone())
    }
    pub fn feeds(&self)->Result<Vec<Feed>> {
        let mut feeds=self.snapshot()?.feeds.clone(); let state=self.shared.state.lock().unwrap();
        for feed in &mut feeds { feed.loading=state.loading.contains("")||state.loading.contains(&feed.name); }
        Ok(feeds)
    }
    pub fn refresh(&self, name:&str, timeout:u64)->Result<()> {
        let names:Vec<_>=if name.is_empty(){self.snapshot()?.feeds.iter().map(|f|f.name.clone()).collect()}else{vec![name.into()]};
        let mut errors=Vec::new();
        for name in names {
            if self.shared.state.lock().unwrap().stopped {anyhow::bail!("RSS owner stopping");}
            if let Err(error)=refresh(&self.shared,&name,timeout.min(10)){errors.push(format!("{error:#}"));}
        }
        if !errors.is_empty(){anyhow::bail!("{}",errors.join("; "));} Ok(())
    }
    pub fn stamp(&self)->Option<Stamp> { stamp() }
    /// The recorded load failure while the same broken file remains in place.
    pub fn problem(&self)->Option<String> {
        let cache=self.cache.lock().unwrap();
        cache.failure.as_ref().filter(|(failed,_)|*failed==stamp()).map(|(_,message)|message.clone())
    }
    pub fn stop(&self) {
        self.shared.state.lock().unwrap().stopped=true; self.shared.changed.notify_one();
        if let Some(worker)=self.worker.lock().unwrap().take(){let _=worker.join();}
    }
}
impl Drop for Service { fn drop(&mut self){self.stop();} }
