use super::*;
use serde_json::json;
use std::path::Path;

fn projection(store: &Store) -> Value {
    json!({"proto":store.hello.as_ref().map(|hello|hello.proto),
        "compatible":store.hello.as_ref().map(|hello|hello.proto==api::PROTO),
        "torrents":store.torrents,"summary":store.summary,"network":store.network,
        "settings":store.settings,"subscriptions":store.subscriptions})
}
fn first_difference(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    if actual == expected {
        return None;
    }
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let pointer = format!("{}/{}", path, key.replace('~', "~0").replace('/', "~1"));
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(diff) = first_difference(a, b, &pointer) {
                            return Some(diff);
                        }
                    }
                    (a, b) => return Some(format!("{pointer}: actual={a:?}, expected={b:?}")),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for index in 0..a.len().max(b.len()) {
                let pointer = format!("{path}/{index}");
                match (a.get(index), b.get(index)) {
                    (Some(a), Some(b)) => {
                        if let Some(diff) = first_difference(a, b, &pointer) {
                            return Some(diff);
                        }
                    }
                    (a, b) => return Some(format!("{pointer}: actual={a:?}, expected={b:?}")),
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: actual={actual}, expected={expected}"))
}
fn compare(store: &Store, expected: &Value) -> Result<()> {
    let actual = projection(store);
    for (key, value) in expected.as_object().unwrap() {
        let Some(found) = actual.get(key) else {
            bail!("/{key}: missing mirror field")
        };
        if let Some(diff) = first_difference(found, value, &format!("/{key}")) {
            bail!("{diff}");
        }
    }
    Ok(())
}
fn replay(text: &str) -> Result<(Store, usize)> {
    let mut store = Store::default();
    let mut requests = BTreeMap::<u64, api::Request>::new();
    let mut expects = 0;
    for (index, line) in text.lines().enumerate() {
        let value: Value = serde_json::from_str(line)?;
        let step = (|| -> Result<()> {
            match value["dir"].as_str().unwrap() {
                "c2s" => {
                    let request: api::Request = serde_json::from_value(value["frame"].clone())?;
                    requests.insert(request.id.get(), request);
                }
                "s2c" => {
                    let frame = &value["frame"];
                    if frame.get("ev").is_some() {
                        let event: api::Event = serde_json::from_value(frame.clone())?;
                        println!("Msg::Api({}) at fixture frame {}", event.name(), index + 1);
                        store.event(&event)?;
                    } else if frame["ok"] == true {
                        let call = &requests[&frame["id"].as_u64().unwrap()].call;
                        let reply = api::Reply::decode(call.op(), frame["result"].clone())?;
                        store.reply(call, reply)?;
                    }
                }
                "expect" => {
                    compare(&store, &value["model"])?;
                    expects += 1;
                }
                "volatile" => {}
                other => bail!("unknown fixture direction {other}"),
            }
            Ok(())
        })();
        step.map_err(|error| anyhow::anyhow!("frame {}: {error:#}", index + 1))?;
    }
    Ok((store, expects))
}
fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/ipc/v1")
            .join(name),
    )
    .unwrap()
}
#[test]
fn fixtures() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/ipc/v1");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|path| path.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ndjson")
        })
        .collect();
    paths.sort();
    let mut total = 0;
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        println!("replay {}", path.display());
        let (_, expects) =
            replay(&text).unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        println!("matched {expects} expect models: {}", path.display());
        total += expects;
    }
    assert_eq!(total, 9, "every frozen expect model must be exercised");
}
#[test]
fn poisoned_expect_reports_the_first_json_pointer() {
    let mut text = fixture("subscribe-gap.ndjson");
    text = text.replace("\"resync\":true", "\"resync\":false");
    let error = replay(&text).err().unwrap().to_string();
    assert!(error.contains("/subscriptions/1/resync"), "{error}");
    let text = fixture("settings-motion.ndjson").replace(
        "\"reduce_motion\":true,\"restart_class\"",
        "\"reduce_motion\":false,\"restart_class\"",
    );
    // Both state and expect lines changed: explicitly poison only the first true expectation.
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    let index = lines
        .iter()
        .position(|line| {
            line.contains("\"dir\":\"expect\"") && line.contains("\"reduce_motion\":false")
        })
        .unwrap();
    lines[index] = lines[index].replace("\"reduce_motion\":false", "\"reduce_motion\":true");
    let error = replay(&lines.join("\n")).err().unwrap().to_string();
    assert!(error.contains("/settings/reduce_motion"), "{error}");
}
#[test]
fn snapshots_commit_atomically_and_gaps_preserve_confirmed_rows() {
    let text = fixture("subscribe-gap.ndjson");
    let lines: Vec<_> = text.lines().collect();
    let (mut store, _) = replay(&lines[..7].join("\n")).unwrap();
    assert!(
        store.torrents.is_empty(),
        "a partial snapshot must not be rendered"
    );
    let end: Value = serde_json::from_str(lines[7]).unwrap();
    store
        .event(&serde_json::from_value(end["frame"].clone()).unwrap())
        .unwrap();
    assert_eq!(store.torrents.len(), 1);
    let before = store.torrents.clone();
    let gap: Value = serde_json::from_str(lines[8]).unwrap();
    let effect = store
        .event(&serde_json::from_value(gap["frame"].clone()).unwrap())
        .unwrap();
    assert!(matches!(effect, Effect::Resubscribe { sub: 1, .. }));
    assert_eq!(store.torrents, before);
    assert_eq!(store.subscriptions[&1].rev, 3);
    assert!(store.subscriptions[&1].resync);
}
#[test]
fn null_patches_remove_unknown_values_and_invalid_patch_is_atomic() {
    let (mut store, _) = replay(&fixture("subscribe-gap.ndjson")).unwrap();
    let hash = store.torrents.keys().next().unwrap().clone();
    let delta = json!({"ev":"delta","sub":2,"topic":"torrents","rev":4,
        "changed":{hash.clone():{"eta_s":null,"dl_bps":null,"ul_bps":null,"peers":null}}});
    store
        .event(&serde_json::from_value(delta).unwrap())
        .unwrap();
    assert!(store.torrents[&hash].eta_s.is_none());
    assert!(store.torrents[&hash].dl_bps.is_none());
    let before = store.torrents.clone();
    let bad = json!({"ev":"delta","sub":2,"topic":"torrents","rev":5,
        "changed":{hash.clone():{"name":"should not commit","progress":2.0}}});
    assert!(store.event(&serde_json::from_value(bad).unwrap()).is_err());
    assert_eq!(store.torrents, before);
    assert_eq!(store.subscriptions[&2].rev, 4);
}
#[test]
fn instance_change_keeps_stale_rows_until_a_new_snapshot() {
    let (mut store, _) = replay(&fixture("subscribe-gap.ndjson")).unwrap();
    let before = store.torrents.clone();
    let mut hello = store.hello.clone().unwrap();
    hello.instance = api::Instance::new("f".repeat(32)).unwrap();
    store.hello(hello);
    assert_eq!(store.torrents, before);
    assert!(store.subscriptions.is_empty());
    assert!(store.snapshots.is_empty());
    store.disconnected();
    assert!(!store.connected);
}
#[test]
fn snapshot_bounds_and_incomplete_snapshot_cannot_destroy_confirmed_rows() {
    let (mut store, _) = replay(&fixture("subscribe-gap.ndjson")).unwrap();
    let before = store.torrents.clone();
    let event = json!({"ev":"snapshot.begin","sub":3,"topic":"torrents","rev":1,"count":MAX_ITEMS+1,"reason":"subscribe"});
    assert!(store
        .event(&serde_json::from_value(event).unwrap())
        .is_err());
    assert_eq!(store.torrents, before);
    let begin = json!({"ev":"snapshot.begin","sub":4,"topic":"torrents","rev":1,"count":1,"reason":"subscribe"});
    store
        .event(&serde_json::from_value(begin).unwrap())
        .unwrap();
    let end = json!({"ev":"snapshot.end","sub":4,"topic":"torrents","rev":2});
    assert!(store.event(&serde_json::from_value(end).unwrap()).is_err());
    assert_eq!(store.torrents, before);
}

#[test]
fn staging_byte_budget_and_subscription_count_are_bounded() {
    let (mut store, _) = replay(&fixture("subscribe-gap.ndjson")).unwrap();
    let row = store.torrents.values().next().unwrap().clone();
    let mut snapshot = Snapshot::new(1);
    assert!(snapshot
        .append(&api::Topic::Torrents, &[api::Item::Torrent(row)], 1)
        .is_err());
    assert_eq!(
        snapshot.len(),
        0,
        "an over-budget chunk must not partially enter staging"
    );
    store.subscriptions.clear();
    for sub in 1..=api::MAX_SUBSCRIPTIONS as u64 {
        let event = json!({"ev":"snapshot.begin","sub":sub,"topic":"torrents","rev":1,"count":0,"reason":"subscribe"});
        store
            .event(&serde_json::from_value(event).unwrap())
            .unwrap();
    }
    let event = json!({"ev":"snapshot.begin","sub":17,"topic":"torrents","rev":1,"count":0,"reason":"subscribe"});
    assert!(store
        .event(&serde_json::from_value(event).unwrap())
        .is_err());
    assert_eq!(store.subscriptions.len(), api::MAX_SUBSCRIPTIONS);
}

#[test]
fn reconnecting_to_the_same_instance_resets_connection_scoped_revisions() {
    let (mut store, _) = replay(&fixture("subscribe-gap.ndjson")).unwrap();
    let before = store.torrents.clone();
    store.hello(store.hello.clone().unwrap());
    assert!(store.subscriptions.is_empty());
    assert!(store.stale);
    assert_eq!(store.torrents, before);
}
