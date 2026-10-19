//! Saved torrent records and durable ledger/metadata writes.
use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::{Condvar, Mutex as StdMutex};

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
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) deferred: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) staged: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) force: bool,
}
fn is_false(value: &bool) -> bool { !value }

impl Record {
    pub(super) fn deferred_magnet(options: &ops::AddRequest, folder: &Path) -> Result<Self> {
        let source = options.sources.first().context("no magnet supplied")?;
        let magnet = librqbit::Magnet::parse(source)?;
        let hash = magnet.as_id20().context("a v1 magnet hash is required")?.as_string();
        let files = magnet.get_select_only();
        let name = magnet.name.unwrap_or_else(|| hash.clone());
        Self::deferred_intent(options, folder, hash, name, source.clone(), files)
    }

    pub(super) fn deferred_torrent(options: &ops::AddRequest, folder: &Path, files: Vec<usize>) -> Result<Self> {
        let (_, bytes) = options.torrent.as_ref().context("local torrent metadata required")?;
        let metadata = librqbit::torrent_from_bytes(bytes)?;
        let info = metadata.info.data.validate()?;
        let hash = metadata.info_hash.as_string();
        let name = info.name().map(|s| s.into_owned()).unwrap_or_else(|| hash.clone());
        let count = info.iter_file_details().count();
        anyhow::ensure!(files.iter().all(|i| *i < count), "file index is out of range");
        let mut record = Self::deferred_intent(options, folder, hash, name, String::new(), Some(files))?;
        if options.save_path.is_none() && info.info().files.is_some() {
            let path = Path::new(&record.name);
            anyhow::ensure!(path.components().count() == 1 && matches!(path.components().next(), Some(std::path::Component::Normal(_))), "invalid torrent directory name");
            record.output_folder = folder.join(path).to_string_lossy().into_owned();
        }
        Ok(record)
    }

    fn deferred_intent(options: &ops::AddRequest, folder: &Path, hash: String, name: String, source: String, files: Option<Vec<usize>>) -> Result<Self> {
        let record = Self {
            hash, name, source, only_files: files,
            output_folder: options.save_path.clone().unwrap_or_else(|| folder.to_string_lossy().into()),
            upload_limit: options.upload_limit, ratio_limit: options.ratio_limit,
            category: options.category.clone(), tags: options.tags.clone(),
            added_on: now(), completion_on: 0, paused: options.stopped,
            resume_after_migration: false, deferred: true, staged: false, force: false,
        };
        record.options(true)?; // Physical transfer is paused; saved intent stays explicit.
        Ok(record)
    }

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
    pub(super) fn dormant_on_restore(&self) -> bool {
        // -1 is the legacy unknown-completion sentinel, never proof of completion.
        self.completion_on > 0 && self.stop_on_complete()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Ledger {
    #[serde(skip)]
    pub(super) generation: Arc<LedgerGeneration>,
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
        self.write_store(cx, "torrents.json", &serde_json::to_vec_pretty(ledger)?)
            .await?;
        ledger.generation.bump();
        Ok(())
    }
    pub(super) async fn write_store(&self, cx: &Cx, name: &str, bytes: &[u8]) -> Result<()> {
        cx.checkpoint()?;
        let temporary = self.directory.join(format!("{name}.tmp"));
        let final_path = self.directory.join(name);
        let mut file = rqbit_runtime::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .await?;
        // mode() only applies to creation; a pre-upgrade temporary may be 0644.
        file.set_permissions(rqbit_runtime::fs::Permissions::from_mode(0o600))
            .await?;
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
        self.write_store(cx, &format!("{hash}.torrent"), bytes)
            .await
    }
}

pub(super) fn save_parked_ledger(directory: &Path, ledger: &Ledger) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join("torrents.json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&serde_json::to_vec_pretty(ledger)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, directory.join("torrents.json"))?;
    std::fs::File::open(directory)?.sync_all()?;
    ledger.generation.bump();
    Ok(())
}

/// Share this handle with standard threads before starting their wait loop. It
/// owns the ledger's single atomic generation and notification pair; waiting
/// never takes the async records mutex. It is in-memory, not part of the store.
#[derive(Debug, Default)]
pub struct LedgerGeneration {
    counter: AtomicU64,
    condition: (StdMutex<()>, Condvar),
}
impl LedgerGeneration {
    pub fn current(&self) -> u64 {
        self.counter.load(Ordering::Acquire)
    }
    fn bump(&self) {
        let _guard = self
            .condition
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.counter.fetch_add(1, Ordering::Release);
        self.condition.1.notify_all();
    }
    pub fn wait_after(&self, generation: u64, timeout: Duration) -> Option<u64> {
        let guard = self
            .condition
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (_guard, _) = self
            .condition
            .1
            .wait_timeout_while(guard, timeout, |_| self.current() == generation)
            .unwrap_or_else(|error| error.into_inner());
        let current = self.current();
        (current != generation).then_some(current)
    }
}
impl EngineDispatcher {
    pub fn ledger_generation(&self) -> Arc<LedgerGeneration> {
        self.generation.clone()
    }
}

#[cfg(test)]
#[path = "tests/ledger.rs"]
mod tests;
