//! Snapshot owned verified pieces on the engine runtime; no tracker guard escapes.
use super::*;
use crate::daemon::watch::{self, Sample};
impl EngineDispatcher {
    pub(crate) fn prioritize_metadata(
        &self, hash: &str, file: usize, range: Option<std::ops::Range<u64>>,
    ) -> Result<()> {
        self.inner.handle(hash)?.prioritize_metadata(file, range)
    }
    pub(crate) fn watch_samples(&self) -> Result<Vec<Sample>> {
        self.runtime.block_on(async {
            let ledger =
                self.observation_ledger().await?;
            let mut result = Vec::new();
            for record in &ledger.records {
                if record.staged {
                    continue;
                }
                let Ok(handle) = self.inner.handle(&record.hash) else {
                    continue;
                };
                let Some(metadata) = handle.metadata.load_full() else {
                    continue;
                };
                let selected = handle.only_files();
                let Some(index) = watch::largest_video(
                    metadata
                        .file_infos
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| selected.as_ref().is_none_or(|ids| ids.contains(i)))
                        .map(|(i, f)| (i, f.relative_filename.as_path(), f.len)),
                ) else {
                    continue;
                };
                let stats = handle.stats();
                if stats.finished || matches!(stats.state, TorrentStatsState::Error) {
                    continue;
                }
                let file = &metadata.file_infos[index];
                let Some(bits) = handle.have_pieces_snapshot() else {
                    continue;
                };
                let pieces = bits.as_slice();
                let piece = u64::from(metadata.lengths().default_piece_length());
                let prefix = watch::prefix(file.offset_in_torrent, file.len, piece, |i| {
                    pieces.get(i).is_some_and(|b| *b)
                });
                let last = file
                    .piece_range
                    .end
                    .checked_sub(1)
                    .is_some_and(|i| pieces.get(i as usize).is_some_and(|b| *b));
                let rate = stats
                    .live
                    .as_ref()
                    .map(|s| s.download_speed.mbps * 1024.0 * 1024.0)
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| v as u64);
                let active = !handle.is_paused();
                let path = handle.output_folder().join(&file.relative_filename);
                let verified = watch::iso_movie(&path).then_some(watch::VerifiedPieces {
                    bits, offset: file.offset_in_torrent, piece,
                });
                result.push(Sample {
                    hash: record.hash.clone(),
                    name: record.name.clone(),
                    file: index as u64,
                    path,
                    size: file.len,
                    prefix,
                    verified_bytes: stats.progress_bytes,
                    last,
                    dl_bps: rate,
                    downloading: active && rate.is_some_and(|v| v > 0),
                    active,
                    download_limit: handle.effective_download_bps(),
                    verified,
                });
            }
            Ok(result)
        })
    }
}
