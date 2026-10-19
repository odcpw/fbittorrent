//! Format only when totals, theme or layout tier change, not on every redraw.
use super::*;
use crate::api::{AllTimeTotals,SampledStats,SessionTotals};
#[derive(Default)]
pub(super) struct Footer {
    key:Option<(SessionTotals,AllTimeTotals,Style,bool)>,
    text:Option<Paragraph<'static>>,
}
impl Footer {
    pub(super) fn render(&mut self,stats:&SampledStats,style:Style,area:Rect,frame:&mut Frame) {
        let (Some(session),Some(all))=(&stats.session,&stats.all_time) else{return;};
        if area.is_empty(){return;}
        let narrow=area.width<100;
        let key=(session.clone(),all.clone(),style,narrow);
        if self.key.as_ref()!=Some(&key) {
            let session=format!("session ↓ {} ↑ {}",format::bytes(Some(session.downloaded)),format::bytes(Some(session.uploaded)));
            let all=format!("all time ↓ {} ↑ {} · {} completed",format::bytes(Some(all.downloaded)),format::bytes(Some(all.uploaded)),all.completed);
            self.text=Some(Paragraph::new(format!("{session}{}{all}",if narrow{"\n"}else{" · "})).style(style).wrap(WrapMode::None));
            self.key=Some(key);
        }
        if let Some(text)=&self.text {text.render(area,frame);}
    }
}
