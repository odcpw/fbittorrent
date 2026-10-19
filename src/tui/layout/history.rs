//! Use only received summary samples; optional chrome never displaces tabs/rates.
use super::*;
use ftui::widgets::sparkline::Sparkline;
impl TuiApp {
    pub(in crate::tui) fn view_speed_history(&self,area:Rect,frame:&mut Frame) {
        if !self.online || self.store.stale || area.width<8 {return;}
        let Some(history)=self.store.summary.as_ref().and_then(|s|s.history.as_ref()).filter(|h|!h.is_empty()) else{return;};
        // Stack storage avoids allocating on every terminal redraw.
        let mut samples=[0.;60];
        let len=history.len().min(60).min(usize::from(area.width));
        for (out,value) in samples.iter_mut().zip(history.iter().skip(history.len()-len)){*out=*value as f64;}
        Sparkline::new(&samples[..len]).min(0.).style(self.theme.dimmed()).render(Rect::new(area.x,area.y,len as u16,1),frame);
    }
}
