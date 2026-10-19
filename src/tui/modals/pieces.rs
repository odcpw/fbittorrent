//! A bounded visible-detail subscription and cached one-line canvas.
use super::*;
use crate::api;
use ftui_extras::canvas::{CanvasRef,Mode,Painter};
use std::sync::{atomic::{AtomicBool,Ordering},Mutex};

pub(super) struct PiecesSession {
    connection:Arc<attach::Presence>,
    closed:AtomicBool,
    sub:Mutex<Option<u64>>,
}
impl PiecesSession {
    pub fn start(connection:Arc<attach::Presence>, hash:String)->(Arc<Self>,Cmd<Msg>) {
        let owner=Arc::new(Self {connection,closed:AtomicBool::new(false),sub:Mutex::new(None)});
        let task=owner.clone();
        let command=Cmd::task(move || {
            let result=(||->Result<()> {
                let reply:api::SubscribeResult=serde_json::from_value(task.connection.request("subscribe",serde_json::json!({"topics":[format!("torrent:{hash}.pieces")],"interval_ms":2000}))?)?;
                let mut sub=task.sub.lock().unwrap();
                if task.closed.load(Ordering::Acquire) {drop(sub);task.connection.request("unsubscribe",serde_json::json!({"sub":reply.sub}))?;}
                else {*sub=Some(reply.sub);}
                Ok(())
            })();
            Msg::ActionDone(result.map(|_|"Pieces observed".into()).map_err(|e|e.to_string()))
        });
        (owner,command)
    }
    pub fn close(&self)->Cmd<Msg> {
        self.closed.store(true,Ordering::Release);
        let Some(sub)=self.sub.lock().unwrap().take() else {return Cmd::none();};
        let connection=self.connection.clone();
        Cmd::task(move || Msg::ActionDone(connection.request("unsubscribe",serde_json::json!({"sub":sub})).map(|_|"Pieces hidden".into()).map_err(|e|e.to_string())))
    }
}

pub(super) struct PiecesCanvas {
    painter:Painter,
    cells:Vec<ftui::Cell>,
    key:Option<(u64,u16,ftui::PackedRgba,ftui::PackedRgba,ftui::PackedRgba)>,
    header:Option<Paragraph<'static>>,
    legend:Paragraph<'static>,
}
impl Default for PiecesCanvas {
    fn default()->Self {Self {painter:Painter::new(0,0,Mode::Block),cells:Vec::new(),key:None,header:None,legend:Paragraph::new("▲ first/last · █ verified · ▒ missing · underline playable prefix")}}
}
impl PiecesCanvas {
    pub fn render(&mut self, pieces:&api::Pieces, bits:&[u8], revision:u64, area:Rect, theme:&Theme, frame:&mut Frame) {
        let width=area.width.min(pieces.count.min(u16::MAX as u64) as u16);
        if width==0 || area.height==0 {return;}
        let row=Rect::new(area.x,area.y+2,width,1);
        if self.key!=Some((revision,width,theme.fg,theme.dim,theme.accent)) {
            self.painter.ensure_size(width,1,Mode::Block);self.painter.clear();self.cells.clear();
            for x in 0..width {
                let first=u64::from(x)*pieces.count/u64::from(width);
                let end=u64::from(x+1)*pieces.count/u64::from(width);
                let p=pieces.priority.partition_point(|p|*p<first);
                let priority=pieces.priority.get(p).is_some_and(|p|*p<end);
                let have=(first..end).all(|index|api::Pieces::has(bits,index));
                let color=if priority {theme.accent}else if have {theme.fg}else {theme.dim};
                self.painter.point_colored(i32::from(x),0,color);
                let mut cell=ftui::Cell::from_char(if priority {'▲'}else if have {'█'}else {'▒'}).with_fg(color);
                if pieces.playable_end.is_some_and(|end|first<end) {cell.attrs=ftui::CellAttrs::new(ftui::StyleFlags::UNDERLINE.into(),0);}
                self.cells.push(cell);
            }
            self.header=Some(Paragraph::new(format!("{} / {} verified pieces",bits.iter().map(|b|u64::from(b.count_ones())).sum::<u64>(),pieces.count)).style(theme.text()));
            self.key=Some((revision,width,theme.fg,theme.dim,theme.accent));
        }
        if let Some(header)=&self.header {header.render(Rect::new(area.x,area.y,area.width,1),frame);}
        self.legend.render(Rect::new(area.x,area.y+4,area.width,2),frame);
        CanvasRef::from_painter(&self.painter).render(row,frame);
        for (offset,cell) in self.cells.iter().enumerate() {frame.buffer.set(row.x+offset as u16,row.y,*cell);}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pieces_canvas_retains_glyphs_prefix_and_cached_storage() {
        let data:api::Pieces=serde_json::from_str(include_str!("../../../agent_ergonomics_audit/audit/regression_tests/fixtures/states/pieces-1000.json")).unwrap();
        let bits=data.decode().unwrap();let mut canvas=PiecesCanvas::default();
        let mut pool=ftui::GraphemePool::new();let mut frame=Frame::new(100,8,&mut pool);let theme=Theme::default();
        canvas.render(&data,&bits,1,Rect::new(0,0,100,8),&theme,&mut frame);
        assert_eq!(frame.buffer.get(0,2).unwrap().content.as_char(),Some('▲'));
        assert_eq!(frame.buffer.get(99,2).unwrap().content.as_char(),Some('▲'));
        assert_eq!(frame.buffer.get(30,2).unwrap().content.as_char(),Some('▒'));
        assert!(frame.buffer.get(10,2).unwrap().attrs.flags().contains(ftui::StyleFlags::UNDERLINE.into()));
        assert!(!frame.buffer.get(30,2).unwrap().attrs.flags().contains(ftui::StyleFlags::UNDERLINE.into()));
        let pointer=canvas.cells.as_ptr();let header=canvas.header.as_ref().unwrap() as *const Paragraph<'static>;
        for _ in 0..32 {canvas.render(&data,&bits,1,Rect::new(0,0,100,8),&theme,&mut frame);}
        assert_eq!(canvas.cells.as_ptr(),pointer);assert_eq!(canvas.header.as_ref().unwrap() as *const Paragraph<'static>,header);
        println!("1000-piece canvas: ▲ at first/last; ▒ missing; verified prefix underline; retained cells/header across32 unchanged frames");
    }
}
