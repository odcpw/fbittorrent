//! RSS intents reuse the app store; refreshes cannot add engine records.
use super::{error, ApiResult};
use crate::{api::*, daemon::Daemon, rss::Store};
use serde_json::json;
pub(crate) fn failure(error:anyhow::Error)->Error {
    let mut failure=super::error(ErrorCode::Upstream,&format!("{error:#}"),None);
    failure.details=Some(serde_json::Map::from_iter([("source".into(),json!("rss"))]));
    failure
}
pub(crate) fn feed(daemon:&Daemon,id:&str)->ApiResult<Feed> { feed_in(&daemon.rss,id) }
fn feed_in(service:&crate::daemon::rss::Service,id:&str)->ApiResult<Feed> {
    service.feeds().map_err(failure)?.into_iter().find(|f|f.id==id).ok_or_else(|| {
        let mut failure=error(ErrorCode::NotFound,"unknown feed",None);
        failure.details=Some(serde_json::Map::from_iter([("feed".into(),json!(id))])); failure
    })
}
pub fn dispatch(daemon:&Daemon,call:&Call)->ApiResult<Reply> {
    dispatch_in(&daemon.rss,call,|args,feed,item| {
        crate::daemon::staged_sources::add(daemon,args,crate::daemon::staged_sources::Source::Rss {feed:feed.into(),item:item.into()})
    })
}
/// In-process catalogue operations share the daemon's exact handlers and store.
/// Grabs always go through the running owner's engine operation.
pub(crate) fn catalogue(service:&crate::daemon::rss::Service,call:&Call)->ApiResult<Reply> {
    dispatch_in(service,call,|_,_,_|Err(error(ErrorCode::EngineUnavailable,"grab requires the daemon",None)))
}
fn dispatch_in(service:&crate::daemon::rss::Service,call:&Call,add_torrent:impl FnOnce(AddArgs,&str,&str)->ApiResult<AddResult>)->ApiResult<Reply> {
    let store=Store::default();
    Ok(match call {
        Call::RssFeeds { .. }=>Reply::RssFeeds(FeedsResult { feeds:service.feeds().map_err(failure)? }),
        Call::RssAdd { args }=> {
            let url=url::Url::parse(&args.url).map_err(|_|error(ErrorCode::InvalidArgs,"invalid feed URL",Some("url")))?;
            if !matches!(url.scheme(),"http"|"https") || url.host_str().is_none() {return Err(error(ErrorCode::InvalidArgs,"feed URL must use HTTP(S)",Some("url")));}
            // Public feed URLs are redacted; compare the stored originals.
            if store.snapshot().map_err(failure)?.iter().any(|row|row.feed.url==args.url){return Err(error(ErrorCode::InvalidState,"feed already followed",Some("url")));}
            let cfg=crate::config::Config::load().map_err(failure)?;
            store.add(&args.url,args.name.as_deref(),cfg.timeout_secs.min(10)).map_err(failure)?;
            Reply::RssAdd(FeedResult { feed:feed_in(service,&crate::rss::stable_id(&args.url))? })
        },
        Call::RssRemove { args }=> { let f=feed_in(service,&args.feed)?; store.remove_id(&f.id).map_err(failure)?; Reply::RssRemove(EmptyResult {}) },
        Call::RssRefresh { args }=> {
            let name=args.as_ref().and_then(|a|a.feed.as_ref()).map(|id|feed_in(service,id).map(|f|f.name)).transpose()?.unwrap_or_default();
            let cfg=crate::config::Config::load().map_err(failure)?;
            service.refresh(&name,cfg.timeout_secs).map_err(failure)?;
            Reply::RssRefresh(MutationResult::Done)
        },
        Call::RssItems { args }=> {
            feed_in(service,&args.feed)?;
            let snapshot=service.snapshot().map_err(failure)?;
            Reply::RssItems(FeedItemsResult { items:snapshot.items.get(&args.feed).into_iter().flatten().filter(|a|!args.unread_only.unwrap_or(false)||!a.read).cloned().collect() })
        },
        Call::RssSetRead { args }=> { let f=feed_in(service,&args.feed)?; require_item(service,&args.feed,&args.item)?; store.set_read(&f.name,Some(&args.item),args.read).map_err(failure)?; Reply::RssSetRead(EmptyResult {}) },
        Call::RssMarkFeedRead { args }=> { let f=feed_in(service,&args.feed)?; store.mark_read(&f.name,None).map_err(failure)?; Reply::RssMarkFeedRead(CountResult {count:f.unread}) },
        Call::RssGrab { args }=> {
            let f=feed_in(service,&args.feed)?; require_item(service,&args.feed,&args.item)?;
            // The public item is redacted; the owner adds from the stored source.
            let item=service.snapshot().map_err(failure)?.sources.get(&args.feed).into_iter().flatten().find(|a|a.id==args.item).cloned()
                .ok_or_else(||error(ErrorCode::NotFound,"unknown feed item",None))?;
            let source=if let Some(magnet)=item.magnet {AddSource::Magnet(magnet)}else if let Some(url)=item.torrent_url {AddSource::Url(url)}else {return Err(error(ErrorCode::InvalidState,"item has no downloadable source",Some("item")));};
            let add = AddArgs {
                source,
                stage: args.stage,
                stopped: args.stopped,
                seed: args.seed,
                save_path: args.save_path.clone(),
                files: args.files.clone(),
                category: args.category.clone(),
                origin: Some("rss".into()),
            };
            let added=add_torrent(add,&f.id,&args.item)?;
            Reply::RssGrab(added)
        },
        _=>return Err(error(ErrorCode::Unsupported,"not an RSS operation",None)),
    })
}
fn require_item(service:&crate::daemon::rss::Service,id:&str,item:&str)->ApiResult<FeedItem> {
    service.snapshot().map_err(failure)?.items.get(id).into_iter().flatten().find(|a|a.id==item).cloned().ok_or_else(|| {
        let mut failure=error(ErrorCode::NotFound,"unknown feed item",None);
        failure.details=Some(serde_json::Map::from_iter([("feed".into(),json!(id)),("item".into(),json!(item))])); failure
    })
}
