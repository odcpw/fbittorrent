//! Read verified in-memory pieces; an absent snapshot is never fabricated.
use super::{ApiResult, error, engine_error};
use crate::{api::*, daemon::{Daemon, slot::Slot}, engine::ops::{EngineOp,EngineResult}};
use base64::{engine::general_purpose::STANDARD, Engine};

pub(crate) fn read(daemon:&Daemon, hash:&Hash)->ApiResult<Pieces> {
    // Establish ledger identity first, including parked/dormant records.
    let EngineResult::Detail(detail)=daemon.execute(EngineOp::Get {hash:hash.to_string()}).map_err(|e|{let mut failure=engine_error(e);if failure.code==ErrorCode::NotFound {failure.details=Some(serde_json::Map::from_iter([("hash".into(),serde_json::json!(hash))]));}failure})? else {unreachable!("detail result")};
    let slot=daemon.slot.read().unwrap_or_else(|e|e.into_inner());
    let Slot::Running(app)=&slot.slot else {return Err(error(ErrorCode::InvalidState,"verified pieces unavailable without a live engine",None));};
    let snapshot=app.dispatcher().pieces_snapshot(hash.as_str()).map_err(|_|error(ErrorCode::InvalidState,"verified pieces unavailable until a live torrent finishes checking",None))?;
    let mut torrent=super::torrents::torrent(detail.torrent)?;
    daemon.watch.lock().unwrap().decorate(std::slice::from_mut(&mut torrent));
    encode(snapshot,torrent.playable.as_ref())
}
pub(crate) fn encode(snapshot:crate::engine::pieces::Snapshot,playable:Option<&Playable>)->ApiResult<Pieces> {
    if snapshot.have.len()>LINE_OUT*3/4 {return Err(error(ErrorCode::Unsupported,"piece snapshot exceeds the API frame limit",None));}
    let playable_end=playable.and_then(|p|snapshot.file_offsets.get(p.file as usize).and_then(|offset|offset.checked_add(p.prefix_bytes))).map(|bytes|bytes.div_ceil(snapshot.piece_size));
    let pieces=Pieces {count:snapshot.count,piece_size:snapshot.piece_size,have:STANDARD.encode(snapshot.have),priority:snapshot.priority,
        playable_end};
    use validation::Validate;
    pieces.validate().map_err(|_|error(ErrorCode::Internal,"invalid verified piece snapshot",None))?;
    if serde_json::to_vec(&pieces).map_err(|_|error(ErrorCode::Internal,"piece encoding failed",None))?.len()+256 > LINE_OUT {
        return Err(error(ErrorCode::Unsupported,"piece snapshot exceeds the API frame limit",None));
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playable_end_uses_the_selected_files_absolute_offset() {
        let snapshot=crate::engine::pieces::Snapshot {count:4,piece_size:16384,have:vec![0xf0],priority:vec![0,3],file_offsets:vec![0,32768]};
        let playable=Playable {file:1,ready:false,prefix_bytes:4096,acked:false,stall_free:None,buffered_s:None,finishes_in_s:None};
        assert_eq!(encode(snapshot,Some(&playable)).unwrap().playable_end,Some(3));
    }
}
