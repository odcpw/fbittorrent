//! Legacy qBittorrent capture, durable import and source retirement (removed in P4-09).
use super::*;
use crate::{backend, config, engine, qbt, rss, search};
use crate::qbt::Client;

impl EngineDispatcher {
    pub fn import_legacy(
        &self,
        torrents: Vec<Torrent>,
        files: HashMap<String, Vec<TorrentFile>>,
        backup: PathBuf,
    ) -> Result<usize> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            let _addition = OwnedMutexGuard::lock(self.inner.additions.clone(), cx).await?;
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            if ledger.migrated_legacy {
                return Ok(0);
            }
            if !ledger.records.is_empty() {
                bail!("native torrents already exist; legacy migration requires an empty native library");
            }
            let mut staged = Vec::new();
            for torrent in torrents {
                if torrent.hash.len() != 40 || !torrent.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    bail!("legacy torrent {} has an unsupported hash; native migration requires v1 torrents", torrent.name);
                }
                let seed_allowed = torrent.seed_allowed();
                let was_running = !torrent.is_stopped();
                let path = backup.join(format!("{}.torrent", torrent.hash));
                let bytes = rqbit_runtime::fs::read(&path)
                    .await
                    .with_context(|| format!("reading old torrent metadata {}", path.display()))?;
                let metadata = librqbit::torrent_from_bytes(&bytes)
                    .with_context(|| format!("reading metainfo for {}", torrent.name))?;
                if metadata.info_hash.as_string() != torrent.hash {
                    bail!("legacy metadata hash does not match {}", torrent.name);
                }
                let info = metadata.info.data.validate()?;
                let list = files.get(&torrent.hash).context("legacy torrent file list is missing")?;
                let selected = Some(list.iter().filter(|file| file.priority != 0).map(|file| file.index as usize).collect());
                let content = PathBuf::from(&torrent.content_path);
                let multi = info.info().files.is_some();
                let output_folder = if !torrent.content_path.is_empty() {
                    if multi {
                        content
                    } else {
                        content
                            .parent()
                            .unwrap_or(Path::new(&torrent.save_path))
                            .to_path_buf()
                    }
                } else {
                    PathBuf::from(&torrent.save_path)
                };
                let details: Vec<_> = info.iter_file_details().collect();
                if details.len() != list.len() {
                    bail!("legacy file list does not match metainfo for {}", torrent.name);
                }
                let mut indices = HashSet::new();
                for file in list {
                    let index = usize::try_from(file.index).context("legacy file index is negative")?;
                    let detail = details.get(index).context("legacy file index is out of range")?;
                    if !indices.insert(index) || file.size < 0 || file.size as u64 != detail.len {
                        bail!("legacy file sizes or indices do not match metainfo for {}", torrent.name);
                    }
                    let native_path = output_folder.join(detail.filename.to_pathbuf());
                    let legacy_path = PathBuf::from(&torrent.save_path).join(&file.name);
                    if native_path != legacy_path {
                        bail!("legacy file {} has a renamed or unsupported layout; native import would use {}; keep the legacy engine for this library", legacy_path.display(), native_path.display());
                    }
                    if file.progress > 0.0 && !native_path.is_file() {
                        bail!("legacy downloaded file {} is missing or uses an incomplete-file suffix; native migration cannot preserve its bytes", native_path.display());
                    }
                }
                let record = Record {
                    display_name:None,
                    hash: torrent.hash.clone(),
                    name: torrent.name,
                    source: String::new(),
                    output_folder: output_folder.to_string_lossy().into_owned(),
                    only_files: selected,
            trackers: Vec::new(), removed_trackers: Vec::new(), download_limit: None,
                    upload_limit: if seed_allowed {
                        torrent.up_limit.max(0)
                    } else {
                        NO_UPLOAD_LIMIT
                    },
                    ratio_limit: if seed_allowed { -2.0 } else { 0.0 },
                    category: torrent.category,
                    origin: ledger::origin_from_tags(&torrent.tags),
                    tags: torrent.tags,
                    added_on: torrent.added_on,
                    completion_on: torrent.completion_on,
                    paused: true,
                    resume_after_migration: was_running, deferred: false, staged: false, force: false,
                };
                record.options(true)?;
                staged.push((record, bytes));
            }
            let mut restored = Vec::new();
            for (record, bytes) in &staged {
                self.inner.write_metadata(cx, &record.hash, bytes).await?;
                match self.inner.restore_record(cx, record, true).await {
                    Ok(handle) => restored.push(handle),
                    Err(error) => {
                        for handle in restored {
                            let _ = self.inner.session.delete(cx, TorrentIdOrHash::Id(handle.id()), false).await;
                        }
                        return Err(error).context("importing old downloads; legacy records remain available for retry");
                    }
                }
            }
            let count = staged.len();
            ledger.records.extend(staged.into_iter().map(|(record, _)| record));
            ledger.migrated_legacy = true;
            self.inner.save(cx, &ledger).await?;
            Ok(count)
        })
    }

    /// True once legacy torrent records have been imported durably.
    /// Source service retirement and resuming imported torrents are separate steps.
    pub fn legacy_migration_completed(&self) -> Result<bool> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            Ok(OwnedMutexGuard::lock(self.inner.ledger.clone(), cx)
                .await?
                .migrated_legacy)
        })
    }

    pub fn resume_imported(&self) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire) {
            bail!("native engine is closing");
        }
        let cx = self.runtime.cx();
        self.runtime.block_on(async {
            let mut ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), cx).await?;
            for record in &mut ledger.records {
                if record.resume_after_migration {
                    record.paused = false;
                    record.resume_after_migration = false;
                }
            }
            self.inner.save(cx, &ledger).await?;
            self.inner
                .reconcile_queue(cx, &mut ledger, &HashSet::new())
                .await
        })
    }

}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct LegacyMigration {
    pub(crate) phase: String,
    pub(crate) source_url: String,
    pub(crate) source_unit: String,
    pub(crate) backup: std::path::PathBuf,
    pub(crate) torrents: Vec<Torrent>,
    pub(crate) files: std::collections::HashMap<String, Vec<qbt::TorrentFile>>,
    pub(crate) feeds: Vec<qbt::RssFeed>,
    pub(crate) plugins: Vec<qbt::SearchPlugin>,
}

impl LegacyMigration {
    pub(crate) fn save(&self, path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = path.parent().context("migration path has no parent")?;
        std::fs::create_dir_all(directory)?;
        let staged = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true)
            .mode(0o600).open(&staged)?;
        // Creation mode does not repair a temporary left by an earlier writer.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.sync_all()?;
        std::fs::rename(&staged, path)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
}

pub(crate) fn migrate_legacy(cfg: &Config, dispatcher: &engine::EngineDispatcher) -> Result<()> {
    let path = config::data_dir().join("legacy-migration.json");
    let mut source_cfg = cfg.clone();
    source_cfg.engine = "qbittorrent".into();
    let mut migration: LegacyMigration = if path.exists() {
        let saved: LegacyMigration = serde_json::from_slice(&std::fs::read(&path)?)
            .context("reading legacy migration progress; the qBittorrent source is preserved")?;
        if saved.phase == "complete" { return Ok(()); }
        anyhow::ensure!(matches!(saved.phase.as_str(), "captured" | "prepared"), "unknown legacy migration phase; source preserved");
        source_cfg.url = saved.source_url.clone();
        source_cfg.backend_unit = saved.source_unit.clone();
        saved
    } else {
        let source = Client::from_config(&source_cfg);
        if source.login().is_err() || source.version().is_err() {
            anyhow::ensure!(!backend::is_active(&source_cfg.backend_unit),
                "the legacy qBittorrent service is active but unavailable; migration is deferred and its records are preserved");
            if !backend::start_for_legacy_migration(&source_cfg, &source)? { return Ok(()); }
        }
        anyhow::ensure!(source_cfg.url.starts_with("http://127.0.0.1:") || source_cfg.url.starts_with("http://localhost:"),
            "automatic legacy migration requires a local qBittorrent WebUI; the source is preserved");
        source.login().context("logging into legacy qBittorrent before migration")?;
        let torrents = source.torrents().context("capturing all legacy torrents")?;
        let mut files = std::collections::HashMap::new();
        for torrent in &torrents {
            let selected = source.files(&torrent.hash)
                .with_context(|| format!("capturing legacy file selection for {}", torrent.name))?;
            anyhow::ensure!(!selected.is_empty(), "legacy torrent {} has no resolved file metadata; source preserved", torrent.name);
            files.insert(torrent.hash.clone(), selected);
        }
        let saved = LegacyMigration {
            phase: "captured".into(),
            source_url: source_cfg.url.clone(),
            source_unit: source_cfg.backend_unit.clone(),
            backup: dirs::data_local_dir().context("no local data directory for qBittorrent backup")?
                .join("qBittorrent/BT_backup"),
            torrents,
            files,
            feeds: source.rss_feeds().context("capturing legacy RSS feeds and read history")?,
            plugins: source.search_plugins().context("capturing legacy provider preferences")?,
        };
        saved.save(&path).context("saving legacy migration snapshot; source preserved")?;
        saved
    };

    if migration.phase == "captured" {
        if migration.files.values().flatten().any(|file| !matches!(file.priority, 0 | 1)) {
            eprintln!("legacy file priorities above normal become normal; native file selection supports skip and normal");
        }
        let count = dispatcher.import_legacy(migration.torrents.clone(), migration.files.clone(), migration.backup.clone())
            .context("importing legacy torrents paused; qBittorrent remains available")?;
        rss::import(&migration.feeds).context("saving legacy RSS history; qBittorrent remains available")?;
        search::import_plugins(&migration.plugins).context("saving legacy search preferences; qBittorrent remains available")?;
        migration.phase = "prepared".into();
        migration.save(&path).context("saving completed import before retiring the source")?;
        eprintln!("saved {count} legacy torrent(s), {} RSS feed(s), and provider preferences", migration.feeds.len());
    }

    let source = Client::from_config(&source_cfg);
    if legacy_endpoint_reachable(&source_cfg.url)? {
        source.login()?;
        let current = source.torrents()?;
        let same_hashes = current.len() == migration.torrents.len()
            && current.iter().all(|torrent| migration.torrents.iter().any(|old| old.hash == torrent.hash));
        anyhow::ensure!(same_hashes, "legacy torrents changed during migration; source kept running and imported torrents remain paused");
        for torrent in &current {
            let old = migration.torrents.iter().find(|old| old.hash == torrent.hash).unwrap();
            anyhow::ensure!(migration_torrent_settings(old) == migration_torrent_settings(torrent),
                "legacy settings changed for {}; source kept running and imported torrents remain paused", torrent.name);
            let files = source.files(&torrent.hash)?;
            let selection = |items: &[qbt::TorrentFile]| items.iter().map(|file| (file.index, file.name.clone(), file.priority)).collect::<Vec<_>>();
            anyhow::ensure!(selection(&files) == selection(&migration.files[&torrent.hash]),
                "legacy file selection changed for {}; source kept running and imported torrents remain paused", torrent.name);
        }
    }
    backend::stop_legacy(&source_cfg).context("retiring the legacy service after saved import; native torrents remain paused")?;
    anyhow::ensure!(!legacy_endpoint_reachable(&source_cfg.url)?,
        "legacy qBittorrent still answers after stopping its service; close that instance before native downloads can resume");
    anyhow::ensure!(dispatcher.legacy_migration_completed()?, "native import has not been durably completed; source records preserved");
    backend::disable_legacy(&source_cfg).context("retiring legacy login startup after verified import")?;
    dispatcher.resume_imported().context("resuming only the legacy torrents that were running")?;
    migration.phase = "complete".into();
    migration.save(&path).context("saving completed legacy migration")?;
    eprintln!("legacy migration complete; downloads now run while fbittorrent is open");
    Ok(())
}

fn migration_torrent_settings(torrent: &Torrent) -> serde_json::Value {
    serde_json::json!({
        "hash": torrent.hash, "name": torrent.name, "save_path": torrent.save_path,
        "content_path": torrent.content_path, "category": torrent.category, "tags": torrent.tags,
        "up_limit": torrent.up_limit, "ratio_limit": torrent.ratio_limit,
        "stopped": torrent.is_stopped(),
    })
}

fn legacy_endpoint_reachable(url: &str) -> Result<bool> {
    let authority = url.strip_prefix("http://").context("legacy migration requires a local HTTP WebUI")?
        .split('/').next().unwrap_or_default();
    let (host, port) = authority.rsplit_once(':').context("legacy WebUI URL has no port")?;
    anyhow::ensure!(matches!(host, "127.0.0.1" | "localhost"), "legacy migration requires a loopback WebUI");
    let port: u16 = port.parse().context("invalid legacy WebUI port")?;
    Ok(std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(250)).is_ok())
}
