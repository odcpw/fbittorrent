//! Shows commands share the owner's handlers and normalized views locally.
use super::{catalog_search,catalogue,shapes,ShowsCmd};
use crate::{agent::{AppError,Code},api::{self,client::Client},config::Config};
use anyhow::{Context,Result};
use serde_json::{json,Value};
use std::collections::BTreeSet;

struct Source<'a> {
    cfg:&'a Config,client:Option<Client>,local:Option<crate::daemon::shows::Service>,imdb:crate::imdb::service::Service,
}
impl<'a> Source<'a> {
    fn new(cfg:&'a Config)->Result<Self> {
        let client=catalog_search::running(cfg,"shows");
        let local=if client.is_none(){Some(crate::daemon::shows::Service::new()?)}else{None};
        Ok(Self{cfg,client,local,imdb:Default::default()})
    }
    fn call(&self,call:api::Call)->Result<Value> {
        self.outcome(call)?.map_err(api::errors::app_error)
    }
    fn outcome(&self,call:api::Call)->Result<std::result::Result<Value,api::Error>> {
        catalogue::validate(&call)?;
        match &self.client {
            Some(client)=>catalogue::execute(client,&call,self.cfg.timeout_secs),
            None=>{
                let rows=if matches!(&call,api::Call::ShowsReleases{..}){catalogue::saved_torrents()?}else{Vec::new()};
                Ok(api::ops::shows::catalogue(self.local.as_ref().unwrap(),&self.imdb,&rows,&call).map(|reply|reply.to_value().expect("typed catalogue reply")))
            },
        }
    }
    fn shows(&self)->Result<Vec<api::Show>> {
        let mut rows=match &self.client {
            Some(client)=>catalogue::fresh(client,&api::Call::ShowsList{args:None},"shows","shows",self.cfg.timeout_secs)?,
            None=>catalogue::typed::<api::ShowsResult>(self.call(api::Call::ShowsList{args:None})?)?.shows,
        };
        rows.sort_by(|a,b|a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.imdb.cmp(&b.imdb)));Ok(rows)
    }
    fn releases(&self,id:&str,unread:bool)->Result<Vec<api::Release>> {
        let mut rows=match &self.client {
            Some(client)=>catalogue::fresh(client,&api::Call::ShowsReleases{args:api::ShowReleasesArgs{imdb:Some(id.into()),inbox:None,unread_only:Some(unread),quality:None}},"releases",&format!("shows.releases:{id}"),self.cfg.timeout_secs)?,
            None=>catalogue::typed::<api::ReleasesResult>(self.call(api::Call::ShowsReleases{args:api::ShowReleasesArgs{imdb:Some(id.into()),inbox:None,unread_only:Some(unread),quality:None}})?)?.releases,
        };
        rows.retain(|row|!unread||!row.read);
        rows.sort_by(|a,b|b.published_at.cmp(&a.published_at).then(a.key.cmp(&b.key)));Ok(rows)
    }
    fn title(&self,query:&str)->Result<api::ImdbCard> {
        let args=super::catalog_imdb::arguments(query,false);
        let result=if let Some(client)=&self.client{catalogue::typed::<api::ImdbLookupResult>(client.call("imdb.lookup",serde_json::to_value(&args)?)?)?.card}
            else {let api::Reply::ImdbLookup(result)=api::ops::imdb::lookup(&self.imdb,&args).map_err(api::errors::app_error)?else{unreachable!()};result.card};
        result.ok_or_else(||AppError::new(Code::NotFound,"show not found").into())
    }
}
fn select<'a>(shows:&'a [api::Show],selector:&str)->Result<&'a api::Show> {
    if let Some(show)=shows.iter().find(|show|show.imdb==selector){return Ok(show);}
    let matches:Vec<_>=shows.iter().filter(|show|shapes::show_label(show).to_lowercase().contains(&selector.to_lowercase())).collect();
    match matches.as_slice() {
        [show]=>Ok(show),
        []=>Err(AppError::new(Code::NotFound,format!("no show matching '{selector}'")).hint("fbittorrent shows list --json").into()),
        _=>Err(AppError::new(Code::Ambiguous,format!("'{selector}' matches {} shows: {}",matches.len(),matches.iter().map(|show|show.title.as_str()).collect::<Vec<_>>().join(", "))).into()),
    }
}
fn print_releases(rows:&[api::Release]) {
    for row in rows {
        let mark=if row.transfer!=api::ReleaseTransfer::None{"↓"}else if row.read{" "}else{"*"};
        println!("{mark} {}  {:>5}  {:>9}  {:>4} seeds  {}",row.key,row.quality.as_deref().unwrap_or(""),row.size.map(|size|crate::human_bytes(size as i64)).unwrap_or_else(||"—".into()),row.seeds.map(|seeds|seeds.to_string()).unwrap_or_else(||"—".into()),row.title);
    }
}
pub(crate) fn run(cfg:&Config,sub:&ShowsCmd,json_mode:bool,no_start:bool)->Result<Code> {
    let source=Source::new(cfg)?;
    match sub {
        ShowsCmd::List=>{
            let shows=source.shows()?;let values:Vec<_>=shows.iter().map(shapes::show).collect();
            crate::emit(json_mode,json!({"ok":true,"count":shows.len(),"new":shows.iter().map(|show|show.unread).sum::<u64>(),"path":crate::shows::path(),"shows":values}),||{
                if shows.is_empty(){println!("no shows followed yet: fbittorrent shows follow \"<title or tt id>\"");}
                for show in &shows {println!("{:<10}  {:<40} {:>3} new / {:>3} eps  checked {}{}",show.imdb,crate::short(&shapes::show_label(show),40),show.unread,show.counts.episodes,show.last_check_at.map(|time|crate::qbt::ymd(time as i64)).unwrap_or_else(||"never".into()),show.warning.as_ref().map(|warning|format!("  [error: {warning}]")).unwrap_or_default());}
            });
        },
        ShowsCmd::Follow{query,quality}=>{
            let query=query.join(" ");
            if query.trim().is_empty(){return Err(AppError::new(Code::Usage,"give a title or an IMDb id").into());}
            let title=source.title(&query)?;
            let existing=source.shows()?.into_iter().find(|show|show.imdb==title.id);
            let added=existing.is_none();
            if existing.as_ref().is_some_and(|show|quality.is_some()&&show.quality!=*quality){return Err(AppError::new(Code::Failure,"show is already followed; changing an existing show's quality requires the quality operation").into());}
            if added {source.call(api::Call::ShowsFollow{args:api::ShowFollowArgs{query:None,imdb:Some(title.id.clone()),quality:quality.clone()}})?;}
            let outcome=source.outcome(api::Call::ShowsCheck{args:Some(api::ShowCheckArgs{imdb:Some(title.id.clone())})})?;
            let shows=source.shows()?;let show=select(&shows,&title.id)?;
            crate::emit(json_mode,json!({"ok":true,"action":"shows-follow","added":added,"imdb":title.id,"title":title.title,"year":title.year,"kind":title.kind,"episodes":show.counts.episodes,"new":show.unread,"fetched":outcome.is_ok(),"warning":show.warning,"catalogue_complete":show.catalogue_complete}),||{
                println!("{} {} [{}]: {} releases on EZTV, {} new",if added{"following"}else{"already following"},shapes::show_label(show),show.imdb,show.counts.episodes,show.unread);
                if let Some(warning)=&show.warning{println!("warning: {warning}");}
            });
        },
        ShowsCmd::Unfollow{show}=>{
            let shows=source.shows()?;let show=select(&shows,show)?;
            source.call(api::Call::ShowsUnfollow{args:api::ShowArgs{imdb:show.imdb.clone()}})?;
            crate::emit(json_mode,json!({"ok":true,"action":"shows-unfollow","imdb":show.imdb,"title":show.title}),||println!("unfollowed {}",shapes::show_label(show)));
        },
        ShowsCmd::Episodes{show,new}=>{
            let shows=source.shows()?;let show=select(&shows,show)?;let rows=source.releases(&show.imdb,*new)?;
            let values:Vec<_>=rows.iter().map(shapes::release).collect();
            crate::emit(json_mode,json!({"ok":true,"imdb":show.imdb,"title":show.title,"last_checked":show.last_check_at.unwrap_or(0),"count":rows.len(),"warning":show.warning,"catalogue_complete":show.catalogue_complete,"episodes":values}),||{
                if rows.is_empty(){println!("{}: nothing listed{}",shapes::show_label(show),if show.last_check_at.is_none(){" (never checked: fbittorrent shows check)"}else{""});}
                if let Some(warning)=&show.warning{println!("warning: {warning}");}print_releases(&rows);
            });
        },
        ShowsCmd::Seen{show,episodes}=>{
            let shows=source.shows()?;let show=select(&shows,show)?;let rows=source.releases(&show.imdb,false)?;
            let mut changed=Vec::new();
            if episodes.iter().any(|episode|episode=="all") {
                let value=source.call(api::Call::ShowsMarkShowRead{args:api::ShowArgs{imdb:show.imdb.clone()}})?;
                changed.push(json!({"all":true,"marked":value["count"]}));
            } else {
                if episodes.is_empty(){return Err(AppError::new(Code::Usage,"give release keys like S01E02 or S01, or `all`").into());}
                let selected=episodes.iter().map(|key|{let key=key.to_uppercase();rows.iter().find(|row|row.key==key).ok_or_else(||AppError::new(Code::NotFound,format!("{} has no episode {key}",show.title)))}).collect::<std::result::Result<Vec<_>,_>>()?;
                for row in selected {source.call(api::Call::ShowsSetRead{args:api::ShowReadArgs{imdb:show.imdb.clone(),key:row.key.clone(),read:!row.read}})?;changed.push(json!({"episode":row.key,"seen":!row.read}));}
            }
            let updated=source.shows()?;let updated=select(&updated,&show.imdb)?;
            crate::emit(json_mode,json!({"ok":true,"action":"shows-seen","imdb":show.imdb,"show":show.title,"changed":changed,"new":updated.unread}),||println!("{}: {} new left",show.title,updated.unread));
        },
        ShowsCmd::Check{show}=>{
            let before=source.shows()?;
            let selected=if let Some(selector)=show{vec![select(&before,selector)?]}else{before.iter().collect::<Vec<_>>()};
            let old=selected.iter().map(|show|Ok((show.imdb.clone(),source.releases(&show.imdb,false)?.into_iter().map(|row|row.key).collect::<BTreeSet<_>>()))).collect::<Result<std::collections::BTreeMap<_,_>>>()?;
            let outcome=source.outcome(api::Call::ShowsCheck{args:show.as_ref().map(|_|api::ShowCheckArgs{imdb:Some(selected[0].imdb.clone())})})?;
            if let Err(error)=&outcome {if error.code!=api::ErrorCode::Upstream{return Err(api::errors::app_error(error.clone()));}}
            let current=source.shows()?;let mut report=Vec::new();let mut releases=Vec::new();let mut failed=0;
            for prior in &selected {
                let show=select(&current,&prior.imdb)?;let rows=source.releases(&show.imdb,false)?;
                let fresh=rows.iter().filter(|row|!old[&show.imdb].contains(&row.key)).count();
                failed+=usize::from(show.warning.is_some());
                report.push(json!({"imdb":show.imdb,"title":shapes::show_label(show),"fresh":fresh,"new":show.unread,"error":show.warning}));
                releases.push((shapes::show_label(show),rows.into_iter().filter(|row|!row.read).collect::<Vec<_>>()));
            }
            crate::emit(json_mode,json!({"ok":true,"action":"shows-check","checked":selected.len(),"failed":failed,"new":current.iter().map(|show|show.unread).sum::<u64>(),"shows":report}),||{
                for row in &report {if let Some(error)=row["error"].as_str(){println!("{}: {error}",row["title"].as_str().unwrap());}else{println!("{}: {} fresh, {} new",row["title"].as_str().unwrap(),row["fresh"],row["new"]);}}
                for(label,rows)in &releases{if !rows.is_empty(){println!("\n{label}");print_releases(rows);}}
            });
            if failed>0&&failed==selected.len(){return Ok(Code::Upstream);}
        },
        ShowsCmd::Grab{show,episode}=>{
            let shows=source.shows()?;let show=select(&shows,show)?;let rows=source.releases(&show.imdb,false)?;let key=episode.to_uppercase();
            let row=rows.iter().find(|row|row.key==key).ok_or_else(||AppError::new(Code::NotFound,format!("{} has no episode {key}",show.title)))?;
            let owner=catalogue::write_owner(cfg,no_start,json_mode)?;
            let call=api::Call::ShowsGrab{args:api::ShowGrabArgs{imdb:show.imdb.clone(),key:row.key.clone(),seed:Some(cfg.seed_by_default),stopped:Some(false),save_path:cfg.save_path.clone(),files:None,category:cfg.category.clone()}};
            let added:api::AddResult=catalogue::typed(catalogue::success(&owner,&call,cfg.timeout_secs)?)?;
            let hash=added.hash.context("grab confirmation omitted hash")?;
            let mut value=json!({"ok":true,"action":"shows-grab","imdb":show.imdb,"show":show.title,"episode":row.key,"hash":hash,"name":row.title});
            if added.deferred==Some(true){value["deferred"]=json!(true);}
            crate::emit(json_mode,value,||println!("added: {}",row.title));
        },
    }
    Ok(Code::Ok)
}
