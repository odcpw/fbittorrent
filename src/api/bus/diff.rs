use super::*;
pub(super) fn state_event(sub: u64, topic: &Topic, rev: u64, value: Value) -> ApiResult<Event> {
    let data = serde_json::from_value(value)
        .map_err(|_| error(ErrorCode::Internal, "invalid state sample", None))?;
    Ok(Event::State {
        sub,
        topic: topic.clone(),
        rev,
        data,
    })
}
fn key_field(topic:&Topic)->&'static str {
    match topic {
        Topic::RssFeeds | Topic::RssItems(_)=>"id",
        Topic::Shows=>"imdb",Topic::ShowReleases(_)=>"key",
        Topic::Torrents => "hash",
        Topic::Files(_) => "index",
        Topic::Peers(_) => "addr",
        Topic::Events | Topic::Log => "seq",
        _ => unreachable!(),
    }
}
fn key(topic: &Topic, value: &Value) -> String {
    if matches!(topic,Topic::ShowReleases(id) if id=="inbox") {return format!("{}/{}",value["show_imdb"].as_str().unwrap(),value["key"].as_str().unwrap());}
    let v = &value[key_field(topic)];
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
type Changed = BTreeMap<String, BTreeMap<String, Value>>;
pub(super) fn diff(topic: &Topic, old: &Value, new: &Value) -> (Changed, Vec<Value>, Vec<Value>) {
    let before: BTreeMap<_, _> = old
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (key(topic, v), v))
        .collect();
    let after: BTreeMap<_, _> = new
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (key(topic, v), v))
        .collect();
    let mut changed = BTreeMap::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for value in new.as_array().unwrap() {
        let k = key(topic, value);
        if let Some(old) = before.get(&k) {
            let mut fields = BTreeMap::new();
            for field in old
                .as_object()
                .unwrap()
                .keys()
                .chain(value.as_object().unwrap().keys())
                .collect::<BTreeSet<_>>()
            {
                if old.get(field) != value.get(field) {
                    fields.insert(
                        field.clone(),
                        value.get(field).cloned().unwrap_or(Value::Null),
                    );
                }
            }
            if !fields.is_empty() {
                changed.insert(k, fields);
            }
        } else {
            added.push(value.clone());
        }
    }
    // Events are append-only. Ring eviction is retention, never a deletion in
    // the client's event history.
    if !matches!(topic, Topic::Events | Topic::Log) {
        for (k, v) in before {
            if !after.contains_key(&k) {
                removed.push(
                    if matches!(topic,Topic::ShowReleases(id) if id=="inbox"){json!(k)}else{v[key_field(topic)].clone()},
                );
            }
        }
    }
    (changed, added, removed)
}
const COALESCED: &[&str] = &[
    "progress",
    "completed_bytes",
    "downloaded",
    "uploaded",
    "uploaded_bytes",
    "dl_bps",
    "ul_bps",
    "eta_s",
    "peers",
    "seeds",
    "ratio",
];
pub(super) fn structural_projection(topic: &Topic, old: &Value, new: &Value) -> Value {
    if matches!(topic, Topic::Summary) {
        let mut v = new.clone();
        for k in ["counts", "dl_bps", "ul_bps"] {
            if let Some(old) = old.get(k) {
                v[k] = old.clone();
            } else {
                v.as_object_mut().unwrap().remove(k);
            }
        }
        if let Some(top) = v.get_mut("top").and_then(Value::as_array_mut) {
            for row in top {
                if let Some(before) = old["top"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|before| before["hash"] == row["hash"]))
                {
                    preserve_counters(before, row);
                }
            }
        }
        return v;
    }
    if !matches!(topic, Topic::Torrents | Topic::Peers(_) | Topic::Files(_)) {
        return new.clone();
    }
    let before: BTreeMap<_, _> = old
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (key(topic, v), v))
        .collect();
    json!(new
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let mut result = v.clone();
            if let Some(old) = before.get(&key(topic, v)) {
                for k in COALESCED {
                    if let Some(old) = old.get(*k) {
                        result[*k] = old.clone();
                    } else {
                        result.as_object_mut().unwrap().remove(*k);
                    }
                }
                if let Some(play) = result.get_mut("playable").and_then(Value::as_object_mut) {
                    for k in ["buffered_s", "finishes_in_s"] {
                        if let Some(old) = old.get("playable").and_then(|p| p.get(k)) {
                            play.insert(k.into(), old.clone());
                        } else {
                            play.remove(k);
                        }
                    }
                }
            }
            result
        })
        .collect::<Vec<_>>())
}

fn preserve_counters(before: &Value, after: &mut Value) {
    for key in COALESCED {
        if let Some(value) = before.get(*key) {
            after[*key] = value.clone();
        } else {
            after.as_object_mut().unwrap().remove(*key);
        }
    }
    if let Some(play) = after.get_mut("playable").and_then(Value::as_object_mut) {
        for key in ["buffered_s", "finishes_in_s"] {
            if let Some(value) = before.get("playable").and_then(|v| v.get(key)) {
                play.insert(key.into(), value.clone());
            } else {
                play.remove(key);
            }
        }
    }
}
