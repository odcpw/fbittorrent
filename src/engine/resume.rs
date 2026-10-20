//! Optional, boot-scoped have cache. A token is consumed before a writer starts;
//! every payload stamp and the exact selection must still match. The durable
//! ledger is independent: after reboot we hash rather than trust unflushed data.
use super::*;
use std::{
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    sync::Mutex as StdMutex,
    time::Instant,
};
mod integration;
#[cfg(test)]
mod tests;
const MAGIC: &[u8; 8] = b"FBTRSM01";
const MAX_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    mtime: i64,
    nanos: i64,
    device: u64,
    inode: u64,
}
impl Stamp {
    fn read(path: PathBuf) -> Result<Self> {
        let stat = std::fs::metadata(&path)?;
        anyhow::ensure!(stat.is_file(), "resume payload is not a regular file");
        Ok(Self {
            path,
            len: stat.len(),
            mtime: stat.mtime(),
            nanos: stat.mtime_nsec(),
            device: stat.dev(),
            inode: stat.ino(),
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Header {
    hash: String,
    folder: String,
    selection: Option<Vec<usize>>,
    pieces: usize,
    files: Vec<Stamp>,
    size: u64,
    have: u64,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Measurement {
    pub size: u64,
    pub have: u64,
}
pub(super) struct Candidate {
    bytes: Vec<u8>,
    measurement: Measurement,
}
#[derive(Default)]
pub(super) struct Prepared {
    candidates: StdMutex<HashMap<String, Candidate>>,
    pending: StdMutex<HashMap<String, Measurement>>,
}
impl Prepared {
    pub(super) fn install(&self,record:&Record,metadata:&[u8],bytes:Vec<u8>)->Result<()> {
        let measurement=measurement(record,metadata,&bytes)?;
        self.pending.lock().unwrap().insert(record.hash.clone(),measurement);
        self.candidates.lock().unwrap().insert(record.hash.clone(),Candidate{bytes,measurement});
        Ok(())
    }

    pub(super) fn take(&self, hash: &str) -> Option<Vec<u8>> {
        let candidate = self.candidates.lock().unwrap().remove(hash)?;
        Some(candidate.bytes)
    }
    pub(super) fn pending(&self, hash: &str) -> Option<Measurement> {
        self.pending.lock().unwrap().get(hash).copied()
    }
    pub(super) fn finish(&self, hash: &str) {
        self.pending.lock().unwrap().remove(hash);
    }
    pub(super) fn remove(&self, hash: &str) {
        self.candidates.lock().unwrap().remove(hash);
        self.finish(hash);
    }
}
pub(crate) struct Store {
    directory: PathBuf,
    clean: AtomicBool,
    boot_marker: Option<Vec<u8>>,
    io: StdMutex<()>,
    periodic: StdMutex<Instant>,
}
impl Store {
    pub(crate) fn open(directory: &Path) -> Result<Self> {
        let directory = directory.join("resume");
        private_directory(&directory)?;
        let marker = directory.join("CLEAN");
        let boot_marker = boot_marker();
        let clean = boot_marker.as_ref().is_some_and(|boot|
            std::fs::read(&marker).is_ok_and(|bytes| bytes == *boot));
        match std::fs::remove_file(&marker) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("consuming resume CLEAN marker"),
        }
        Ok(Self {
            directory,
            clean: AtomicBool::new(clean),
            boot_marker,
            io: StdMutex::new(()),
            periodic: StdMutex::new(Instant::now()),
        })
    }
    pub(super) fn prepare(&self, root: &Path, records: &[Record]) -> Result<Prepared> {
        let _io = self.io.lock().unwrap();
        let clean = self.clean.swap(false, Ordering::AcqRel);
        let marker = self.directory.join("CLEAN");
        // On an in-process engine restart the previous clean slot created this.
        if marker.exists() {
            if let Err(error) = std::fs::remove_file(&marker)
            {
                return Err(error).context("consuming slot resume marker");
            }
        }
        let prepared = Prepared::default();
        for record in records {
            if record.dormant_on_restore() {
                continue;
            }
            let candidate = clean.then(|| self.load(root, record)).transpose();
            match candidate {
                Ok(Some(candidate)) => {
                    eprintln!("INFO resume trusted hash={}", record.hash);
                    prepared
                        .pending
                        .lock()
                        .unwrap()
                        .insert(record.hash.clone(), candidate.measurement);
                    prepared
                        .candidates
                        .lock()
                        .unwrap()
                        .insert(record.hash.clone(), candidate);
                }
                _ if root.join(format!("{}.torrent", record.hash)).is_file() => eprintln!(
                    "INFO resume check_required hash={} clean={clean}",
                    record.hash
                ),
                _ => {}
            }
        }
        Ok(prepared)
    }
    fn load(&self, root: &Path, record: &Record) -> Result<Candidate> {
        let bytes = read_bounded(&self.path(&record.hash)?)?;
        anyhow::ensure!(
            bytes.starts_with(MAGIC) && bytes.len() >= 16,
            "invalid resume format"
        );
        let length = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
        anyhow::ensure!(length <= bytes.len() - 16, "truncated resume header");
        let end = bytes.len() - 4;
        anyhow::ensure!(
            crc(&bytes[..end]) == u32::from_be_bytes(bytes[end..].try_into().unwrap()),
            "resume checksum mismatch"
        );
        let header: Header = serde_json::from_slice(&bytes[12..12 + length])?;
        anyhow::ensure!(
            header.hash == record.hash
                && header.folder == record.output_folder
                && header.selection == record.only_files,
            "resume identity or selection changed"
        );
        let bits = &bytes[12 + length..end];
        let metadata = read_bounded(&root.join(format!("{}.torrent", record.hash)))?;
        let torrent = librqbit::torrent_from_bytes(&metadata)?;
        anyhow::ensure!(
            torrent.info_hash.as_string() == record.hash,
            "resume metainfo hash mismatch"
        );
        let info = torrent.info.data.validate()?;
        let pieces = info.lengths().total_pieces() as usize;
        validate_bits(bits, pieces)?;
        anyhow::ensure!(pieces == header.pieces, "resume piece count mismatch");
        let expected = stamps(record, &metadata, bits)?;
        anyhow::ensure!(header.files == expected, "resume payload changed");
        let measurement = measurement(record, &metadata, bits)?;
        anyhow::ensure!(
            measurement.size == header.size && measurement.have == header.have,
            "resume measurement mismatch"
        );
        Ok(Candidate {
            bytes: bits.to_vec(),
            measurement,
        })
    }
    fn path(&self, hash: &str) -> Result<PathBuf> {
        ledger::validate_info_hash(hash)?;
        Ok(self.directory.join(format!("{hash}.bin")))
    }
    #[cfg(test)]
    pub(super) fn periodic_elapsed(&self, elapsed: Duration) {
        *self.periodic.lock().unwrap() = Instant::now() - elapsed;
    }
    pub(super) fn invalidate(&self, hash: &str) -> Result<()> {
        let _io = self.io.lock().unwrap();
        match std::fs::remove_file(self.path(hash)?) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
    pub(super) fn snapshot(&self, record: &Record, handle: &ManagedTorrent, _clean: bool) -> Result<()> {
        let bits = handle
            .have_pieces_snapshot()
            .context("torrent has no verified have snapshot")?;
        let metadata = handle
            .metadata
            .load_full()
            .context("torrent has no metadata")?;
        let pieces = metadata.lengths().total_pieces() as usize;
        validate_bits(bits.as_bytes(), pieces)?;
        let value = measurement(record, &metadata.torrent_bytes, bits.as_bytes())?;
        let header = Header {
            hash: record.hash.clone(),
            folder: record.output_folder.clone(),
            selection: record.only_files.clone(),
            pieces,
            files: stamps(record, &metadata.torrent_bytes, bits.as_bytes())?,
            size: value.size,
            have: value.have,
        };
        // Cache trust never crosses a kernel boot. Written, verified pages
        // survive process exit in this boot, so shutdown must not fsync the
        // entire payload (uncancellable under write pressure). A reboot always
        // requires hashing, even if these optional cache files reached disk.
        let header = serde_json::to_vec(&header)?;
        anyhow::ensure!(
            16 + header.len() + bits.as_bytes().len() <= MAX_BYTES as usize,
            "resume snapshot exceeds bound"
        );
        let mut data = Vec::with_capacity(16 + header.len() + bits.as_bytes().len());
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&(header.len() as u32).to_be_bytes());
        data.extend_from_slice(&header);
        data.extend_from_slice(bits.as_bytes());
        data.extend_from_slice(&crc(&data).to_be_bytes());
        atomic_write(&self.path(&record.hash)?, &data)
    }
    fn save(
        &self,
        records: &[Record],
        handles: &HashMap<String, Arc<ManagedTorrent>>,
        clean: bool,
    ) -> Result<()> {
        let _io = self.io.lock().unwrap();
        if !clean && self.clean.load(Ordering::Acquire) {
            return Ok(());
        }
        self.clean.store(false, Ordering::Release);
        for record in records {
            match handles
                .get(&record.hash)
                .filter(|handle| handle.have_pieces_snapshot().is_some())
            {
                Some(handle) => self.snapshot(record, handle, clean)?,
                None => {
                    let path = self.path(&record.hash)?;
                    if path.exists() {
                        std::fs::remove_file(path)?;
                    }
                }
            }
        }
        if clean {
            if let Some(marker) = &self.boot_marker {
                atomic_write(&self.directory.join("CLEAN"), marker)?;
                self.clean.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
}
fn boot_marker() -> Option<Vec<u8>> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let id = id.trim();
    if id.len() != 36 || !id.bytes().enumerate().all(|(i, b)|
        if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }) {
        return None;
    }
    Some(format!("boot-v2 {id}\n").into_bytes())
}
fn private_directory(path: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    drop(file);
    std::fs::rename(temp, path)?;
    Ok(())
}
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "resume input exceeds bound"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "resume input exceeds bound"
    );
    Ok(bytes)
}
fn bit(bits: &[u8], index: usize) -> bool {
    bits.get(index / 8)
        .is_some_and(|byte| byte & (0x80 >> (index % 8)) != 0)
}
fn validate_bits(bits: &[u8], pieces: usize) -> Result<()> {
    anyhow::ensure!(
        bits.len() == pieces.div_ceil(8),
        "resume bitfield length mismatch"
    );
    if pieces % 8 != 0 {
        anyhow::ensure!(
            bits.last()
                .is_none_or(|b| b & ((1 << (8 - pieces % 8)) - 1) == 0),
            "resume bitfield padding"
        );
    }
    Ok(())
}
fn stamps(record: &Record, metadata: &[u8], bits: &[u8]) -> Result<Vec<Stamp>> {
    let torrent = librqbit::torrent_from_bytes(metadata)?;
    let info = torrent.info.data.validate()?;
    info.iter_file_details_ext()
        .enumerate()
        .filter(|(index, file)| {
            record
                .only_files
                .as_ref()
                .is_none_or(|ids| ids.contains(index))
                || file.pieces.clone().any(|piece| bit(bits, piece as usize))
        })
        .filter(|(_, file)| !file.details.attrs().padding)
        .map(|(_, file)| {
            Stamp::read(Path::new(&record.output_folder).join(file.details.filename.to_pathbuf()))
        })
        .collect()
}
fn measurement(record: &Record, metadata: &[u8], bits: &[u8]) -> Result<Measurement> {
    let torrent = librqbit::torrent_from_bytes(metadata)?;
    let info = torrent.info.data.validate()?;
    let lengths = info.lengths();
    let mut selected = vec![0u8; (lengths.total_pieces() as usize).div_ceil(8)];
    for (index, file) in info.iter_file_details_ext().enumerate() {
        if !file.details.attrs().padding
            && record
                .only_files
                .as_ref()
                .is_none_or(|ids| ids.contains(&index))
        {
            for piece in file.pieces {
                let i = piece as usize;
                selected[i / 8] |= 0x80 >> (i % 8);
            }
        }
    }
    let mut value = Measurement { size: 0, have: 0 };
    let length = u64::from(lengths.default_piece_length());
    for i in 0..lengths.total_pieces() as usize {
        if bit(&selected, i) {
            let bytes = length.min(lengths.total_length() - i as u64 * length);
            value.size += bytes;
            if bit(bits, i) {
                value.have += bytes;
            }
        }
    }
    Ok(value)
}

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
