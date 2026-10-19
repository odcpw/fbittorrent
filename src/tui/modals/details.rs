//! Stable-hash details. Opening a row always starts on Overview, at every size.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailsTab {
    Overview,
    Files,
    Peers,
    Trackers,
    Log,
    Pieces,
}
#[derive(Default)]
struct TabLabel {key: Option<(DetailsTab,bool,bool,bool)>,text: String}
pub(super) struct DetailsModal {
    pub hash: String,
    pub name: String,
    pub tab: DetailsTab,
    pub loading: bool,
    pub pieces:Option<Arc<PiecesSession>>,
    pub canvas:RefCell<PiecesCanvas>,
    pub trackers: modals_trackers::TrackerPanel,
    tab_label: RefCell<TabLabel>,
    pub peer_offset: usize,
    pub peer_cache: RefCell<modals_peers::PeerTable>,
}
pub(super) fn details_area(bounds: Rect) -> Rect {
    if bounds.width < 100 || bounds.height < 30 {
        return bounds;
    }
    let width = bounds.width.min(100);
    Rect::new(
        bounds.x + (bounds.width - width) / 2,
        bounds.y + (bounds.height - 30) / 2,
        width,
        30,
    )
}
impl TuiApp {
    pub(super) fn toggle_details_seed(&mut self)->Cmd<Msg> {
        let Some(details)=self.details.as_ref() else{return Cmd::none();};
        let Some(row)=self.store.torrents.get(&details.hash) else{return Cmd::none();};
        let on=!row.upload_enabled;let hash=details.hash.clone();
        self.service_action("torrents.set_seed",serde_json::json!({"hashes":[hash],"on":on}),if on {"Upload: on"}else{"Upload: off"})
    }

    pub(super) fn open_details(&mut self, torrent: &Torrent) -> Cmd<Msg> {
        let close=self.close_details();
        self.details = Some(DetailsModal {
            hash: torrent.hash.clone(),
            name: torrent.name.clone(),
            tab: DetailsTab::Overview,
            loading: false,
            pieces:None,
            canvas:RefCell::new(PiecesCanvas::default()),
            trackers: Default::default(),
            tab_label: RefCell::default(),
            peer_offset: 0,
            peer_cache: RefCell::default(),
        });
        let overview=self.refresh_details();
        Cmd::batch(vec![close,overview])
    }
    /// Fetch immutable metainfo once, and again when metadata becomes known.
    /// One outstanding read belongs to the visible detail on this connection.
    pub(super) fn refresh_details(&mut self)->Cmd<Msg> {
        let Some(connection)=self.presence.clone().filter(|c|c.alive()) else {return Cmd::none();};
        let Some(details)=self.details.as_mut().filter(|d|!d.loading) else {return Cmd::none();};
        let hash=crate::api::Hash::new(details.hash.clone()).unwrap();
        let size=self.store.torrents.get(hash.as_str()).and_then(|row|row.size);
        if self.store.details.get(&hash).is_some_and(|row|row.torrent.size==size) {return Cmd::none();}
        details.loading=true;
        Cmd::task(move|| {
            let result=connection.request("torrents.get",serde_json::json!({"hash":hash}))
                .and_then(|value|Ok(serde_json::from_value(value)?)).map_err(|e|e.to_string());
            Msg::ApiDetail {connection:connection.id,hash,result}
        })
    }
    pub(super) fn detail_loaded(&mut self,connection:u64,hash:crate::api::Hash,result:Result<crate::api::GetResult,String>)->Cmd<Msg> {
        if self.presence.as_ref().is_none_or(|c|c.id!=connection || !c.alive()) {return Cmd::none();}
        let Some(details)=self.details.as_mut().filter(|d|d.hash==hash.as_str()) else {return Cmd::none();};
        details.loading=false;
        match result.and_then(|result|self.store.reply(&crate::api::Call::TorrentsGet {args:crate::api::HashArgs {hash}},crate::api::Reply::TorrentsGet(result)).map_err(|e|e.to_string())) {
            Ok(())=>{}, Err(error)=>self.set_status(format!("Torrent details: {error}"),true),
        }
        Cmd::none()
    }
    pub(super) fn resume_pieces(&mut self)->Cmd<Msg> {
        let Some(details)=self.details.as_mut().filter(|d|d.tab==DetailsTab::Pieces) else {return Cmd::none();};
        let close=details.pieces.take().map(|p|p.close()).unwrap_or_else(Cmd::none);
        self.store.pieces.clear();
        if let Some(connection)=self.presence.clone().filter(|c|c.hello.caps.iter().any(|cap|cap=="pieces")) {
            let (session,command)=PiecesSession::start(connection,details.hash.clone());details.pieces=Some(session);
            Cmd::batch(vec![close,command])
        } else {details.tab=DetailsTab::Overview;close}
    }
    pub(super) fn details_tab(&mut self, next: bool) -> Cmd<Msg> {
        let tabs = self.details_tabs();
        let Some(current) = self.details.as_ref().map(|d| d.tab) else {return Cmd::none();};
        let Some(index) = tabs.iter().position(|tab| *tab == current) else {return Cmd::none();};
        let index = if next {index.saturating_add(1).min(tabs.len()-1)} else {index.saturating_sub(1)};
        let tab = tabs[index];
        if tab == current {return Cmd::none();}
        let cleanup=self.details.as_mut().unwrap().pieces.take().map(|p|p.close()).unwrap_or_else(Cmd::none);
        self.store.pieces.clear();
        self.details.as_mut().unwrap().tab = tab;
        if tab!=DetailsTab::Trackers {self.store.trackers.clear();let panel=&mut self.details.as_mut().unwrap().trackers;panel.busy=false;panel.request=0;}
        self.files = None;
        if tab != DetailsTab::Peers {self.store.peers.clear();}
        if tab == DetailsTab::Pieces {let command=self.resume_pieces();return if matches!(cleanup,Cmd::None) {command} else {Cmd::batch(vec![cleanup,command])};}
        if tab == DetailsTab::Trackers {let command=self.load_trackers(false);return if matches!(cleanup,Cmd::None) {command} else {Cmd::batch(vec![cleanup,command])};}
        if tab == DetailsTab::Files {
            if let Some(torrent)=self.selected_torrent().cloned() {let command=self.open_files(&torrent);return if matches!(cleanup,Cmd::None) {command} else {Cmd::batch(vec![cleanup,command])};}
        }
        cleanup
    }
    pub(super) fn details_tabs(&self) -> Vec<DetailsTab> {
        let mut tabs=vec![DetailsTab::Overview,DetailsTab::Files];
        if self.has_cap("peers") {tabs.push(DetailsTab::Peers);}
        if self.has_cap("trackers") {tabs.push(DetailsTab::Trackers);}
        tabs.push(DetailsTab::Log);
        if self.has_cap("pieces") {tabs.push(DetailsTab::Pieces);}
        tabs
    }
    fn details_tab_label(&self) -> std::cell::Ref<'_,str> {
        let details=self.details.as_ref().expect("visible details");
        let key=(details.tab,self.has_cap("peers"),self.has_cap("trackers"),self.has_cap("pieces"));
        let mut cache=details.tab_label.borrow_mut();
        if cache.key!=Some(key) {
            cache.text=self.details_tabs().iter().map(|tab| {
                let name=match tab {DetailsTab::Overview=>"Overview",DetailsTab::Files=>"Files",DetailsTab::Peers=>"Peers",DetailsTab::Trackers=>"Trackers",DetailsTab::Pieces=>"Pieces",DetailsTab::Log=>"Log"};
                if *tab==details.tab {format!("[{name}]")} else {name.to_string()}
            }).collect::<Vec<_>>().join("  ")+"    [ / ] tabs";
            cache.key=Some(key);
        }
        drop(cache);
        std::cell::Ref::map(details.tab_label.borrow(),|c|c.text.as_str())
    }
    pub(super) fn has_cap(&self, cap: &str) -> bool {
        self.store.hello.as_ref().is_some_and(|hello|hello.caps.iter().any(|c|c==cap))
    }
    pub(super) fn close_details(&mut self) -> Cmd<Msg> {
        let close=self.details.take().and_then(|mut d|d.pieces.take()).map(|p|p.close()).unwrap_or_else(Cmd::none);
        self.files=None;
        self.store.pieces.clear();
        self.store.details.clear();
        self.store.peers.clear();self.store.trackers.clear();
        close
    }
    pub(super) fn view_details(&self, frame: &mut Frame) {
        let Some(details) = &self.details else { return };
        let th = &self.theme;
        let bounds = frame.bounds();
        self.rects.borrow_mut().overlay();
        let area = details_area(bounds);
        self.rects.borrow_mut().details = Some(area);
        Paragraph::new(Text::from_lines((0..area.height).map(|_| {
            Line::styled(" ".repeat(area.width as usize), th.text().bg(th.selection))
        })))
        .render(area, frame);
        let footer = Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 2);
        let panel = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(2));
        let title = format!(
            " {} ",
            short(&details.name, panel.width.saturating_sub(4) as usize)
        );
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border_focused())
            .title(&title)
            .padding(Sides::horizontal(1));
        let inner = block.inner(panel);
        block.render(panel, frame);
        let rows = Flex::vertical()
            .constraints([Constraint::Fixed(2), Constraint::Fill])
            .split(inner);
        let tabs=self.details_tab_label();
        Paragraph::new(&*tabs)
            .style(th.accented())
            .render(rows[0], frame);
        if self.selected_torrent().is_none() {
            Paragraph::new("This torrent is no longer in your list.")
                .style(th.dimmed())
                .render(rows[1], frame);
        } else if details.tab==DetailsTab::Trackers {
            self.view_trackers(rows[1],frame);
        } else if details.tab==DetailsTab::Peers {
            self.view_peers(rows[1],frame);
        } else if details.tab==DetailsTab::Log {
            self.view_log_contents(rows[1],Some(&details.hash),frame);
        } else if details.tab == DetailsTab::Pieces {
            let hash=crate::api::Hash::new(details.hash.clone()).unwrap();
            if let Some((data,bits))=self.store.pieces.get(&hash) {
                details.canvas.borrow_mut().render(data,bits,self.store.pieces_rev,rows[1],th,frame);
            } else {Paragraph::new("Verified pieces unavailable until checking completes").style(th.dimmed()).render(rows[1],frame);}
        } else if details.tab == DetailsTab::Files {
            self.view_file_contents(rows[1], frame);
        } else if let Some(t) = self.selected_torrent() {
            let api = self.store.torrents.get(&t.hash);
            let metrics = format::Metrics { legacy: t, api };
            let kv = |key: &str, value: String| {
                Line::from_spans([
                    Span::styled(format!("{key:<10}"), th.dimmed()),
                    Span::styled(value, th.text()),
                ])
            };
            let mut lines = vec![kv(
                "state",
                format!(
                    "{} · {}",
                    format::state_cell(t, api).trim_end(),
                    self.torrent_reason(t)
                ),
            )];
            if let Some(playable) = api.and_then(|r| r.playable.as_ref()) {
                let ready = if !playable.ready { "Buffering" }
                    else if playable.stall_free==Some(true) || (playable.stall_free.is_none() && playable.buffered_s.is_some_and(|s|s>=120)) {"▶ Ready to watch"}
                    else {"Playable · may stall"};
                let buffer = playable
                    .buffered_s
                    .map(|s| format!(" · {} buffered", human_eta(s.min(i64::MAX as u64) as i64)))
                    .unwrap_or_default();
                let finish=playable.finishes_in_s.map(|s|format!(" · finishes in {}",human_eta(s.min(i64::MAX as u64) as i64))).unwrap_or_default();
                let estimate=if playable.buffered_s.is_none() {
                    let steady=api.is_some_and(|r|crate::daemon::watch::estimate_rate(r.size.unwrap_or(0),r.dl_bps.unwrap_or(0)));
                    if steady {" · rate estimate: keeps up with a 2 h video"}else{" · rate estimate: may stall for a 2 h video"}
                }else{""};
                lines.push(kv("ready", format!("{ready}{buffer}{finish}{estimate} · (o) Open")));
            }
            lines.extend([
                kv("progress", metrics.progress()),
                kv(
                    "speed",
                    format!("{} · ETA {}", metrics.speed(), metrics.eta()),
                ),
                kv("peers", metrics.peers(self.cfg.native())),
                kv("policy", metrics.upload()),
                kv("path", t.save_path.clone()),
                kv("added", ago(t.added_on)),
            ]);
            if let Some(row) = api {
                lines.push(kv("down cap",self.overview_cap_text(row)));
                lines.push(kv(
                    "origin",
                    row.origin.clone().unwrap_or_else(|| "unknown".into()),
                ));
            }
            if let Some(detail)=self.store.details.values().find(|row|row.torrent.hash.as_str()==details.hash && row.torrent.size.is_some()) {
                lines.push(kv("network",if detail.private {"Private torrent (no DHT, no PeX)"}else{"Public torrent"}.into()));
            }
            if let Some(error) = &t.error_message {
                lines.push(Line::styled(error.clone(), th.err().bold()));
            }
            let height = lines.len().min(u16::MAX as usize) as u16;
            let sections = Flex::vertical()
                .constraints([Constraint::Fixed(height + 1), Constraint::Fill])
                .split(rows[1]);
            Paragraph::new(Text::from_lines(lines))
                .wrap(WrapMode::None)
                .render(sections[0], frame);
            self.view_imdb_card(&t.name, sections[1], frame);
        }
        self.rects.borrow_mut().footer=footer;
        self.view_status(footer, frame);
    }
}
