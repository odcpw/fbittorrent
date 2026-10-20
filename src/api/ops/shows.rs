//! Deliberate Shows intents over the existing three-way-merged local history.
use super::{error,ApiResult};
use crate::{api::*,daemon::Daemon,shows::Shows};
use serde_json::json;
pub(crate) fn failure(e:anyhow::Error)->Error {let mut error=error(ErrorCode::Upstream,&format!("{e:#}"),None);error.details=Some(serde_json::Map::from_iter([("source".into(),json!("eztv"))]));error}
pub(crate) fn require(daemon:&Daemon,id:&str)->ApiResult<Show>{require_in(&daemon.shows,id)}
fn require_in(service:&crate::daemon::shows::Service,id:&str)->ApiResult<Show>{service.list().map_err(failure)?.into_iter().find(|s|s.imdb==id).ok_or_else(||{let mut e=error(ErrorCode::NotFound,"show not followed",None);e.details=Some(serde_json::Map::from_iter([("imdb".into(),json!(id))]));e})}
fn episode(all:&Shows,id:&str,key:&str)->ApiResult<crate::shows::Episode>{all.get(id).and_then(|s|s.episode(key)).cloned().ok_or_else(||{let mut e=error(ErrorCode::NotFound,"release not found",None);e.details=Some(serde_json::Map::from_iter([("imdb".into(),json!(id)),("key".into(),json!(key))]));e})}
pub(crate) fn releases(daemon:&Daemon,args:&ShowReleasesArgs)->ApiResult<Vec<Release>>{
    if args.imdb.is_some()==args.inbox.unwrap_or(false){return Err(error(ErrorCode::InvalidArgs,"choose imdb or inbox:true",Some("imdb")));}
    if let Some(id)=&args.imdb{require(daemon,id)?;}
    let transitional=matches!(&daemon.slot.read().unwrap().slot,crate::daemon::slot::Slot::Starting|crate::daemon::slot::Slot::Stopping);
    let rows=if transitional{Vec::new()}else{super::torrents::list(daemon)?};
    daemon.shows.releases(args.imdb.as_deref(),args.unread_only.unwrap_or(args.inbox.unwrap_or(false)),args.quality.as_deref(),&rows).map_err(failure)
}
pub fn dispatch(daemon:&Daemon,call:&Call)->ApiResult<Reply>{
    dispatch_in(&daemon.shows,&daemon.imdb,call,|args|releases(daemon,args),|args,source|{
        crate::daemon::staged_sources::add(daemon,args,source)
    })
}
pub(crate) fn catalogue(service:&crate::daemon::shows::Service,imdb:&crate::imdb::service::Service,rows:&[Torrent],call:&Call)->ApiResult<Reply>{
    dispatch_in(service,imdb,call,|args|{
        if args.imdb.is_some()==args.inbox.unwrap_or(false){return Err(error(ErrorCode::InvalidArgs,"choose imdb or inbox:true",Some("imdb")));}
        if let Some(id)=&args.imdb{require_in(service,id)?;}
        service.releases(args.imdb.as_deref(),args.unread_only.unwrap_or(args.inbox.unwrap_or(false)),args.quality.as_deref(),rows).map_err(failure)
    },|_,_|Err(error(ErrorCode::EngineUnavailable,"grab requires the daemon",None)))
}
fn dispatch_in(service:&crate::daemon::shows::Service,imdb:&crate::imdb::service::Service,call:&Call,
    release_list:impl FnOnce(&ShowReleasesArgs)->ApiResult<Vec<Release>>,
    add_torrent:impl FnOnce(AddArgs,crate::daemon::staged_sources::Source)->ApiResult<AddResult>)->ApiResult<Reply>{
    Ok(match call{
        Call::ShowsList {..}=>Reply::ShowsList(ShowsResult {shows:service.list().map_err(failure)?}),
        Call::ShowsSetQuality {args}=>{
            let _edit=service.edit.lock().unwrap();require_in(service,&args.imdb)?;
            let mut all=Shows::load().map_err(failure)?;
            all.set_quality(&args.imdb,if let Update::Set(q)=args.quality {Some(q.as_str())} else {None}).map_err(failure)?;
            all.save().map_err(failure)?;
            Reply::ShowsSetQuality(ShowResult {show:require_in(service,&args.imdb)?})
        },
        Call::ShowsFollow {args}=>{
            if args.imdb.is_some()==args.query.is_some(){return Err(error(ErrorCode::InvalidArgs,"choose exactly one of query or imdb",Some("query")));}
            if args.imdb.as_ref().is_some_and(|id|!crate::imdb::service::valid_id(id)){return Err(error(ErrorCode::InvalidArgs,"invalid IMDb id",Some("imdb")));}
            if args.quality.as_ref().is_some_and(|q|q.trim().is_empty()){return Err(error(ErrorCode::InvalidArgs,"empty quality",Some("quality")));}
            let title=imdb.lookup(args.query.as_deref(),args.imdb.as_deref(),false).map_err(failure)?.ok_or_else(||error(ErrorCode::NotFound,"show not found",Some("query")))?;
            let _edit=service.edit.lock().unwrap();let mut all=Shows::load().map_err(failure)?;
            if !all.follow(&title){return Err(error(ErrorCode::InvalidState,"show already followed",Some("imdb")));}
            all.get_mut(&title.id).unwrap().quality=args.quality.clone();all.save().map_err(failure)?;
            Reply::ShowsFollow(ShowResult {show:require_in(service,&title.id)?})
        },
        Call::ShowsUnfollow {args}=>{
            let _edit=service.edit.lock().unwrap();require_in(service,&args.imdb)?;let mut all=Shows::load().map_err(failure)?;
            all.unfollow(&args.imdb);all.save().map_err(failure)?;Reply::ShowsUnfollow(EmptyResult {})
        },
        Call::ShowsCheck {args}=>{
            let id=args.as_ref().and_then(|a|a.imdb.as_deref());if let Some(id)=id{require_in(service,id)?;}
            let report=service.check(id).map_err(failure)?;
            if report.checked>0&&report.checked==report.warnings.len(){return Err(failure(anyhow::anyhow!("{}",report.warnings.iter().map(|w|w.message.as_str()).collect::<Vec<_>>().join("; "))));}
            Reply::ShowsCheck(ShowCheckResult::Done {new:report.new})
        },
        Call::ShowsReleases {args}=>Reply::ShowsReleases(ReleasesResult {releases:release_list(args)?}),
        Call::ShowsSetRead {args}=>{
            let _edit=service.edit.lock().unwrap();let mut all=Shows::load().map_err(failure)?;let e=episode(&all,&args.imdb,&args.key)?;
            let show=all.get_mut(&args.imdb).unwrap();let before=show.seen.contains(&e.key);
            if args.read{show.seen.insert(e.key.clone());}else{show.seen.remove(&e.key);}
            all.save().map_err(failure)?;
            if args.read&&!before{*service.undo.lock().unwrap()=Some(ReadTarget {imdb:args.imdb.clone(),key:e.key});}
            Reply::ShowsSetRead(EmptyResult {})
        },
        Call::ShowsMarkShowRead {args}=>{
            let _edit=service.edit.lock().unwrap();require_in(service,&args.imdb)?;let mut all=Shows::load().map_err(failure)?;
            let count=all.get_mut(&args.imdb).unwrap().mark_all_seen() as u64;all.save().map_err(failure)?;Reply::ShowsMarkShowRead(CountResult {count})
        },
        Call::ShowsUndoRead {..}=>{
            let _edit=service.edit.lock().unwrap();let mut target=service.undo.lock().unwrap();let mut all=Shows::load().map_err(failure)?;
            let restored=if let Some(previous)=target.as_ref(){if let Some(show)=all.get_mut(&previous.imdb){show.seen.remove(&previous.key);all.save().map_err(failure)?;target.take()}else{target.take();None}}else{None};
            Reply::ShowsUndoRead(UndoReadResult {restored})
        },
        Call::ShowsGrab {args}=>{
            let e={let _edit=service.edit.lock().unwrap();episode(&Shows::load().map_err(failure)?,&args.imdb,&args.key)?};
            let source=if e.magnet.starts_with("http://")||e.magnet.starts_with("https://"){AddSource::Url(e.magnet.clone())}else{AddSource::Magnet(e.magnet.clone())};
            let add = AddArgs {
                source,
                stage: args.stage,
                stopped: args.stopped,
                seed: args.seed,
                save_path: args.save_path.clone(),
                files: args.files.clone(),
                category: args.category.clone(),
                origin: Some("shows".into()),
            };
            let source=crate::daemon::staged_sources::Source::Show {imdb:args.imdb.clone(),key:e.key,magnet:e.magnet};
            Reply::ShowsGrab(add_torrent(add,source)?)
        },
        _=>return Err(error(ErrorCode::Unsupported,"not a Shows operation",None)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_grab_preserves_stage_and_releases_edit_lock() {
        const CHILD: &str = "FBT_SHARED_SHOW_GRAB_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root = std::env::temp_dir().join(format!("fbt-show-grab-{}-{}",
                std::process::id(), std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            std::fs::create_dir(&root).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "api::ops::shows::tests::shared_grab_preserves_stage_and_releases_edit_lock", "--nocapture"])
                .env(CHILD, "1").env("XDG_DATA_HOME", &root).output().unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(output.status.success(), "{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        let mut saved = Shows::default();
        saved.follow(&crate::imdb::Title { id:"tt1234001".into(), name:"Merge fixture".into(), ..Default::default() });
        saved.get_mut("tt1234001").unwrap().episodes.push(serde_json::from_value(json!({
            "key":"S01E01", "season":1, "episode":1, "title":"Merge fixture S01E01",
            "magnet":format!("magnet:?xt=urn:btih:{}", "ab".repeat(20)), "seeds":1,
            "size":1024, "released":0, "quality":"1080p", "candidates":1
        })).unwrap());
        saved.save().unwrap();
        let service = crate::daemon::shows::Service::new().unwrap();
        let imdb = crate::imdb::service::Service::default();
        for staged in [false, true] {
            let args: ShowGrabArgs = serde_json::from_value(json!({
                "imdb":"tt1234001", "key":"S01E01", "stage":staged
            })).unwrap();
            let call = Call::ShowsGrab { args };
            let reply = dispatch_in(&service, &imdb, &call, |_| panic!("not a read"), |args, source| {
                assert_eq!(args.stage, Some(staged));
                assert!(service.edit.try_lock().is_ok(), "source admission must not hold the edit lock");
                assert!(matches!(source, crate::daemon::staged_sources::Source::Show {imdb,key,..}
                    if imdb == "tt1234001" && key == "S01E01"));
                assert!(Shows::load().unwrap().get("tt1234001").unwrap().grabbed.is_empty());
                Ok(serde_json::from_value(json!({"state":"done", "hash":"ab".repeat(20), "duplicate":false})).unwrap())
            }).unwrap();
            assert!(matches!(reply, Reply::ShowsGrab(_)));
            assert!(Shows::load().unwrap().get("tt1234001").unwrap().grabbed.is_empty(),
                "source marks belong to successful admission/commit, not the shared handler");
            let rejected = catalogue(&service, &imdb, &[], &call).unwrap_err();
            assert_eq!(rejected.code, ErrorCode::EngineUnavailable);
        }
        let original = Shows::load().unwrap().get("tt1234001").unwrap().episodes.clone();
        for quality in [Some("720p"), None] {
            let args=serde_json::from_value(json!({"imdb":"tt1234001","quality":quality})).unwrap();
            assert!(matches!(catalogue(&service,&imdb,&[],&Call::ShowsSetQuality {args}).unwrap(),Reply::ShowsSetQuality(_)));
            let saved=Shows::load().unwrap();let show=saved.get("tt1234001").unwrap();
            assert_eq!(show.quality.as_deref(),quality);
            assert_eq!(show.episodes,original);
            assert!(show.grabbed.is_empty());
        }
        println!("shared Shows grab/quality: stage forwarded, edit lock released, marks delegated, local grab refused, quality saved without changing releases");
    }
}
