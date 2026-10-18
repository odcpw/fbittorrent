//! Saved torrent records and durable ledger/metadata writes.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) hash: String,
    pub(super) name: String,
    pub(super) source: String,
    pub(super) output_folder: String,
    pub(super) only_files: Option<Vec<usize>>,
    pub(super) upload_limit: i64,
    pub(super) ratio_limit: f64,
    pub(super) category: String,
    pub(super) tags: String,
    pub(super) added_on: i64,
    pub(super) completion_on: i64,
    // Durable user intent. Queue and completion suspension never rewrite this.
    pub(super) paused: bool,
    #[serde(default)]
    pub(super) resume_after_migration: bool,
}

impl Record {
    pub(super) fn options(&self, paused: bool) -> Result<AddTorrentOptions> {
        if self.upload_limit < 0 {
            bail!("upload limit cannot be negative");
        }
        // rqbit reserves an entire protocol chunk from its rate limiter. A smaller
        // bucket would make a normal 16 KiB request impossible to serve.
        if self.upload_limit > 0 && self.upload_limit < 16 * 1024 {
            bail!("native upload limits must be zero (unlimited) or at least 16 KiB/s");
        }
        let upload_bps = if self.upload_limit > 0 {
            Some(
                NonZeroU32::new(
                    u32::try_from(self.upload_limit)
                        .context("upload limit exceeds the native engine maximum")?,
                )
                .context("invalid upload limit")?,
            )
        } else {
            None
        };
        Ok(AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: Some(self.output_folder.clone()),
            only_files: self.only_files.clone(),
            ratelimits: LimitsConfig {
                upload_bps,
                download_bps: None,
            },
            peer_limit: Some(96),
            ..Default::default()
        })
    }
    pub(super) fn stop_on_complete(&self) -> bool {
        self.ratio_limit == 0.0
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Ledger {
    #[serde(default)]
    pub(super) migrated_legacy: bool,
    pub(super) records: Vec<Record>,
}

fn validate_info_hash(hash: &str) -> Result<()> {
    if hash.len() != 40
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("saved torrent hash must be 40 lowercase hexadecimal characters");
    }
    Ok(())
}

pub(super) fn read_ledger(path: &Path) -> Result<Ledger> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let ledger: Ledger = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "reading saved torrent records {}; the file has been preserved",
            path.display()
        )
    })?;
    let mut hashes = HashSet::new();
    for record in &ledger.records {
        validate_info_hash(&record.hash).with_context(|| {
            format!(
                "invalid saved torrent record {} in {}",
                record.name,
                path.display()
            )
        })?;
        if !hashes.insert(&record.hash) {
            bail!(
                "duplicate saved torrent hash {} in {}; the file has been preserved",
                record.hash,
                path.display()
            );
        }
        record.options(true).with_context(|| {
            format!(
                "invalid saved options for {} in {}",
                record.name,
                path.display()
            )
        })?;
    }
    Ok(ledger)
}

impl Inner {
    pub(super) fn torrent_path(&self, hash: &str) -> Result<PathBuf> {
        validate_info_hash(hash)?;
        Ok(self.directory.join(format!("{hash}.torrent")))
    }
    pub(super) async fn save(&self, cx: &Cx, ledger: &Ledger) -> Result<()> {
        self.write_store(cx, "torrents.json", &serde_json::to_vec_pretty(ledger)?).await
    }
    pub(super) async fn write_store(&self, cx: &Cx, name: &str, bytes: &[u8]) -> Result<()> {
        cx.checkpoint()?;
        let temporary = self.directory.join(format!("{name}.tmp"));
        let final_path = self.directory.join(name);
        let mut file = rqbit_runtime::fs::OpenOptions::new()
            .write(true).create(true).truncate(true).mode(0o600).open(&temporary).await?;
        // mode() only applies to creation; a pre-upgrade temporary may be 0644.
        file.set_permissions(rqbit_runtime::fs::Permissions::from_mode(0o600)).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        rqbit_runtime::fs::rename(&temporary, &final_path).await?;
        // Make both the replacement ledger and preceding metadata renames durable.
        rqbit_runtime::fs::File::open(&self.directory)
            .await?
            .sync_all()
            .await?;
        Ok(())
    }
    pub(super) async fn cache_metadata(&self, cx: &Cx, handle: &Arc<ManagedTorrent>) -> Result<()> {
        let metadata = handle
            .metadata
            .load_full()
            .context("torrent metadata is not available")?;
        self.write_metadata(cx, &handle.info_hash().as_string(), &metadata.torrent_bytes)
            .await?;
        Ok(())
    }
    pub(super) async fn write_metadata(&self, cx: &Cx, hash: &str, bytes: &[u8]) -> Result<()> {
        validate_info_hash(hash)?;
        self.write_store(cx, &format!("{hash}.torrent"), bytes).await
    }
}

pub(super) fn save_parked_ledger(directory: &Path, ledger: &Ledger) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join("torrents.json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&serde_json::to_vec_pretty(ledger)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, directory.join("torrents.json"))?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}
