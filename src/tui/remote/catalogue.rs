//! Pushed catalogue changes reload today's local files without network work.
use super::*;
use std::os::unix::fs::MetadataExt;
type Stamp=(u64,u64,i64,i64);
fn stamp()->Option<Stamp>{std::fs::metadata(shows::path()).ok().map(|meta|(meta.ino(),meta.len(),meta.mtime(),meta.mtime_nsec()))}
pub(in crate::tui) struct ShowsSnapshot {before:Option<Stamp>,after:Option<Stamp>,rows:Shows}
impl TuiApp {
    pub(in crate::tui) fn reload_cached_shows(&self)->Cmd<Msg> {
        let Some(connection)=self.presence.as_ref().map(|connection|connection.id)else{return Cmd::none();};
        Cmd::task(move||{
            let before=stamp();
            let result=Shows::load().map(|rows|ShowsSnapshot{before,after:stamp(),rows}).map_err(|error|format!("{error:#}"));
            Msg::ShowsCached{connection,result}
        })
    }
    pub(in crate::tui) fn cached_shows(&mut self,connection:u64,result:Result<ShowsSnapshot,String>)->Cmd<Msg> {
        if self.presence.as_ref().is_none_or(|current|current.id!=connection||!current.alive()){return Cmd::none();}
        match result {
            Ok(snapshot) if snapshot.before==snapshot.after && snapshot.after==stamp()=>{
                let selected=self.selected_show().map(|show|show.imdb.clone());
                let episode=self.selected_release_target();
                let inbox=self.shows_inbox();
                self.shows=snapshot.rows;
                self.shows.shows.sort_by(|a,b|a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.imdb.cmp(&b.imdb)));
                let index=if inbox{Some(0)}else{selected.and_then(|id|self.shows.shows.iter().position(|show|show.imdb==id).map(|index|index+1)).or(Some(0))};
                self.show_list.borrow_mut().select(index);
                self.rebuild_inbox();
                if let Some(target)=episode {
                    let index=if self.shows_inbox(){self.inbox_rows.iter().position(|row|row.imdb==target.imdb&&row.key==target.key)}
                        else{self.selected_show().and_then(|show|show.episodes.iter().position(|row|row.key==target.key))};
                    self.ep_list.borrow_mut().select(index);
                }
            },
            Ok(_)=>{}, // A newer committed file has its own pushed update.
            Err(error)=>self.set_status(error,true),
        }
        Cmd::none()
    }
}
