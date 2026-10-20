//! Saved metainfo and payload presence for rows without a live handle.
//!
//! The info dictionary is bound to its info hash, so one parse per hash is
//! reused until the `.torrent` file itself changes (one `stat` per lookup).
//! Payload presence is a cheap `stat` of the selected files: explicit reads
//! (`Info`, `Get`) always stat, the shared sampler at most every
//! [`PRESENCE_TTL`]. Either way 'Files missing' is derived from the disk, never
//! guessed, and a listing never creates or hashes payload.
use super::*;
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

/// Sampler cadence for payload presence of rows that are not transferring.
pub(crate) const PRESENCE_TTL: Duration = Duration::from_secs(30);

pub(super) type Stamp = (u64, u64, u64, i64, i64);
fn stamp(metadata: &std::fs::Metadata) -> Stamp {
    (metadata.dev(), metadata.ino(), metadata.len(), metadata.mtime(), metadata.mtime_nsec())
}
/// Identity of a file replaced by atomic rename; None when it is absent.
pub(super) fn file_stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).ok().map(|metadata| stamp(&metadata))
}

pub(super) struct SavedFile {
    pub name: String,
    pub path: PathBuf,
    pub len: u64,
    pub padding: bool,
    pub pieces: std::ops::Range<u32>,
}

/// The parts of a validated info dictionary that listings need.
pub(super) struct SavedInfo {
    pub files: Vec<SavedFile>,
    pub piece_length: u64,
    pub total_length: u64,
}

impl SavedInfo {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let torrent = librqbit::torrent_from_bytes(bytes)?;
        let info = torrent.info.data.validate()?;
        let lengths = info.lengths();
        Ok(Self {
            files: info
                .iter_file_details_ext()
                .map(|file| {
                    let path = file.details.filename.to_pathbuf();
                    SavedFile {
                        name: path.to_string_lossy().into(),
                        path,
                        len: file.details.len,
                        padding: file.details.attrs().padding,
                        pieces: file.pieces.clone(),
                    }
                })
                .collect(),
            piece_length: u64::from(lengths.default_piece_length()),
            total_length: lengths.total_length(),
        })
    }

    /// The saved file list and librqbit's selected byte total for a record.
    pub(super) fn selection(&self, record: &Record) -> (Vec<TorrentFile>, u64) {
        let mut selected_end = 0;
        let mut selected_bytes = 0;
        let files = self
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let selected = record.only_files.as_ref().is_none_or(|ids| ids.contains(&index));
                // Match librqbit's compute_selected_pieces / hns.total(): the
                // union of non-padding selected files' pieces, not file sizes.
                // Metainfo order gives increasing ranges; count shared boundary
                // pieces once and clip the final piece to its actual length.
                let start = file.pieces.start.max(selected_end);
                if selected && !file.padding && start < file.pieces.end {
                    let end_bytes = (u64::from(file.pieces.end) * self.piece_length).min(self.total_length);
                    selected_bytes += end_bytes - u64::from(start) * self.piece_length;
                    selected_end = file.pieces.end;
                }
                TorrentFile {
                    index: index as i64,
                    name: file.name.clone(),
                    size: file.len as i64,
                    progress: if record.completion_on > 0 { 1.0 } else { 0.0 },
                    // This is the saved completion claim, not a measurement of data.
                    progress_known: false,
                    priority: if selected { 1 } else { 0 },
                }
            })
            .collect();
        (files, selected_bytes)
    }

    /// Every selected, non-padding file is a regular file of its full length.
    fn present(&self, record: &Record) -> bool {
        self.files.iter().enumerate().all(|(index, file)| {
            let selected = record.only_files.as_ref().is_none_or(|ids| ids.contains(&index));
            !selected
                || file.padding
                || std::fs::metadata(Path::new(&record.output_folder).join(&file.path))
                    .is_ok_and(|m| m.is_file() && m.len() == file.len)
        })
    }
}

struct Presence {
    folder: String,
    selection: Option<Vec<usize>>,
    checked: Instant,
    present: bool,
}

struct LivePresence {
    folder: PathBuf,
    selection: Option<Vec<usize>>,
    checked: Instant,
    missing: bool,
}

/// Owned by one engine or parked ledger; dropped with it.
#[derive(Default)]
pub(crate) struct SavedCache {
    metadata: StdMutex<HashMap<String, (Stamp, std::result::Result<Arc<SavedInfo>, String>)>>,
    presence: StdMutex<HashMap<String, Presence>>,
    live: StdMutex<HashMap<String, LivePresence>>,
}

impl SavedCache {
    /// Validated saved metainfo for `record`, or None when no file exists.
    pub(super) fn info(&self, record: &Record, directory: &Path) -> Result<Option<Arc<SavedInfo>>> {
        let path = directory.join(format!("{}.torrent", record.hash));
        let current = match std::fs::metadata(&path) {
            Ok(metadata) => stamp(&metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.metadata.lock().unwrap().remove(&record.hash);
                return Ok(None);
            }
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        if let Some((seen, parsed)) = self.metadata.lock().unwrap().get(&record.hash) {
            if *seen == current {
                return parsed.clone().map(Some).map_err(|message| anyhow!(message));
            }
        }
        // parked_metadata validates the bytes against this record's hash.
        let parsed = match parked::parked_metadata(record, directory) {
            Ok(None) => return Ok(None),
            Ok(Some(bytes)) => SavedInfo::parse(&bytes).map(Arc::new).map_err(|e| format!("{e:#}")),
            Err(error) => Err(format!("{error:#}")),
        };
        self.metadata.lock().unwrap().insert(record.hash.clone(), (current, parsed.clone()));
        parsed.map(Some).map_err(|message| anyhow!(message))
    }

    /// Payload presence; `fresh` always stats, otherwise at most every TTL.
    pub(super) fn present(&self, record: &Record, info: &SavedInfo, fresh: bool) -> bool {
        let mut presence = self.presence.lock().unwrap();
        if let Some(seen) = presence.get(&record.hash) {
            if !fresh
                && seen.folder == record.output_folder
                && seen.selection == record.only_files
                && seen.checked.elapsed() < PRESENCE_TTL
            {
                return seen.present;
            }
        }
        let present = info.present(record);
        presence.insert(record.hash.clone(), Presence {
            folder: record.output_folder.clone(),
            selection: record.only_files.clone(),
            checked: Instant::now(),
            present,
        });
        present
    }
}

impl SavedCache {
    /// A handle claims bytes of a selected file that is gone from disk (or is
    /// no longer a regular file, or is shorter than a complete file). Files
    /// with no downloaded bytes are not expected to exist yet. Like dormant
    /// rows: explicit reads stat now, the sampler at most every PRESENCE_TTL.
    pub(super) fn live_missing(&self, hash: &str, handle: &ManagedTorrent, fresh: bool) -> bool {
        let folder = handle.output_folder();
        let selection = handle.only_files();
        if let Some(seen) = self.live.lock().unwrap().get(hash) {
            if !fresh && seen.folder == folder && seen.selection == selection && seen.checked.elapsed() < PRESENCE_TTL {
                return seen.missing;
            }
        }
        let missing = handle.metadata.load_full().is_some_and(|metadata| {
            let have = handle.stats().file_progress;
            metadata.file_infos.iter().enumerate().any(|(index, file)| {
                let bytes = have.get(index).copied().unwrap_or(0);
                selection.as_ref().is_none_or(|ids| ids.contains(&index))
                    && !file.attrs.padding
                    && bytes > 0
                    && match std::fs::metadata(folder.join(&file.relative_filename)) {
                        Ok(found) => !found.is_file() || (bytes >= file.len && found.len() < file.len),
                        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                    }
            })
        });
        self.live.lock().unwrap().insert(hash.into(), LivePresence {
            folder: folder.to_path_buf(),
            selection,
            checked: Instant::now(),
            missing,
        });
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, Record) {
        let root = std::env::temp_dir().join(format!("fbt-saved-{name}-{}-{}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("downloads")).unwrap();
        let mut bytes = b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f,
            0x3b, 0x48, 0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
        std::fs::write(root.join(format!("{hash}.torrent")), &bytes).unwrap();
        std::fs::write(root.join("downloads/hello.txt"), b"hello").unwrap();
        let mut record = super::super::tests::review_record(&hash, &root.join("downloads"));
        record.completion_on = 1;
        (root, record)
    }

    #[test]
    fn metadata_is_parsed_once_until_the_file_changes() {
        let (root, record) = fixture("parse-once");
        let cache = SavedCache::default();
        let first = cache.info(&record, &root).unwrap().unwrap();
        let second = cache.info(&record, &root).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &second), "an unchanged .torrent is not reparsed");
        assert_eq!((first.total_length, first.files.len(), first.files[0].name.as_str()), (5, 1, "hello.txt"));
        let path = root.join(format!("{}.torrent", record.hash));
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(cache.info(&record, &root).unwrap().is_none(), "a removed file is unavailable at once");
        std::fs::write(root.join("staged"), &bytes).unwrap();
        std::fs::rename(root.join("staged"), &path).unwrap();
        assert!(!Arc::ptr_eq(&first, &cache.info(&record, &root).unwrap().unwrap()), "a replaced file is reparsed");
        // A corrupt replacement is an error each time, without a reread.
        std::fs::write(root.join("staged"), b"not bencode").unwrap();
        std::fs::rename(root.join("staged"), &path).unwrap();
        assert!(cache.info(&record, &root).is_err());
        assert!(cache.info(&record, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sampled_presence_is_bounded_and_explicit_reads_stat() {
        let (root, mut record) = fixture("presence");
        let cache = SavedCache::default();
        let info = cache.info(&record, &root).unwrap().unwrap();
        assert!(cache.present(&record, &info, false));
        std::fs::remove_file(root.join("downloads/hello.txt")).unwrap();
        assert!(cache.present(&record, &info, false), "the sampler reuses a recent observation");
        assert!(!cache.present(&record, &info, true), "an explicit read stats the disk");
        assert!(!cache.present(&record, &info, false), "and refreshes the sampled value");
        std::fs::write(root.join("downloads/hello.txt"), b"hello").unwrap();
        record.only_files = Some(vec![0]);
        assert!(cache.present(&record, &info, false), "a changed selection is a new observation");
        std::fs::write(root.join("downloads/hello.txt"), b"hi").unwrap();
        record.output_folder = root.join("elsewhere").to_string_lossy().into();
        assert!(!cache.present(&record, &info, false), "a moved record is a new observation");
        std::fs::remove_dir_all(root).unwrap();
    }
}
