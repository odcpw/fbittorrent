//! Literal field obligations from the bead, independent of serde/TABLE discovery.
use super::*;

type Objects = BTreeMap<&'static str, Vec<Value>>;
fn collect(value: &Value, parent: &str, out: &mut Objects) {
    if let Some(o) = value.as_object() {
        let mut kinds = Vec::new();
        if o.contains_key("hash") && o.contains_key("intent") {
            kinds.push("Torrent");
            if o.contains_key("private") {
                kinds.push("TorrentDetail");
            }
        } else if o.contains_key("hash")
            && o.contains_key("state")
            && o.contains_key("name")
            && !o.contains_key("ev")
        {
            kinds.push("TopTorrent");
        }
        if parent == "playable" {
            kinds.push("Playable");
        }
        if o.contains_key("index") && o.contains_key("sort_key") {
            kinds.push("File");
        }
        if o.contains_key("counts") && o.contains_key("disk") && o.contains_key("daemon") {
            kinds.push("Summary");
        }
        if o.contains_key("busy_reasons") {
            kinds.push("SummaryDaemon");
        }
        if o.contains_key("message") && o.contains_key("ready") {
            kinds.push("NetworkState");
        }
        if o.get("state")
            .and_then(Value::as_str)
            .is_some_and(|v| matches!(v, "ready" | "waiting" | "off"))
            && o.contains_key("message")
        {
            kinds.push("SummaryNetwork");
        }
        if parent == "limits" && !o.contains_key("line_in") {
            kinds.push("Limits");
        }
        if o.contains_key("max_active_downloads") && o.contains_key("peer_encryption") {
            kinds.push("Settings");
        }
        if o.contains_key("level") && o.contains_key("source") && o.contains_key("seq") {
            kinds.push("LogEntry");
        }
        for kind in kinds {
            out.entry(kind).or_default().push(value.clone());
        }
        for (key, child) in o {
            collect(child, key, out);
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            collect(child, parent, out);
        }
    }
}
fn catalog() -> Objects {
    let mut out = Objects::new();
    for (_, text) in GROUPS {
        for line in text.lines() {
            collect(&serde_json::from_str::<Value>(line).unwrap(), "", &mut out);
        }
    }
    out
}
fn both(objects: &Objects, kind: &str, fields: &[&str]) {
    let values = objects
        .get(kind)
        .unwrap_or_else(|| panic!("no {kind} fixture"));
    for field in fields {
        assert!(
            values
                .iter()
                .any(|v| v.get(*field).is_some_and(|v| !v.is_null())),
            "missing {kind}.{field} present fixture"
        );
        assert!(
            values.iter().any(|v| v.get(*field).is_none()),
            "missing {kind}.{field} absent fixture"
        );
    }
}
const TORRENT_OPTIONAL: &[&str] = &[
    "reason",
    "error",
    "progress",
    "size",
    "completed_bytes",
    "uploaded_bytes",
    "dl_bps",
    "ul_bps",
    "eta_s",
    "peers",
    "seeds",
    "upload_limit_bps",
    "ratio",
    "content_path",
    "completed_at",
    "origin",
    "imdb_id",
    "queue_pos",
    "playable",
];
#[test]
fn every_optional_field_has_present_and_absent_examples() {
    let objects = catalog();
    both(&objects, "Torrent", TORRENT_OPTIONAL);
    both(&objects, "TorrentDetail", TORRENT_OPTIONAL);
    both(
        &objects,
        "TorrentDetail",
        &[
            "file_count",
            "piece_count",
            "piece_size",
            "comment",
            "created_by",
            "tracker_count",
            "undo_until",
        ],
    );
    both(
        &objects,
        "Playable",
        &["stall_free", "buffered_s", "finishes_in_s"],
    );
    both(&objects, "File", &["ready"]);
    both(&objects, "Summary", &["dl_bps", "ul_bps", "history"]);
    both(&objects, "SummaryDaemon", &["idle_exit_in_s"]);
    both(&objects, "SummaryNetwork", &["interface"]);
    both(
        &objects,
        "TopTorrent",
        &["progress", "dl_bps", "eta_s", "playable"],
    );
    both(
        &objects,
        "Limits",
        &[
            "download_bps",
            "upload_bps",
            "alt_download_bps",
            "alt_upload_bps",
        ],
    );
    both(&objects, "NetworkState", &["selection", "interface"]);
    both(
        &objects,
        "Settings",
        &[
            "network_interface",
            "download_limit_bps",
            "upload_limit_bps",
            "alt_download_limit_bps",
            "alt_upload_limit_bps",
        ],
    );
    both(&objects, "LogEntry", &["hash"]);
    let playable = &objects["Playable"];
    for field in [
        "file",
        "ready",
        "prefix_bytes",
        "acked",
        "stall_free",
        "buffered_s",
        "finishes_in_s",
    ] {
        assert!(
            playable.iter().any(|v| v.get(field).is_some()),
            "missing Playable.{field}"
        );
    }
    // These typed objects occur in generic diagnostic details, without adding
    // the P1-14 operation or topic. Still enforce typed-loss-free fixture data.
    for entry in &objects["LogEntry"] {
        let typed: LogEntry = serde_json::from_value(entry.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *entry);
    }
}
#[test]
fn all_states_and_state_conditioned_absence() {
    let objects = catalog();
    let values = &objects["Torrent"];
    let present: BTreeSet<_> = values
        .iter()
        .map(|v| v["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        present,
        [
            "metadata",
            "checking",
            "downloading",
            "stalled",
            "queued",
            "paused",
            "done",
            "seeding",
            "waiting_vpn",
            "error"
        ]
        .into_iter()
        .collect()
    );
    for state in ["queued", "paused", "done", "waiting_vpn", "error"] {
        assert!(
            values
                .iter()
                .any(|v| v["state"] == state && v["reason"].is_string()),
            "{state} needs an explicit reason fixture"
        );
    }
    assert!(
        values
            .iter()
            .any(|v| v["completed_bytes"] == 0 && v.get("ratio").is_none()),
        "known zero downloaded bytes must omit ratio"
    );
    for value in values {
        let state = value["state"].as_str().unwrap();
        if matches!(state, "metadata" | "waiting_vpn") {
            assert!(
                value.get("progress").is_none(),
                "{state}: unknown progress must be absent"
            );
        }
        if state == "metadata" {
            assert!(
                value.get("size").is_none(),
                "before metadata size must be absent"
            );
        }
        if !matches!(state, "downloading" | "stalled" | "seeding") {
            for field in ["peers", "dl_bps", "ul_bps"] {
                assert!(
                    value.get(field).is_none(),
                    "{state}: {field} without a live engine"
                );
            }
        }
        if value
            .get("completed_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
        {
            assert!(value.get("ratio").is_none());
        }
    }
}
#[test]
fn addition_removal_and_app_info_shape_are_exercised() {
    let mut added = false;
    let mut removed = false;
    let mut infos = 0;
    for (_, text) in GROUPS {
        let mut calls = BTreeMap::new();
        for line in text.lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            let f = &row["frame"];
            if row["dir"] == "c2s" {
                calls.insert(
                    f["id"].as_u64().unwrap(),
                    f["op"].as_str().unwrap().to_owned(),
                );
            }
            if f["ev"] == "delta" {
                added |= f["added"].as_array().is_some_and(|v| !v.is_empty());
                removed |= f["removed"].as_array().is_some_and(|v| !v.is_empty());
            }
            if f["ok"] == true
                && calls
                    .get(&f["id"].as_u64().unwrap())
                    .is_some_and(|op| op == "app.info")
            {
                for field in [
                    "proto",
                    "server",
                    "engine",
                    "instance",
                    "engine_epoch",
                    "state",
                    "caps",
                    "limits",
                    "policy",
                    "uptime_s",
                    "upgrade_pending",
                    "clients",
                    "subscriptions",
                ] {
                    assert!(f["result"].get(field).is_some(), "missing app.info.{field}");
                }
                infos += 1;
            }
        }
    }
    assert!(
        added && removed,
        "both non-empty delta added and removed fixtures are required"
    );
    assert!(infos > 0);
}
