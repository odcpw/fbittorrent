//! Tracker wire aliases retain identity without exporting announce credentials.
use super::*;
use std::{io::Read,sync::OnceLock};
const ID_PREFIX:&str="fbt-tracker=";
pub(crate) fn is_alias(url:&url::Url)->bool {
    let Some(fragment)=url.fragment() else {return false;};
    let mut value=fragment.to_owned();
    for _ in 0..4 {
        if value.to_ascii_lowercase().starts_with(ID_PREFIX) {return true;}
        let next=urlencoding::decode(&value).map(|s|s.into_owned()).unwrap_or_else(|_|value.clone());
        if next==value {break;}value=next;
    }
    // Deeply escaped alias markers are not valid announce fragments either.
    value.contains('%')||value.to_ascii_lowercase().starts_with("fbt-tracker")
}
pub(crate) fn alias(url:&url::Url)->anyhow::Result<String> {
    static KEY:OnceLock<ring::hmac::Key>=OnceLock::new();
    if KEY.get().is_none() {
        let mut bytes=[0;32];std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let _=KEY.set(ring::hmac::Key::new(ring::hmac::HMAC_SHA256,&bytes));
    }
    let tag=ring::hmac::sign(KEY.get().unwrap(),url.as_str().as_bytes());
    let id=tag.as_ref().iter().map(|byte|format!("{byte:02x}")).collect::<String>();
    let mut public=url::Url::parse(&public_url(url.as_str()))
        .unwrap_or_else(|_|url::Url::parse("tracker://unsupported/REDACTED").unwrap());
    if public.as_str().len()>4096 {public.set_path("/REDACTED-LONG-PATH");}
    if public.as_str().len()>4096 {public=url::Url::parse("tracker://unsupported/REDACTED").unwrap();}
    public.set_fragment(Some(&format!("{ID_PREFIX}{id}")));
    Ok(public.into())
}
fn unix(value:Option<std::time::SystemTime>)->Option<u64> {
    value.and_then(|v|v.duration_since(std::time::UNIX_EPOCH).ok()).map(|v|v.as_secs())
}
pub(crate) fn wire(row:librqbit::TrackerStat)->anyhow::Result<Tracker> {
    let message=row.message.map(|text|librqbit::sanitize_tracker_message(&row.url,&text));
    Ok(Tracker{url:alias(&row.url)?,tier:None,status:match row.status {
        librqbit::TrackerStatus::Ok=>TrackerStatus::Ok,librqbit::TrackerStatus::Error=>TrackerStatus::Error,
        librqbit::TrackerStatus::Updating=>TrackerStatus::Updating,librqbit::TrackerStatus::NotContacted=>TrackerStatus::NotContacted,
    },last_announce_at:unix(row.last_announce_at),next_announce_at:unix(row.next_announce_at),peers:row.peers,message})
}
pub(crate) fn public_url(value:&str)->String {
    match url::Url::parse(value) {
        Ok(url) if matches!(url.scheme(),"http"|"https"|"udp")=>librqbit::redact_tracker_url(&url).split('?').next().unwrap().to_owned(),
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
    fn wire_hides_paths_and_keeps_distinct_process_scoped_identities() {
        let a=url::Url::parse("https://tracker.invalid/0123456789abcdef0123456789abcdef/announce?token=abc").unwrap();
        let b=url::Url::parse("https://tracker.invalid/FEDCBA9876543210FEDCBA9876543210/announce?token=xyz").unwrap();
        let first=wire(librqbit::TrackerStat::not_contacted(a.clone())).unwrap();
        let second=wire(librqbit::TrackerStat::not_contacted(b)).unwrap();
        assert_ne!(first.url,second.url);assert_eq!(first.url,alias(&a).unwrap());
        assert_eq!(public_url(&first.url),public_url(&second.url));
        for secret in ["0123456789abcdef0123456789abcdef","FEDCBA9876543210FEDCBA9876543210","token=abc","token=xyz"] {
            assert!(!serde_json::to_string(&[&first,&second]).unwrap().contains(secret));
        }
        for path in ["passkey%2Fabc/announce","passkey%252Fabc/announce","abc123passkey/announce"] {
            let row=wire(librqbit::TrackerStat::not_contacted(url::Url::parse(&format!("https://tracker.invalid/{path}")).unwrap())).unwrap();
            assert_eq!(public_url(&row.url),"https://tracker.invalid/REDACTED/announce");
        }
        assert!(first.peers.is_none());assert!(is_alias(&url::Url::parse(&first.url).unwrap()));
        let long=url::Url::parse(&format!("https://tracker.invalid/{}announce", "short/".repeat(700))).unwrap();
        assert!(alias(&long).unwrap().len()<8192);
        let unsupported=wire(librqbit::TrackerStat::not_contacted(url::Url::parse("ftp://tracker.invalid/SECRET_CREDENTIAL_PATH").unwrap())).unwrap();
        assert_eq!(public_url(&unsupported.url),"Unsupported tracker URL");assert!(!unsupported.url.contains("SECRET_CREDENTIAL_PATH"));
    }
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
