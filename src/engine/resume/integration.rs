use super::*;
impl Inner {
    pub(in crate::engine) fn invalidate_resume(&self, hash: &str) -> Result<()> {
        self.resume_prepared.remove(hash);
        self.resume.invalidate(hash)
    }
    fn resume_handles(&self) -> HashMap<String, Arc<ManagedTorrent>> {
        self.session.with_torrents(|rows| {
            rows.filter(|(_,handle)|!self.moves.active(&handle.info_hash().as_string())).map(|(_, handle)| (handle.info_hash().as_string(), handle.clone()))
                .collect()
        })
    }
    pub(in crate::engine) async fn save_resume(&self, cx: &Cx, clean: bool) -> Result<()> {
        let records = OwnedMutexGuard::lock(self.ledger.clone(), cx)
            .await?
            .records
            .clone();
        let handles = self.resume_handles();
        let store = self.resume.clone();
        task::spawn_blocking(cx, move || store.save(&records, &handles, clean)).await??;
        Ok(())
    }
    pub(in crate::engine) async fn periodic_resume(&self, cx: &Cx) -> Result<()> {
        let due = {
            let mut previous = self.resume.periodic.lock().unwrap();
            if previous.elapsed() < Duration::from_secs(300) {
                false
            } else {
                *previous = Instant::now();
                true
            }
        };
        if due {
            self.save_resume(cx, false).await?;
        }
        Ok(())
    }
}
impl Store {
    pub(crate) fn abandon_clean(&self) {
        let _io = self.io.lock().unwrap();
        self.clean.store(false, Ordering::Release);
        let _ = std::fs::remove_file(self.directory.join("CLEAN"));
    }
}

impl Store {
    /// A parked slot has no payload writers. Preserve a previously earned clean
    /// token across a clean daemon exit, without authorizing an unclean cache.
    pub(crate) fn preserve_parked_clean(&self) -> Result<()> {
        let _io = self.io.lock().unwrap();
        if self.clean.load(Ordering::Acquire) {
            if let Some(marker) = &self.boot_marker {
                atomic_write(&self.directory.join("CLEAN"), marker)?;
            }
        }
        Ok(())
    }
}
