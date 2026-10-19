//! RSS commands use one set of native handlers and views on either side of api.sock.
use super::{catalog_search,catalogue,shapes,RssCmd};
use crate::{agent::{AppError,Code},api::{self,client::Client},config::Config};
use anyhow::{Context,Result};
use serde_json::{json,Value};

struct Source<'a> {cfg:&'a Config,client:Option<Client>,local:Option<crate::daemon::rss::Service>}
impl<'a> Source<'a> {
    fn new(cfg:&'a Config)->Result<Self> {
        let client=catalog_search::running(cfg,"rss");
        let local=if client.is_none(){Some(crate::daemon::rss::Service::new()?)}else{None};
        Ok(Self{cfg,client,local})
    }
    fn call(&self,call:api::Call)->Result<Value> {
        catalogue::validate(&call)?;
        match &self.client {
            Some(client)=>catalogue::success(client,&call,self.cfg.timeout_secs),
            None=>Ok(api::ops::rss::catalogue(self.local.as_ref().unwrap(),&call).map_err(api::errors::app_error)?.to_value()?),
        }
    }
    fn feeds(&self)->Result<Vec<api::Feed>> {
        let mut rows=match &self.client {
            Some(client)=>catalogue::fresh(client,&api::Call::RssFeeds{args:None},"feeds","rss.feeds",self.cfg.timeout_secs)?,
            None=>catalogue::typed::<api::FeedsResult>(self.call(api::Call::RssFeeds{args:None})?)?.feeds,
        };
        rows.sort_by(|a,b|a.id.cmp(&b.id));Ok(rows)
    }
    fn items(&self,feed:&str,unread:bool)->Result<Vec<api::FeedItem>> {
        let mut rows=match &self.client {
            Some(client)=>catalogue::fresh(client,&api::Call::RssItems{args:api::FeedItemsArgs{feed:feed.into(),unread_only:Some(unread)}},"items",&format!("rss.items:{feed}"),self.cfg.timeout_secs)?,
            None=>catalogue::typed::<api::FeedItemsResult>(self.call(api::Call::RssItems{args:api::FeedItemsArgs{feed:feed.into(),unread_only:Some(unread)}})?)?.items,
        };
        rows.retain(|row|!unread||!row.read);
        rows.sort_by(|a,b|b.published_at.cmp(&a.published_at).then(a.id.cmp(&b.id)));Ok(rows)
    }
}
fn select<'a>(feeds:&'a [api::Feed],name:&str)->Result<&'a api::Feed> {
    if let Some(feed)=feeds.iter().find(|feed|feed.name==name){return Ok(feed);}
    let candidates:Vec<_>=feeds.iter().filter(|feed|feed.name.to_lowercase().contains(&name.to_lowercase())).collect();
    match candidates.as_slice() {
        [feed]=>Ok(feed),
        []=>Err(AppError::new(Code::NotFound,format!("no feed named '{name}'")).hint("fbittorrent rss list --json").into()),
        _=>Err(AppError::new(Code::Ambiguous,format!("'{name}' matches {} feeds: {}",candidates.len(),candidates.iter().map(|feed|feed.name.as_str()).collect::<Vec<_>>().join(", "))).into()),
    }
}
pub(crate) fn run(cfg:&Config,sub:&RssCmd,json_mode:bool,no_start:bool)->Result<Code> {
    let source=Source::new(cfg)?;
    match sub {
        RssCmd::List=>{
            let feeds=source.feeds()?;
            let rows=feeds.iter().map(|feed|Ok(shapes::feed(feed,&source.items(&feed.id,false)?))).collect::<Result<Vec<_>>>()?;
            crate::emit(json_mode,json!({"ok":true,"count":rows.len(),"feeds":rows}),||{
                for feed in &rows {println!("{:<30} {:>4} unread / {:>4}  {}{}",feed.name,feed.articles.iter().filter(|item|!item.is_read).count(),feed.articles.len(),feed.url,if feed.has_error{"  [error]"}else{""});}
            });
        },
        RssCmd::Add{url,name}=>{
            let name=name.clone().or_else(||crate::host_of(url)).unwrap_or_else(||url.clone());
            source.call(api::Call::RssAdd{args:api::FeedAddArgs{url:url.clone(),name:Some(name.clone())}})?;
            crate::emit(json_mode,json!({"ok":true,"action":"rss-add","name":name,"url":url}),||println!("added feed {name}"));
        },
        RssCmd::Remove{name}=>{
            let feeds=source.feeds()?;let feed=select(&feeds,name)?;
            source.call(api::Call::RssRemove{args:api::FeedArgs{feed:feed.id.clone()}})?;
            crate::emit(json_mode,json!({"ok":true,"action":"rss-remove","name":feed.name}),||println!("removed {}",feed.name));
        },
        RssCmd::Refresh{name}=>{
            let feeds=source.feeds()?;
            let selected=if let Some(name)=name{vec![select(&feeds,name)?]}else{feeds.iter().collect()};
            let names:Vec<_>=selected.iter().map(|feed|feed.name.clone()).collect();
            for feed in &selected {source.call(api::Call::RssRefresh{args:Some(api::FeedRefreshArgs{feed:Some(feed.id.clone())})})?;}
            let current=source.feeds()?;
            anyhow::ensure!(current.iter().filter(|feed|selected.iter().any(|selected|selected.id==feed.id)).all(|feed|!feed.loading),"refresh confirmation still reports loading");
            crate::emit(json_mode,json!({"ok":true,"action":"rss-refresh","feeds":names}),||println!("refreshing {} feed(s)",names.len()));
        },
        RssCmd::Items{name,unread}=>{
            let feeds=source.feeds()?;let feed=select(&feeds,name)?;
            let rows=source.items(&feed.id,*unread)?.iter().map(shapes::article).collect::<Vec<_>>();
            crate::emit(json_mode,json!({"ok":true,"feed":feed.name,"count":rows.len(),"items":rows}),||{
                for item in &rows {println!("{}  {}  {}  {}",if item.is_read{" "}else{"*"},crate::qbt::ymd(item.ts),item.id,item.title);}
            });
        },
        RssCmd::Grab{name,article_id}=>{
            let feeds=source.feeds()?;let feed=select(&feeds,name)?;
            let items=source.items(&feed.id,false)?;
            let item=items.iter().find(|item|item.id==*article_id).ok_or_else(||AppError::new(Code::NotFound,format!("no item with id '{article_id}' in feed '{}'",feed.name)).hint(format!("fbittorrent rss items \"{}\" --json",feed.name)))?;
            let owner=catalogue::write_owner(cfg,no_start,json_mode)?;
            let call=api::Call::RssGrab{args:api::FeedGrabArgs{feed:feed.id.clone(),item:item.id.clone(),seed:Some(cfg.seed_by_default),stopped:Some(false),save_path:cfg.save_path.clone(),files:None,category:cfg.category.clone()}};
            let added:api::AddResult=catalogue::typed(catalogue::success(&owner,&call,cfg.timeout_secs)?)?;
            let hash=added.hash.context("grab confirmation omitted hash")?;
            let mut value=json!({"ok":true,"action":"rss-grab","hash":hash,"name":item.title,"feed":feed.name});
            if added.deferred==Some(true){value["deferred"]=json!(true);}
            crate::emit(json_mode,value,||println!("added: {}",item.title));
        },
    }
    Ok(Code::Ok)
}
