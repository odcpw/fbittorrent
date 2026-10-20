use super::*;
impl Inner {
    pub(in crate::engine) fn invalidate_resume(&self, hash: &str) -> Result<()> {
        self.resume_prepared.remove(hash);
        self.resume.invalidate(hash)
    }
    /// Completion becomes durable before the ledger may restore it dormant.
    /// A stop cancels this kernel wait while completion_on is still unset, so
    /// an unflushed completed payload is checked again after a machine reboot.
    pub(in crate::engine) async fn sync_completed_payload(&self, cx: &Cx, record: &Record, handle: &ManagedTorrent) -> Result<bool> {
        let metadata = handle.metadata.load_full().context("completed torrent metadata unavailable")?;
        let bits = handle.have_pieces_snapshot().context("completed torrent have unavailable")?;
        let files = match stamps(record, &metadata.torrent_bytes, bits.as_bytes()) {
            Ok(files) => files,
            Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error|error.kind()==std::io::ErrorKind::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut directories = std::collections::BTreeSet::new();
        let root=Path::new(&record.output_folder);
        for stamp in files {
            let file = rqbit_runtime::fs::File::open(&stamp.path).await?.into_std()?;
            crate::durable_sync::sync_all(cx, &file).await?;
            let mut parent=stamp.path.parent();
            while let Some(path)=parent.filter(|path|path.starts_with(root)) {
                directories.insert(path.to_path_buf()); parent=path.parent();
            }
        }
        // Persist the directory links as well as the payload inode. The output
        // root itself can be the torrent's newly created named directory.
        directories.insert(root.to_path_buf());
        if let Some(parent)=root.parent(){directories.insert(parent.to_path_buf());}
        for path in directories {
            let directory = rqbit_runtime::fs::File::open(&path).await?.into_std()?;
            crate::durable_sync::sync_all(cx, &directory).await?;
        }
        Ok(true)
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
