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
        "native_state":t.state,"checked":t.checked,"staged":t.staged});
    let object = value.as_object_mut().unwrap();
    for (key, field) in [
        ("reason", json!(t.reason)),
        ("error_message", json!(t.error)),
        ("origin", json!(t.origin)),
        ("eta_s", json!(t.eta_s)),
        ("seeds", json!(t.seeds)),
        ("download_limit_bps",json!(t.download_limit_bps)),
        ("effective_download_limit_bps",json!(t.effective_download_limit_bps)),
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

/// Keep the selected result's legacy engine name while retaining every deduped provider.
pub(crate) fn search_legacy(item:&api::SearchItem,providers:&[api::Provider]) -> qbt::SearchResult {
    let mut row=crate::search::normalize::legacy(item);
    if let Some(primary)=item.providers.iter().find(|name|providers.iter().any(|p|&p.name==*name && p.url.as_ref().is_some_and(|url|Some(url)==item.provider_url.as_ref()))) {
        row.engine=primary.clone();
    }
    row
}
pub(crate) fn search(item:&api::SearchItem,providers:&[api::Provider]) -> Value {
    let mut value=serde_json::to_value(search_legacy(item,providers)).expect("legacy search fields serialize");
    value["providers"]=json!(item.providers);
    value
}
pub(crate) fn providers(values:&[api::Provider]) -> Vec<qbt::SearchPlugin> {
    values.iter().map(|p|qbt::SearchPlugin {kind:if p.kind==api::ProviderKind::Torznab {"torznab"}else{"nova"}.into(),name:p.name.clone(),full_name:p.full_name.clone().unwrap_or_default(),
        enabled:p.enabled,url:p.url.clone().unwrap_or_default(),version:p.version.clone().unwrap_or_default()}).collect()
}

pub(crate) fn imdb_legacy(card:&api::ImdbCard) -> crate::imdb::Title {
    crate::imdb::Title {id:card.id.clone(),name:card.title.clone(),year:card.year,kind:card.kind.clone(),
        stars:card.stars.clone().unwrap_or_default(),poster_url:card.poster_url.clone().unwrap_or_default(),
        rating:card.rating.map(|v|v as f32),votes:card.votes,blurb:card.blurb.clone().unwrap_or_default(),wiki_url:card.wiki_url.clone().unwrap_or_default()}
}
pub(crate) fn imdb(card:&api::ImdbCard) -> Value {
    let mut value=serde_json::to_value(imdb_legacy(card)).expect("IMDb legacy fields serialize");
    value["ok"]=json!(true);value["found"]=json!(true);value["url"]=json!(card.url);value["poster_path"]=json!(card.poster_path);value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_preserves_characterized_legacy_fields_and_provider_provenance() {
        let before:Value=serde_json::from_str(include_str!("../../agent_ergonomics_audit/audit/regression_tests/golden/cli/search-212c3cd.json")).unwrap();
        let rows:Vec<qbt::SearchResult>=serde_json::from_value(before["results"].clone()).unwrap();
        let items=crate::search::normalize::aggregate(&rows,5000);
        let providers=vec![api::Provider {name:"shape".into(),kind:api::ProviderKind::Nova,enabled:true,full_name:Some("Shape provider".into()),url:Some("https://shape.invalid".into()),version:None}];
        let shaped:Vec<_>=items.iter().map(|item|{
            let mut row=search(item,&providers);assert_eq!(row["providers"],json!(["shape"]));row.as_object_mut().unwrap().remove("providers");row
        }).collect();
        assert_eq!(json!(shaped),before["results"]);
        let mut item=items[0].clone();item.providers=vec!["alpha".into(),"shape".into()];
        assert_eq!(search_legacy(&item,&providers).engine,"shape","engineName follows the selected result source, not alphabetical provenance");
        println!("every characterized pre-P3-07 field unchanged; additive sorted providers retained; winning provider identity preserved");
    }
    #[test]
    fn imdb_roundtrip_retains_legacy_required_and_nullable_fields() {
        let old=crate::imdb::Title {id:"tt1".into(),name:"Fixture".into(),year:None,rating:Some(8.2),..Default::default()};
        let card:api::ImdbCard=serde_json::from_value(json!({"id":"tt1","title":"Fixture","kind":"","url":old.url(),"rating":8.2})).unwrap();
        let new=imdb(&card);let mut expected=serde_json::to_value(old).unwrap();
        expected["ok"]=json!(true);expected["found"]=json!(true);expected["url"]=json!("https://www.imdb.com/title/tt1/");expected["poster_path"]=Value::Null;
        assert_eq!(new,expected);println!("IMDb legacy floats, empty strings, absent-source nulls and key names unchanged");
    }
    #[test]
    #[ignore = "process-level live CLI shape bridge; requires private input/output paths"]
    fn live_shape_bridge() {
        let input = std::env::var("FBT_TEST_SHAPES_INPUT").unwrap();
        let output = std::env::var("FBT_TEST_SHAPES_OUTPUT").unwrap();
        let values: Vec<api::Torrent> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
        std::fs::write(output,serde_json::to_vec(&rows(&values)).unwrap()).unwrap();
    }
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
