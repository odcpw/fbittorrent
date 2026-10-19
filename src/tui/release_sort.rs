//! Release ordering runs on actions and data changes, never in the renderer.
use super::*;
use std::cmp::Ordering;
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(super) enum ReleaseSort { Date, Title, Seeds, Size }
impl ReleaseSort {
    pub(super) fn label(self)->&'static str {match self {Self::Date=>"date",Self::Title=>"title",Self::Seeds=>"seeds",Self::Size=>"size"}}
}
fn optional(a:i64,b:i64,descending:bool)->Ordering {
    match ((a>=0).then_some(a),(b>=0).then_some(b)) {
        (Some(a),Some(b))=>if descending {b.cmp(&a)}else{a.cmp(&b)},
        (None,None)=>Ordering::Equal,(None,Some(_))=>Ordering::Greater,(Some(_),None)=>Ordering::Less,
    }
}
pub(super) fn episode_order(a:&Episode,b:&Episode,field:ReleaseSort,descending:bool)->Ordering {
    let order=match field {
        ReleaseSort::Date=>optional(if a.released>0 {a.released}else{-1},if b.released>0 {b.released}else{-1},descending),
        ReleaseSort::Title=>if descending {b.title.cmp(&a.title)}else{a.title.cmp(&b.title)},
        ReleaseSort::Seeds=>optional(a.seeds,b.seeds,descending),
        ReleaseSort::Size=>optional(a.size,b.size,descending),
    };
    order
}
impl TuiApp {
    pub(super) fn order_release_rows(&mut self) {
        let (field,descending)=(self.release_sort,self.release_descending);
        for show in &mut self.shows.shows {show.episodes.sort_by(|a,b|episode_order(a,b,field,descending).then_with(||b.key.cmp(&a.key)));}
    }
    pub(super) fn cycle_release_sort(&mut self, reverse:bool)->Cmd<Msg> {
        use ReleaseSort::*;
        let selected=self.selected_release_target();
        let fields=[Date,Title,Seeds,Size];
        if reverse {self.release_descending=!self.release_descending;}else {
            self.release_sort=fields[(fields.iter().position(|f|*f==self.release_sort).unwrap()+1)%fields.len()];
            self.release_descending=self.release_sort!=Title;
        }
        self.order_release_rows();self.rebuild_inbox();
        let index=selected.and_then(|target|if self.shows_inbox() {self.inbox_rows.iter().position(|r|r==&target)}else{self.selected_show().and_then(|show|show.episodes.iter().position(|r|r.key==target.key))}).or_else(||(self.episode_count()>0).then_some(0));
        self.ep_list.borrow_mut().select(index);self.ep_list.borrow_mut().hovered=None;
        self.shows_focus_eps=true;
        self.set_status(format!("Releases by {} {}",self.release_sort.label(),if self.release_descending{"↓"}else{"↑"}),false);
        Cmd::none()
    }
}
