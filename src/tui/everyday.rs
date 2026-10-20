//! Deliberate keyboard actions over current confirmed rows.
use super::*;
impl TuiApp {
    pub(super) fn sync_activity_label(&mut self) {self.activity_text=self.compute_activity_label();}
    fn compute_activity_label(&self)->Option<String> {
        if self.store.network.as_ref().is_some_and(|n|!n.ready) || self.store.summary.as_ref().is_some_and(|s|s.network.state==crate::api::NetworkReadiness::Waiting) {
            return Some("Waiting for VPN".into());
        }
        let summary=self.store.summary.as_ref()?;
        if summary.dl_bps.is_some_and(|rate|rate>0)||summary.ul_bps.is_some_and(|rate|rate>0) {return None;}
        if matches!(summary.daemon.state,crate::api::DaemonState::Starting) {return Some("Starting…".into());}
        if let Some(seconds)=summary.daemon.idle_exit_in_s {return Some(format!("Idle · stops in {}",human_eta(seconds.min(i64::MAX as u64) as i64)));}
        if summary.daemon.busy_reasons.iter().any(|r|r=="search") {return Some("Searching…".into());}
        let counts=&summary.counts;
        if counts.error>0 {return Some(format!("! {} error{}",counts.error,if counts.error==1 {""}else{"s"}));}
        if counts.downloading+counts.metadata+counts.checking+counts.stalled+counts.queued+counts.seeding==0 && summary.daemon.busy_reasons.iter().all(|r|r=="interactive") {return Some("Idle".into());}
        None
    }
    pub(super) fn followable_title(&self, name: &str) -> bool {
        if let Some(ImdbState::Ready(title, _)) = self.imdb_state(name) {
            return title.kind.get(..2).is_some_and(|prefix|prefix.eq_ignore_ascii_case("tv"));
        }
        // Episode/season syntax is a useful TV hint before the card has loaded.
        // This predicate is used by footer rendering, so it does not parse/allocate.
        name.as_bytes().windows(4).any(|w| w[0].eq_ignore_ascii_case(&b's')
            && w[1].is_ascii_digit() && w[2].is_ascii_digit()
            && (w[3].eq_ignore_ascii_case(&b'e') || !w[3].is_ascii_alphanumeric()))
    }
    pub(super) fn cycle_search_sort(&mut self, reverse: bool)->Cmd<Msg> {
        let fields=[Column::Seeds,Column::Name,Column::Size,Column::Rating,Column::Uploaded,Column::Provider,Column::Peers];
        let index=fields.iter().position(|c|c.search_field()==Some(self.search_sort)).unwrap_or(0);
        self.sort_search_column(fields[if reverse {index}else{(index+1)%fields.len()}])
    }
    pub(super) fn copy_search_magnet(&mut self) -> Cmd<Msg> {
        let magnet = self.selected_result().and_then(|r| {
            self.wire_result(r)
                .and_then(|row| row.magnet.clone())
                .or_else(|| {
                    r.file_url
                        .starts_with("magnet:")
                        .then(|| r.file_url.clone())
                })
        });
        self.copy_magnet(magnet)
    }
    pub(super) fn copy_torrent_magnet(&mut self) -> Cmd<Msg> {
        let magnet = self.selected_torrent().map(|t| {
            format!(
                "magnet:?xt=urn:btih:{}&dn={}",
                t.hash,
                crate::qbt::urlencode(&t.name)
            )
        });
        self.copy_magnet(magnet)
    }
    fn copy_magnet(&mut self, magnet: Option<String>) -> Cmd<Msg> {
        let Some(magnet) = magnet else {
            self.set_status(
                "No magnet supplied by this provider; use O for its source page",
                true,
            );
            return Cmd::none();
        };
        copy(Some(magnet))
    }
    pub(super) fn copy_episode_magnet(&mut self)->Cmd<Msg> {
        let magnet=self.selected_episode().map(|(_,e)|e.magnet.clone()).filter(|s|s.starts_with("magnet:"));
        self.copy_magnet(magnet)
    }
    pub(super) fn can_undo_removal(&self)->bool {
        self.online&&!self.service_busy&&self.removal_undo.as_ref().is_some_and(|(_,until)|(crate::shows::now().max(0) as u64)<*until)
    }
    pub(super) fn undo_removal(&mut self)->Cmd<Msg> {
        if !self.online || self.service_busy {return Cmd::none();}
        let Some((_,until))=self.removal_undo.as_ref() else {return Cmd::none()};
        if *until<=crate::shows::now().max(0) as u64 {
            self.removal_undo=None;self.set_status("Removal undo expired (10 seconds); your files were kept",true);return Cmd::none();
        }
        let (hash,_)=self.removal_undo.take().unwrap();
        self.service_action("torrents.add",serde_json::json!({"source":{"restore":hash}}),"Removal undone")
    }
}
fn copy(source:Option<String>)->Cmd<Msg> {
    Cmd::task(move||{
        let result=(||->Result<()> {
            use std::io::Write;
            use std::process::{Command,Stdio};
            let source=source.filter(|s|s.starts_with("magnet:")&&s.len()<=32768&&crate::magnet_hash(s).is_some()).ok_or_else(||anyhow::anyhow!("No magnet is available for this row"))?;
            // Clipboard data goes through stdin, never argv or status/history.
            let mut child=Command::new("wl-copy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|anyhow::anyhow!("Could not start wl-copy"))?;
            let deadline=std::time::Instant::now()+Duration::from_secs(2);
            let mut input=child.stdin.take().unwrap();
            use std::os::fd::AsRawFd;
            let flags=unsafe {libc::fcntl(input.as_raw_fd(),libc::F_GETFL)};
            if flags<0 || unsafe {libc::fcntl(input.as_raw_fd(),libc::F_SETFL,flags|libc::O_NONBLOCK)}<0 {
                let _=child.kill();let _=child.wait();anyhow::bail!("Could not open a bounded clipboard pipe");
            }
            let mut offset=0;
            while offset<source.len() {
                let written=input.write(&source.as_bytes()[offset..]);
                match written {
                    Ok(n) if n>0=>offset+=n,
                    Err(error) if matches!(error.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::Interrupted)=>{},
                    _=>{let _=child.kill();let _=child.wait();anyhow::bail!("Could not write the selected magnet to the clipboard");},
                }
                if std::time::Instant::now()>=deadline {let _=child.kill();let _=child.wait();anyhow::bail!("Clipboard copy timed out");}
                if offset<source.len() {std::thread::sleep(Duration::from_millis(20));}
            }
            drop(input);
            loop {
                match child.try_wait() {
                    Ok(Some(status))=>{anyhow::ensure!(status.success(),"wl-copy could not copy the selected magnet");return Ok(());},
                    Ok(None)=>{},
                    Err(_)=>{let _=child.kill();let _=child.wait();anyhow::bail!("Could not finish clipboard copy");},
                }
                if std::time::Instant::now()>=deadline {let _=child.kill();let _=child.wait();anyhow::bail!("Clipboard copy timed out");}
                std::thread::sleep(Duration::from_millis(20));
            }
        })();
        Msg::ActionDone(result.map(|_|"Magnet copied".into()).map_err(|e|e.to_string()))
    })
}
