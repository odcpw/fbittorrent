//! A staged review owns no read/grab mark until its torrent is committed.
use crate::{api::{self, ops::{ApiResult,error}, *}, daemon::Daemon, shows::Shows};
use std::{collections::BTreeMap,sync::Mutex};

#[derive(Clone)]
pub(crate) enum Source {
    Show {imdb:String,key:String,magnet:String},
    Rss {feed:String,item:String},
}
struct Pending {source:Source,committed:bool}
#[derive(Default)]
pub(crate) struct Registry {rows:Mutex<BTreeMap<Hash,Pending>>}

impl Source {
    fn mark(&self,daemon:&Daemon)->ApiResult<()> {
        match self {
            Self::Show {imdb,key,magnet}=>{
                let _edit=daemon.shows.edit.lock().unwrap();
                let mut all=Shows::load().map_err(api::ops::shows::failure)?;
                let show=all.get_mut(imdb).ok_or_else(||error(ErrorCode::NotFound,"show was unfollowed during review",Some("imdb")))?;
                show.mark_grabbed(key,magnet);all.save().map_err(api::ops::shows::failure)
            }
            Self::Rss {feed,item}=>{
                let feed=api::ops::rss::feed(daemon,feed)?;
                crate::rss::Store::default().mark_grabbed(&feed.name,item).map_err(api::ops::rss::failure)
            }
        }
    }
}

/// Used by both shows.grab and rss.grab. The short-lived association survives
/// engine-slot restarts; daemon restart discards the corresponding staged rows.
pub(crate) fn add(daemon:&Daemon,args:AddArgs,mut source:Source)->ApiResult<AddResult> {
    let staged=args.stage==Some(true);
    // Serialize publication with commit/discard so another client cannot commit
    // the newly visible hash before its source association has been installed.
    let mut pending=daemon.staged_sources.rows.lock().unwrap();
    if staged && pending.len()>=4096 {return Err(error(ErrorCode::Busy,"too many staged source reviews",None));}
    let Reply::TorrentsAdd(added)=api::ops::torrents::dispatch(daemon,&Call::TorrentsAdd {args})? else{unreachable!()};
    if let (Source::Show {magnet,..},Some(hash))=(&mut source,&added.hash) {*magnet=format!("magnet:?xt=urn:btih:{hash}");}
    if staged {
        if added.duplicate!=Some(true) {
            let hash=added.hash.clone().ok_or_else(||error(ErrorCode::Internal,"staged add omitted hash",None))?;
            pending.insert(hash,Pending {source,committed:false});
        }
    } else {source.mark(daemon)?;}
    Ok(added)
}

/// A source-save failure stays retryable with a new mutation ref: the engine
/// commit is recorded here first, so retrying never replays the transfer intent.
pub(crate) fn commit(daemon:&Daemon,hash:&Hash,apply:impl FnOnce()->ApiResult<Reply>)->ApiResult<Reply> {
    let mut pending=daemon.staged_sources.rows.lock().unwrap();
    if !pending.get(hash).is_some_and(|row|row.committed) {
        apply()?;
        if let Some(row)=pending.get_mut(hash){row.committed=true;}
    }
    if let Some(row)=pending.get(hash){row.source.mark(daemon)?;}
    pending.remove(hash);
    Ok(Reply::TorrentsCommit(DoneResult {state:DoneState::Done}))
}
pub(crate) fn discard(daemon:&Daemon,hash:&Hash,apply:impl FnOnce()->ApiResult<Reply>)->ApiResult<Reply> {
    let mut pending=daemon.staged_sources.rows.lock().unwrap();let result=apply()?;pending.remove(hash);Ok(result)
}
