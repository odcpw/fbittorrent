//! Additive contract-1.x view of the normalized API model.
use crate::{api, qbt};
use serde_json::{json, Value};

pub fn torrent(t: &api::Torrent) -> Value {
    let state = match t.state {
        api::TorrentState::Metadata => "metaDL",
        api::TorrentState::Checking => "checkingDL",
        api::TorrentState::WaitingVpn if t.intent == api::Intent::Pause => {
            if t.completed_at.is_some() {
                "stoppedUP"
            } else {
                "stoppedDL"
            }
        }
        api::TorrentState::WaitingVpn => "waitingVPN",
        api::TorrentState::Queued => "queuedDL",
        api::TorrentState::Paused => {
            if t.completed_at.is_some() {
                "stoppedUP"
            } else {
                "stoppedDL"
            }
        }
        api::TorrentState::Downloading => "downloading",
        api::TorrentState::Stalled => "stalledDL",
        api::TorrentState::Done => "stoppedUP",
        api::TorrentState::Seeding => "uploading",
        api::TorrentState::Error => "error",
    };
    let mut value = json!({"hash":t.hash,"name":t.name,"state":state,"progress":t.progress.unwrap_or(0.0),"progress_known":t.progress.is_some(),
        "size":t.size.unwrap_or(0),"completed":t.completed_bytes.unwrap_or(0),"dlspeed":t.dl_bps.unwrap_or(0),"upspeed":t.ul_bps.unwrap_or(0),
        "eta":t.eta_s.unwrap_or(8640000),"num_seeds":t.seeds.unwrap_or(0),"num_leechs":t.peers.unwrap_or(0),"ratio":t.ratio.unwrap_or(0.0),
        "seq_dl":true,"f_l_piece_prio":true,"save_path":t.save_path,"content_path":t.content_path.as_deref().unwrap_or(&t.save_path),"category":"",
        "added_on":t.added_at,"completion_on":t.completed_at.unwrap_or(0),"up_limit":t.upload_limit_bps.unwrap_or(0),"ratio_limit":if t.seed{-2.0}else{0.0},"tags":if t.seed {qbt::SEED_TAG}else{""},
        "native_state":t.state,"checked":t.checked});
    let object = value.as_object_mut().unwrap();
    for (key, field) in [
        ("reason", json!(t.reason)),
        ("error_message", json!(t.error)),
        ("origin", json!(t.origin)),
        ("eta_s", json!(t.eta_s)),
        ("seeds", json!(t.seeds)),
        ("playable", json!(t.playable)),
    ] {
        if !field.is_null() {
            object.insert(key.into(), field);
        }
    }
    value
}
pub fn rows(values: &[api::Torrent]) -> Vec<Value> {
    values.iter().map(torrent).collect()
}
pub fn legacy(values: &[api::Torrent]) -> anyhow::Result<Vec<qbt::Torrent>> {
    Ok(values
        .iter()
        .map(|t| serde_json::from_value(torrent(t)))
        .collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_values_preserve_legacy_sentinels_only() {
        let text = include_str!("../../contracts/ipc/v1/deferred-add.ndjson");
        let v: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        let t: api::Torrent =
            serde_json::from_value(v["frame"]["result"]["torrent"].clone()).unwrap();
        let row = torrent(&t);
        assert_eq!(row["state"], "waitingVPN");
        assert_eq!(row["native_state"], "waiting_vpn");
        assert_eq!(row["progress"], 0.0);
        assert_eq!(row["progress_known"], false);
        assert_eq!(row["eta"], 8640000);
        assert!(row.get("eta_s").is_none());
        let old: qbt::Torrent = serde_json::from_value(row).unwrap();
        assert_eq!(old.hash, t.hash.to_string());
    }
}
