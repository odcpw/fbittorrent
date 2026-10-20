//! Pause, resume, file selection updates and safe torrent removal.

use super::*;

fn remove_files_and_dirs(infos: &FileInfos, files: &dyn TorrentStorage) {
    let mut all_dirs = HashSet::new();
    for (id, fi) in infos.iter().enumerate() {
        if fi.attrs.padding {
            continue;
        }
        let mut fname = &*fi.relative_filename;
        if let Err(e) = files.remove_file(id, fname) {
            warn!(target: super::LOG_TARGET, { ?fi.relative_filename, error=?e }, "could not delete file");
        } else {
            debug!(target: super::LOG_TARGET, { ?fi.relative_filename }, "deleted the file")
        }
        while let Some(parent) = fname.parent() {
            if parent != Path::new("") {
                all_dirs.insert(parent);
            }
            fname = parent;
        }
    }

    let all_dirs = {
        let mut v = all_dirs.into_iter().collect::<Vec<_>>();
        v.sort_unstable_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));
        v
    };
    for dir in all_dirs {
        if let Err(e) = files.remove_directory_if_empty(dir) {
            warn!(target: super::LOG_TARGET, "error removing {dir:?}: {e:#}");
        } else {
            debug!(target: super::LOG_TARGET, "removed {dir:?}")
        }
    }
}

impl Session {
    pub async fn delete(&self, cx: &Cx, id: TorrentIdOrHash, delete_files: bool) -> anyhow::Result<()> {
        cx.checkpoint()?;
        let id = match id {
            TorrentIdOrHash::Id(id) => id,
            TorrentIdOrHash::Hash(h) => self
                .db
                .read()
                .torrents
                .values()
                .find_map(|v| {
                    if v.info_hash() == h {
                        Some(v.id())
                    } else {
                        None
                    }
                })
                .context("no such torrent in db")?,
        };
        let removed = self
            .db
            .write()
            .torrents
            .remove(&id)
            .with_context(|| format!("torrent with id {id} did not exist"))?;

        if let Err(e) = removed.pause() {
            debug!(target: super::LOG_TARGET, "error pausing torrent before deletion: {e:#}")
        }
        removed.close_run().await?;

        let metadata = removed.metadata.load_full().expect("TODO");

        let storage = removed
            .with_state_mut(|s| match s.take() {
                ManagedTorrentState::Initializing(p) => p.files.take().ok(),
                ManagedTorrentState::Paused(p) => Some(p.files),
                ManagedTorrentState::Live(l) => l
                    .pause()
                    // inspect_err not available in 1.75
                    .map_err(|e| {
                        warn!(target: super::LOG_TARGET, ?id, "error pausing torrent: {e:#}");
                        e
                    })
                    .ok()
                    .map(|p| p.files),
                _ => None,
            })
            .map(Ok)
            .unwrap_or_else(|| {
                removed
                    .shared
                    .storage_factory
                    .create(removed.shared(), &metadata)
            });

        match (storage, delete_files) {
            (Err(e), true) => return Err(e).context("torrent deleted, but could not delete files"),
            (Ok(storage), true) => {
                debug!(target: super::LOG_TARGET, "will delete files");
                remove_files_and_dirs(&metadata.file_infos, &storage);
                // A user-selected folder (including a single-file torrent's
                // save path) is never removed; only rqbit's own subfolder is.
                if removed.shared().options.derived_output_folder
                    && let Err(e) = storage.remove_directory_if_empty(Path::new(""))
                {
                    warn!(target: super::LOG_TARGET,
                        ?id,
                        "error removing {:?}: {e:#}",
                        removed.shared().options.output_folder
                    )
                }
            }
            (_, false) => {
                debug!(target: super::LOG_TARGET, "not deleting files")
            }
        };

        info!(target: super::LOG_TARGET, id, "deleted torrent");
        Ok(())
    }

    pub async fn pause(&self, cx: &Cx, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        cx.checkpoint()?;
        handle.pause()?;
        handle.close_run().await?;
        Ok(())
    }

    pub async fn unpause(self: &Arc<Self>, cx: &Cx, handle: &ManagedTorrentHandle) -> anyhow::Result<()> {
        handle.start(cx, None, false).await?;
        Ok(())
    }

    pub async fn update_only_files(
        self: &Arc<Self>,
        cx: &Cx,
        handle: &ManagedTorrentHandle,
        only_files: &HashSet<usize>,
    ) -> anyhow::Result<()> {
        cx.checkpoint()?;
        handle.update_only_files(only_files)?;
        Ok(())
    }
}
