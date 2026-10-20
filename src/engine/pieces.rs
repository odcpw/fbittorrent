//! In-memory diagnostics only: no runtime task, payload reads or state admission.
use super::*;

pub(crate) struct Snapshot {
    pub count: u64,
    pub piece_size: u64,
    pub have: Vec<u8>,
    pub priority: Vec<u64>,
    pub file_offsets:Vec<u64>,
}
impl EngineDispatcher {
    pub(crate) fn pieces_snapshot(&self, hash: &str) -> Result<Snapshot> {
        let handle=self.inner.handle(hash)?;
        let metadata=handle.metadata.load_full().context("torrent metadata is unavailable")?;
        let bits=handle.have_pieces_snapshot().context("verified pieces are unavailable while checking")?;
        let included=handle.only_files();
        let mut file_offsets=Vec::with_capacity(metadata.file_infos.len());
        let mut priority=std::collections::BTreeSet::new();
        if let Some(range) = handle.metadata_priority() {
            anyhow::ensure!(range.len() <= crate::api::LINE_OUT / 24, "piece priority snapshot exceeds the API frame limit");
            priority.extend(range.map(u64::from));
        }
        for (index,file) in metadata.info.iter_file_details_ext().enumerate() {
            file_offsets.push(file.offset);
            if included.as_ref().is_some_and(|files| !files.contains(&index)) || file.details.attrs().padding || file.pieces.is_empty() {continue;}
            priority.insert(u64::from(file.pieces.start));
            priority.insert(u64::from(file.pieces.end-1));
        }
        Ok(Snapshot {count:u64::from(metadata.lengths().total_pieces()),
            piece_size:u64::from(metadata.lengths().default_piece_length()),have:bits.as_bytes().to_vec(),priority:priority.into_iter().collect(),file_offsets})
    }
}
