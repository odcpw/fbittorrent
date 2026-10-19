//! Deliberate Shows intents over the existing three-way-merged local history.
use super::{error,ApiResult};
use crate::{api::*,daemon::Daemon,shows::Shows};
use serde_json::json;
pub(crate) fn failure(e:anyhow::Error)->Error {let mut error=error(ErrorCode::Upstream,&format!("{e:#}"),None);error.details=Some(serde_json::Map::from_iter([("source".into(),json!("eztv"))]));error}
pub(crate) fn require(daemon:&Daemon,id:&str)->ApiResult<Show>{daemon.shows.list().map_err(failure)?.into_iter().find(|s|s.imdb==id).ok_or_else(||{let mut e=error(ErrorCode::NotFound,"show not followed",None);e.details=Some(serde_json::Map::from_iter([("imdb".into(),json!(id))]));e})}
fn episode(all:&Shows,id:&str,key:&str)->ApiResult<crate::shows::Episode>{all.get(id).and_then(|s|s.episode(key)).cloned().ok_or_else(||{let mut e=error(ErrorCode::NotFound,"release not found",None);e.details=Some(serde_json::Map::from_iter([("imdb".into(),json!(id)),("key".into(),json!(key))]));e})}
pub(crate) fn releases(daemon:&Daemon,args:&ShowReleasesArgs)->ApiResult<Vec<Release>>{
    if args.imdb.is_some()==args.inbox.unwrap_or(false){return Err(error(ErrorCode::InvalidArgs,"choose imdb or inbox:true",Some("imdb")));}
    if let Some(id)=&args.imdb{require(daemon,id)?;}
    let transitional=matches!(&daemon.slot.read().unwrap().slot,crate::daemon::slot::Slot::Starting|crate::daemon::slot::Slot::Stopping);
    let rows=if transitional{Vec::new()}else{super::torrents::list(daemon)?};
    daemon.shows.releases(args.imdb.as_deref(),args.unread_only.unwrap_or(args.inbox.unwrap_or(false)),args.quality.as_deref(),&rows).map_err(failure)
}
pub fn dispatch(daemon:&Daemon,call:&Call)->ApiResult<Reply>{
    Ok(match call{
        Call::ShowsList {..}=>Reply::ShowsList(ShowsResult {shows:daemon.shows.list().map_err(failure)?}),
        Call::ShowsSetQuality {args}=>{
            let _edit=daemon.shows.edit.lock().unwrap();require(daemon,&args.imdb)?;
            let mut all=Shows::load().map_err(failure)?;
            all.set_quality(&args.imdb,if let Update::Set(q)=args.quality {Some(q.as_str())} else {None}).map_err(failure)?;
            all.save().map_err(failure)?;
            Reply::ShowsSetQuality(ShowResult {show:require(daemon,&args.imdb)?})
        },
        Call::ShowsFollow {args}=>{
            if args.imdb.is_some()==args.query.is_some(){return Err(error(ErrorCode::InvalidArgs,"choose exactly one of query or imdb",Some("query")));}
            if args.imdb.as_ref().is_some_and(|id|!crate::imdb::service::valid_id(id)){return Err(error(ErrorCode::InvalidArgs,"invalid IMDb id",Some("imdb")));}
            if args.quality.as_ref().is_some_and(|q|q.trim().is_empty()){return Err(error(ErrorCode::InvalidArgs,"empty quality",Some("quality")));}
            let title=daemon.imdb.lookup(args.query.as_deref(),args.imdb.as_deref(),false).map_err(failure)?.ok_or_else(||error(ErrorCode::NotFound,"show not found",Some("query")))?;
            let _edit=daemon.shows.edit.lock().unwrap();let mut all=Shows::load().map_err(failure)?;
            if !all.follow(&title){return Err(error(ErrorCode::InvalidState,"show already followed",Some("imdb")));}
            all.get_mut(&title.id).unwrap().quality=args.quality.clone();all.save().map_err(failure)?;
            Reply::ShowsFollow(ShowResult {show:require(daemon,&title.id)?})
        },
        Call::ShowsUnfollow {args}=>{
            let _edit=daemon.shows.edit.lock().unwrap();require(daemon,&args.imdb)?;let mut all=Shows::load().map_err(failure)?;
            all.unfollow(&args.imdb);all.save().map_err(failure)?;Reply::ShowsUnfollow(EmptyResult {})
        },
        Call::ShowsCheck {args}=>{
            let id=args.as_ref().and_then(|a|a.imdb.as_deref());if let Some(id)=id{require(daemon,id)?;}
            let report=daemon.shows.check(id).map_err(failure)?;
            if report.checked>0&&report.checked==report.warnings.len(){return Err(failure(anyhow::anyhow!("{}",report.warnings.iter().map(|w|w.message.as_str()).collect::<Vec<_>>().join("; "))));}
            Reply::ShowsCheck(ShowCheckResult::Done {new:report.new})
        },
        Call::ShowsReleases {args}=>Reply::ShowsReleases(ReleasesResult {releases:releases(daemon,args)?}),
        Call::ShowsSetRead {args}=>{
            let _edit=daemon.shows.edit.lock().unwrap();let mut all=Shows::load().map_err(failure)?;let e=episode(&all,&args.imdb,&args.key)?;
            let show=all.get_mut(&args.imdb).unwrap();let before=show.seen.contains(&e.key);
            if args.read{show.seen.insert(e.key.clone());}else{show.seen.remove(&e.key);}
            all.save().map_err(failure)?;
            if args.read&&!before{*daemon.shows.undo.lock().unwrap()=Some(ReadTarget {imdb:args.imdb.clone(),key:e.key});}
            Reply::ShowsSetRead(EmptyResult {})
        },
        Call::ShowsMarkShowRead {args}=>{
            let _edit=daemon.shows.edit.lock().unwrap();require(daemon,&args.imdb)?;let mut all=Shows::load().map_err(failure)?;
            let count=all.get_mut(&args.imdb).unwrap().mark_all_seen() as u64;all.save().map_err(failure)?;Reply::ShowsMarkShowRead(CountResult {count})
        },
        Call::ShowsUndoRead {..}=>{
            let _edit=daemon.shows.edit.lock().unwrap();let mut target=daemon.shows.undo.lock().unwrap();let mut all=Shows::load().map_err(failure)?;
            let restored=if let Some(previous)=target.as_ref(){if let Some(show)=all.get_mut(&previous.imdb){show.seen.remove(&previous.key);all.save().map_err(failure)?;target.take()}else{target.take();None}}else{None};
            Reply::ShowsUndoRead(UndoReadResult {restored})
        },
        Call::ShowsGrab {args}=>{
            let _edit=daemon.shows.edit.lock().unwrap();let mut all=Shows::load().map_err(failure)?;let e=episode(&all,&args.imdb,&args.key)?;
            let source=if e.magnet.starts_with("http://")||e.magnet.starts_with("https://"){AddSource::Url(e.magnet.clone())}else{AddSource::Magnet(e.magnet.clone())};
            let add=serde_json::from_value(json!({"source":source,"seed":args.seed,"stopped":args.stopped,"save_path":args.save_path,"files":args.files,"category":args.category,"origin":"shows"})).map_err(|_|error(ErrorCode::InvalidArgs,"invalid add options",None))?;
            let Reply::TorrentsAdd(added)=super::torrents::dispatch(daemon,&Call::TorrentsAdd {args:add})? else{unreachable!()};
            all.get_mut(&args.imdb).unwrap().mark_grabbed(&e.key,&e.magnet);all.save().map_err(failure)?;Reply::ShowsGrab(added)
        },
        _=>return Err(error(ErrorCode::Unsupported,"not a Shows operation",None)),
    })
}
