//! Shared tracker projection and safe presentation; private URL identity stays on the socket.
use super::*;
fn unix(value:Option<std::time::SystemTime>)->Option<u64> {
    value.and_then(|v|v.duration_since(std::time::UNIX_EPOCH).ok()).map(|v|v.as_secs())
}
pub(crate) fn wire(row:librqbit::TrackerStat)->Tracker {
    let message=row.message.map(|text|librqbit::sanitize_tracker_message(&row.url,&text));
    Tracker{url:row.url.to_string(),tier:None,status:match row.status {
        librqbit::TrackerStatus::Ok=>TrackerStatus::Ok,librqbit::TrackerStatus::Error=>TrackerStatus::Error,
        librqbit::TrackerStatus::Updating=>TrackerStatus::Updating,librqbit::TrackerStatus::NotContacted=>TrackerStatus::NotContacted,
    },last_announce_at:unix(row.last_announce_at),next_announce_at:unix(row.next_announce_at),peers:row.peers,message}
}
pub(crate) fn public_url(value:&str)->String {
    match url::Url::parse(value) {
        Ok(mut url) if matches!(url.scheme(),"http"|"https"|"udp")=>{let _=url.set_username("");let _=url.set_password(None);url.set_query(None);url.set_fragment(None);url.to_string()},
        _=>"Unsupported tracker URL".into(),
    }
}
pub(crate) fn public_message(value:&str)->String {
    // A dummy URL contributes no credential values; sanitizer still removes
    // userinfo/query/fragment data from every URL token in the text.
    librqbit::sanitize_tracker_message(&url::Url::parse("https://redaction.invalid/").unwrap(),value)
        .replace("?…","")
}
pub(crate) fn public_tracker(mut row:Tracker)->Tracker {
    row.message=row.message.map(|value|match url::Url::parse(&row.url) {
        Ok(url)=>public_message(&librqbit::sanitize_tracker_message(&url,&value)),Err(_)=>public_message(&value),
    });
    row.url=public_url(&row.url);row
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_tracker_keeps_observations_without_any_url_credential() {
        let row=public_tracker(Tracker{url:"https://login:password@tracker.invalid/announce?passkey=private-token#fragment".into(),tier:None,status:TrackerStatus::Error,last_announce_at:None,next_announce_at:None,peers:None,message:Some("login password private-token https://else:else-pass@other.invalid/error?other-secret=secret".into())});
        assert_eq!(row.url,"https://tracker.invalid/announce");
        assert_eq!(row.message.as_deref(),Some("… … … https://other.invalid/error"));
        assert!(row.last_announce_at.is_none() && row.peers.is_none());
    }
}

#[cfg(test)]
#[test]
fn tracker_topic_and_snapshot_are_typed_and_preserve_unknowns() {
    use validation::Validate;
    let topic:Topic=serde_json::from_value(serde_json::json!(format!("torrent:{}.trackers","ab".repeat(20)))).unwrap();
    let row=Tracker{url:"https://tracker.invalid/announce".into(),tier:None,status:TrackerStatus::NotContacted,last_announce_at:None,next_announce_at:None,peers:None,message:None};
    let event=Event::SnapshotChunk{sub:1,topic:topic.clone(),rev:2,items:vec![Item::Tracker(row.clone())]};
    event.validate().unwrap();
    let value=serde_json::to_value(&event).unwrap();
    assert!(value["items"][0].get("peers").is_none());
    assert!(value["items"][0].get("last_announce_at").is_none());
    assert_eq!(serde_json::from_value::<Event>(value).unwrap(),event);
    assert!(serde_json::from_value::<Topic>(serde_json::json!("torrent:bad.trackers")).is_err());
    let wrong=Event::SnapshotChunk{sub:1,topic:Topic::Torrents,rev:2,items:vec![Item::Tracker(row)]};
    assert!(wrong.validate().is_err());
}
