//! Shared fixture format and contract checks. The daemon and UI replay runners
//! consume these same files; this module makes no claim of live server proof.
use super::validation::{self, NegotiatedCaps, Revisions, TorrentFileSizes, Validate};
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const GROUPS: &[(&str, &str)] = &[
    ("trackers",include_str!("../../contracts/ipc/v1/trackers.ndjson")),
    ("origin",include_str!("../../contracts/ipc/v1/origin.ndjson")),
    ("queue",include_str!("../../contracts/ipc/v1/queue.ndjson")),
    ("search", include_str!("../../contracts/ipc/v1/search.ndjson")),
    ("recheck", include_str!("../../contracts/ipc/v1/recheck.ndjson")),
    ("limits", include_str!("../../contracts/ipc/v1/limits.ndjson")),
    ("log", include_str!("../../contracts/ipc/v1/log.ndjson")),
    ("free-space", include_str!("../../contracts/ipc/v1/free-space.ndjson")),
    ("settings", include_str!("../../contracts/ipc/v1/settings.ndjson")),
    ("imdb", include_str!("../../contracts/ipc/v1/imdb.ndjson")),
    ("hello", include_str!("../../contracts/ipc/v1/hello.ndjson")),
    (
        "errors",
        include_str!("../../contracts/ipc/v1/errors.ndjson"),
    ),
    (
        "torrents",
        include_str!("../../contracts/ipc/v1/torrents.ndjson"),
    ),
    (
        "subscribe",
        include_str!("../../contracts/ipc/v1/subscribe.ndjson"),
    ),
    (
        "subscribe-gap",
        include_str!("../../contracts/ipc/v1/subscribe-gap.ndjson"),
    ),
    (
        "mutations",
        include_str!("../../contracts/ipc/v1/mutations.ndjson"),
    ),
    (
        "settings-motion",
        include_str!("../../contracts/ipc/v1/settings-motion.ndjson"),
    ),
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "dir", rename_all = "snake_case", deny_unknown_fields)]
enum Line {
    C2s { frame: Request },
    S2c { frame: ServerFrame },
    Expect { model: Model },
    Volatile { paths: Vec<String> },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ServerFrame {
    Response(Response<Value>),
    Event(Event),
}

/// Common expected store fields. A revision-gap marker means the suspect delta
/// was not applied; the following c2s frames specify unsubscribe/re-subscribe.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    #[serde(skip_serializing_if = "Option::is_none")]
    proto: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compatible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    torrents: Option<BTreeMap<Hash, Torrent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<NetworkState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<Settings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscriptions: Option<BTreeMap<String, ExpectedSubscription>>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedSubscription {
    topic: Topic,
    rev: u64,
    resync: bool,
}

fn parse(group: &str, n: usize, text: &str) -> Result<Line, String> {
    serde_json::from_str(text).map_err(|e| format!("contracts/ipc/v1/{group}.ndjson:{n}: {e}"))
}

fn canonical(mut value: Value) -> String {
    value.sort_all_objects();
    serde_json::to_string(&value).unwrap()
}

/// The explicit gap fixture is a client recovery scenario. Every other fixture
/// is checked as a contiguous server transcript, using the production tracker.
pub(super) fn check_group(group: &str, text: &str) -> Result<(), String> {
    let mut calls = BTreeMap::new();
    let mut caps = NegotiatedCaps::default();
    let mut file_sizes = TorrentFileSizes::default();
    let mut revisions = Revisions::default();
    let mut unsubscribes = BTreeMap::new();
    let mut expected_gaps = 0;
    for (i, text) in text.lines().enumerate() {
        let location = format!("contracts/ipc/v1/{group}.ndjson:{}", i + 1);
        let line = parse(group, i + 1, text)?;
        let check = (|| -> Result<(), String> {
            match &line {
                Line::Volatile { .. } if i != 0 => return Err("volatile must be first".into()),
                Line::C2s { frame } => {
                    frame.validate().map_err(|e| e.to_string())?;
                    validation::frame(frame, LINE_IN).map_err(|e| e.to_string())?;
                    calls.insert(frame.id.get(), frame.call.clone());
                    unsubscribes.remove(&frame.id.get());
                    if let Call::Unsubscribe { args } = &frame.call {
                        unsubscribes.insert(frame.id.get(), args.sub);
                    }
                }
                Line::S2c { frame } => {
                    validation::frame(frame, LINE_OUT).map_err(|e| e.to_string())?;
                    match frame {
                        ServerFrame::Response(Response::Success(frame)) => {
                            let call = calls
                                .get(&frame.id.get())
                                .ok_or("response has no request")?;
                            let op = call.op();
                            let reply = Reply::decode(op, frame.result.clone())
                                .map_err(|e| format!("{op} result: {e}"))?;
                            if let Reply::Hello(hello) = &reply {
                                caps = NegotiatedCaps::from_hello(hello);
                            }
                            caps.reply(&reply).map_err(|e| e.to_string())?;
                            file_sizes.reply(call, &reply).map_err(|e| e.to_string())?;
                            if op == "unsubscribe" {
                                if let Some(sub) = unsubscribes.remove(&frame.id.get()) {
                                    revisions.unsubscribe(sub);
                                }
                            }
                            let typed = Success {
                                id: frame.id,
                                ok: Bool,
                                result: reply,
                            };
                            if canonical(serde_json::to_value(typed).unwrap())
                                != canonical(serde_json::to_value(frame).unwrap())
                            {
                                return Err("typed success envelope differs".into());
                            }
                        }
                        ServerFrame::Response(Response::Failure(frame)) => {
                            frame.error.validate().map_err(|e| e.to_string())?
                        }
                        ServerFrame::Event(event) => {
                            event.validate().map_err(|e| e.to_string())?;
                            caps.event(event).map_err(|e| e.to_string())?;
                            if let Event::MutationResult {
                                op,
                                result: Some(result),
                                ..
                            } = event
                            {
                                if ops::TABLE.iter().any(|r| r.name == op) {
                                    let raw = Value::Object(result.clone());
                                    let typed = Reply::decode(op, raw.clone())
                                        .map_err(|e| e.to_string())?;
                                    if canonical(typed.to_value().unwrap()) != canonical(raw) {
                                        return Err("typed mutation result differs".into());
                                    }
                                }
                            }
                            if let Some((sub, rev)) = event.subscription_revision() {
                                if let Err(error) = revisions.observe(sub, rev) {
                                    if group == "subscribe-gap"
                                        && expected_gaps == 0
                                        && revisions.last(sub) == 3
                                        && matches!(
                                            event,
                                            Event::Delta {
                                                sub: 1,
                                                rev: 5,
                                                topic: Topic::Torrents,
                                                ..
                                            }
                                        )
                                    {
                                        expected_gaps += 1;
                                    } else {
                                        return Err(error.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                Line::Expect { model } => {
                    if let Some(torrents) = &model.torrents {
                        for t in torrents.values() {
                            t.validate().map_err(|e| e.to_string())?;
                            caps.torrent(t).map_err(|e| e.to_string())?;
                        }
                    }
                    if let Some(summary) = &model.summary {
                        summary.validate().map_err(|e| e.to_string())?;
                        caps.summary(summary).map_err(|e| e.to_string())?;
                    }
                }
                _ => {}
            }
            let original: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
            if canonical(serde_json::to_value(line).unwrap()) != canonical(original) {
                return Err("canonical round-trip differs".into());
            }
            Ok(())
        })();
        check.map_err(|e| format!("{location}: {e}"))?;
    }
    if group == "subscribe-gap" && expected_gaps != 1 {
        return Err("subscribe-gap: missing the one deliberate revision gap".into());
    }
    Ok(())
}

#[test]
fn roundtrip() {
    for (group, text) in GROUPS {
        check_group(group, text).unwrap_or_else(|e| panic!("{e}"));
    }
}

/// Report JSON pointers, including escaped object keys, to the exact bad value.
fn sentinel(value: &Value, path: &str) -> Result<(), String> {
    if value
        .as_f64()
        .is_some_and(|n| n == -1.0 || n == 8_640_000.0)
    {
        return Err(format!("{path}: sentinel {value}"));
    }
    if value.is_null() {
        return Err(format!("{path}: omit unknown full-object fields"));
    }
    if let Some(map) = value.as_object() {
        for (key, child) in map {
            let pointer = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
            // Do not mistake valid indices/counts/bytes at the beginning of a
            // known transfer for unknown values. Rates may be genuinely zero.
            if child.as_f64() == Some(0.0) {
                let invalid = match key.as_str() {
                    "eta_s" | "rating" | "uploaded_at" => true,
                    // P6-11 reports an observed zero in its typed destination result.
                    "free_bytes" => !map.get("writable").is_some_and(Value::is_boolean),
                    "size" => !map.contains_key("index"),
                    "progress" => {
                        !map.contains_key("index")
                            && !map
                                .get("size")
                                .is_some_and(|s| s.as_u64().is_some_and(|n| n > 0))
                    }
                    "ratio" => !map
                        .get("completed_bytes")
                        .is_some_and(|n| n.as_u64().is_some_and(|n| n > 0)),
                    "peers" | "seeds" => !map
                        .get("state")
                        .and_then(Value::as_str)
                        .is_some_and(|s| matches!(s, "downloading" | "stalled" | "seeding")),
                    _ => false,
                };
                if invalid {
                    return Err(format!("{pointer}: zero substitutes for an unknown value"));
                }
            }
            // Null is meaningful only in a deletion patch, an explicit input
            // upload-limit reset, or the required invalid-ID response envelope.
            if child.is_null() {
                if path.contains("/changed/")
                    || key == "id" && map.get("ok") == Some(&Value::Bool(false))
                    || matches!(key.as_str(), "bps" | "download_bps" | "upload_bps" | "alt_download_bps" | "alt_upload_bps") && path.ends_with("/args")
                {
                    continue;
                }
                return Err(format!("{pointer}: omit unknown full-object fields"));
            }
            sentinel(child, &pointer)?;
        }
    } else if let Some(items) = value.as_array() {
        for (i, child) in items.iter().enumerate() {
            sentinel(child, &format!("{path}/{i}"))?;
        }
    }
    Ok(())
}

#[test]
fn no_sentinels() {
    for (group, text) in GROUPS {
        for (i, text) in text.lines().enumerate() {
            let value: Value = serde_json::from_str(text).unwrap();
            sentinel(&value, "")
                .unwrap_or_else(|e| panic!("contracts/ipc/v1/{group}.ndjson:{}: {e}", i + 1));
        }
    }
}

#[test]
fn sentinel_negative_controls() {
    for (value, pointer) in [
        (json!({"result":{"eta_s":8640000}}), "/result/eta_s"),
        (json!({"result":{"progress":-1}}), "/result/progress"),
        (json!({"result":{"progress":-1.0}}), "/result/progress"),
        (json!({"result":{"history":[8640000]}}), "/result/history/0"),
        (json!({"result":{"size":0}}), "/result/size"),
        (json!({"result":{"progress":0}}), "/result/progress"),
        (json!({"result":{"ratio":0}}), "/result/ratio"),
        (json!({"result":{"peers":0}}), "/result/peers"),
        (json!({"result":{"seeds":0}}), "/result/seeds"),
        (json!({"result":{"eta_s":0}}), "/result/eta_s"),
        (json!({"result":{"free_bytes":0}}), "/result/free_bytes"),
        (json!({"result":{"size":null}}), "/result/size"),
        (json!({"result":{"files":[null]}}), "/result/files/0"),
    ] {
        assert!(sentinel(&value, "").unwrap_err().starts_with(pointer));
    }
    // Known, observed zeros survive; unknown is represented by omission.
    sentinel(&json!({"free_bytes":0,"writable":false}), "").unwrap();
    sentinel(&json!({"size":4096,"progress":0,"completed_bytes":0,"dl_bps":0,"ul_bps":0,"index":0,"total":0}), "").unwrap();
    sentinel(
        &json!({"state":"stalled","peers":0,"seeds":0,"completed_bytes":1024,"ratio":0}),
        "",
    )
    .unwrap();
    sentinel(&json!({"changed":{"a/b":{"eta_s":null}}}), "").unwrap();
}

#[test]
fn strict_input_and_tolerant_output() {
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let good = json!({"id":1,"op":"torrents.remove","args":{"hashes":[hash],"delete_files":false}});
    let request: Request = serde_json::from_value(good.clone()).unwrap();
    assert_eq!(request.call.op(), "torrents.remove");
    for bad in [
        json!({"id":0,"op":"app.info"}),
        json!({"id":-1,"op":"app.info"}),
        json!({"id":1.5,"op":"app.info"}),
        json!({"id":1,"op":"typo"}),
        json!({"id":1,"op":"app.info","args":{"typo":true}}),
        json!({"id":1,"op":"torrents.remove","args":{"hashes":[hash]}}),
        json!({"id":1,"op":"torrents.remove","args":{"hashes":[hash],"delete_files":"false"}}),
        json!({"id":1,"op":"torrents.stop","args":{"hashes":[]}}),
        json!({"id":1,"op":"torrents.stop","args":{"hashes":[hash.to_uppercase()]}}),
        json!({"id":1,"op":"torrents.stop","args":{"hashes":(vec![hash;257])}}),
        json!({"id":1,"op":"torrents.add","args":{"source":{"path":"a","url":"b"}}}),
        json!({"id":1,"op":"app.client","args":{"interactiv":true}}),
        json!({"id":1,"op":"app.info","instance":"not-an-instance"}),
        json!({"id":1,"op":"app.info","ref":"x".repeat(65)}),
        json!({"id":1,"op":"app.info","ref":"café"}),
    ] {
        assert!(
            serde_json::from_value::<Request>(bad.clone()).is_err(),
            "accepted invalid request: {bad}"
        );
    }
    let mut output = json!({"ready":false,"message":"waiting","future_field":123});
    let _: NetworkState = serde_json::from_value(output.take()).unwrap();
    assert!(serde_json::from_value::<Response<Value>>(
        json!({"id":1,"ok":true,"error":{"code":"internal","message":"x","retryable":false}})
    )
    .is_err());
    assert!(serde_json::from_value::<ErrorCode>(json!("duplicate")).is_err());
    // Open enum fields retain future values; the presentation layer renders other.
    let log: LogEntry = serde_json::from_value(
        json!({"seq":1,"ts":1,"level":"info","source":"future-source","message":"x"}),
    )
    .unwrap();
    assert_eq!(log.source, "future-source");
    let no_args: Request =
        serde_json::from_value(json!({"id":1,"op":"app.info","args":{}})).unwrap();
    assert_eq!(
        serde_json::to_value(no_args).unwrap(),
        json!({"id":1,"op":"app.info","args":{}})
    );
}

#[test]
fn nullable_limit_is_not_omission() {
    let base = json!({"hashes":["0123456789abcdef0123456789abcdef01234567"]});
    for (value, expected) in [
        (base.clone(), Update::Unset),
        (json!({"hashes":base["hashes"],"bps":null}), Update::Clear),
        (
            json!({"hashes":base["hashes"],"bps":32768}),
            Update::Set(32768),
        ),
    ] {
        let args: UploadLimitArgs = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(args.bps, expected);
        assert_eq!(serde_json::to_value(args).unwrap(), value);
    }
}

#[test]
fn malformed_lines_report_file_and_line() {
    for text in [
        "{}",
        "{",
        "[]",
        r#"{"dir":"typo","frame":{}}"#,
        r#"{"dir":"c2s","frame":{"id":0,"op":"app.info"}}"#,
    ] {
        assert!(parse("negative", 7, text)
            .unwrap_err()
            .starts_with("contracts/ipc/v1/negative.ndjson:7:"));
    }
}

#[test]
fn operation_event_error_and_document_coverage() {
    let mut operations = BTreeSet::new();
    let mut events = BTreeSet::new();
    let mut codes = BTreeSet::new();
    for (group, text) in GROUPS {
        for (i, text) in text.lines().enumerate() {
            match parse(group, i + 1, text).unwrap() {
                Line::C2s { frame } => {
                    operations.insert(frame.call.op());
                }
                Line::S2c {
                    frame: ServerFrame::Event(event),
                } => {
                    events.insert(event.name());
                }
                Line::S2c {
                    frame: ServerFrame::Response(Response::Failure(f)),
                } => {
                    codes.insert(f.error.code);
                }
                _ => {}
            }
        }
    }
    let ipc = include_str!("../../docs/IPC.md");
    // Additions to this literal require their owning bead (imdb.lookup: P3-04).
    // TABLE and fixture changes alone cannot authorize an operation.
    let required_operations = [
        "torrents.trackers",
        "torrents.reannounce",
        "torrents.force_start",
        "torrents.queue_move",
        "search.start", "search.stop", "search.providers", "search.set_provider",
        "search.install", "search.browse_sources", "search.browse", "search.grab",
        "torrents.recheck",
        "app.log",
        "app.stats",
        "app.free_space", // P6-11: planned additive operation.
        "app.settings.get",
        "app.settings.set",
        "imdb.lookup",
        "hello",
        "app.info",
        "app.client",
        "app.shutdown",
        "app.restart",
        "app.network",
        "transfer.info",
        "transfer.set_limits",
        "transfer.set_alt",
        "torrents.list",
        "torrents.get",
        "torrents.add",
        "torrents.start",
        "torrents.stop",
        "torrents.remove",
        "torrents.files",
        "torrents.set_files",
        "torrents.set_seed",
        "torrents.set_upload_limit",
        "torrents.open_target",
        "subscribe",
        "unsubscribe",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(
        operations, required_operations,
        "fixture operations must match the bead"
    );
    assert_eq!(
        ops::TABLE.iter().map(|r| r.name).collect::<BTreeSet<_>>(),
        required_operations,
        "TABLE must match the bead, independently of fixture coverage"
    );
    let idempotent = ipc
        .split("- Idempotent operations")
        .nth(1)
        .unwrap()
        .split(". Every other")
        .next()
        .unwrap();
    for row in ops::TABLE {
        assert_eq!(
            row.idempotent,
            row.mutating && idempotent.contains(&format!("`{}`", row.name)),
            "resend metadata drifted for {}",
            row.name
        );
        assert!(
            ipc.contains(&format!("| `{}` |", row.name)),
            "missing documented op {}",
            row.name
        );
    }
    for name in [
        "snapshot.begin",
        "snapshot.chunk",
        "snapshot.end",
        "delta",
        "state",
        "engine.changed",
        "torrent.added",
        "torrent.completed",
        "torrent.playable",
        "torrent.error",
        "mutation.result",
        "search.done",
        "shows.checked",
        "clipboard.magnet",
        "heartbeat",
        "bye",
        "error",
    ] {
        assert!(events.contains(name), "missing event fixture: {name}");
        assert!(
            ipc.contains(&format!("| `{name}` |")),
            "missing documented event: {name}"
        );
    }
    assert_eq!(
        events.len(),
        17,
        "event names must match the bead's literal list"
    );
    assert_eq!(
        codes.len(),
        16,
        "every ErrorCode needs a fixture; duplicate is not an error"
    );
    let exits: BTreeSet<_> = codes.into_iter().map(ErrorCode::exit_code).collect();
    assert_eq!(exits, (1..=8).collect());
}

#[test]
fn documentation_examples_are_fixture_lines() {
    let docs = include_str!("../../docs/IPC.md");
    let mut in_example = false;
    let mut examples = 0;
    for line in docs.lines() {
        if line == "```ndjson" {
            in_example = true;
            continue;
        }
        if line == "```" {
            in_example = false;
        }
        if in_example {
            let _: Line = serde_json::from_str(line).expect("invalid documented fixture line");
            let value: Value = serde_json::from_str(line).unwrap();
            assert!(
                GROUPS.iter().any(
                    |(_, text)| text
                        .lines()
                        .any(|l| serde_json::from_str::<Value>(l).unwrap() == value)
                ),
                "documented example is absent from fixtures: {line}"
            );
            examples += 1;
        }
    }
    assert!(
        examples >= 3,
        "keep examples tested against the shared fixtures"
    );
}

#[test]
fn all_argument_objects_reject_unknown_keys() {
    for (group, text) in GROUPS {
        for (i, text) in text.lines().enumerate() {
            let mut line: Value = serde_json::from_str(text).unwrap();
            if line["dir"] != "c2s" {
                continue;
            }
            let frame = line["frame"].as_object_mut().unwrap();
            let args = frame.entry("args").or_insert_with(|| json!({}));
            args.as_object_mut()
                .unwrap()
                .insert("misspelled_argument".into(), json!(true));
            assert!(
                serde_json::from_value::<Request>(Value::Object(frame.clone())).is_err(),
                "{group}:{} accepted an unknown argument",
                i + 1
            );
        }
    }
}

#[test]
fn topic_payloads_and_chunk_bounds() {
    let hash = Hash::new("0123456789abcdef0123456789abcdef01234567").unwrap();
    let file = File {
        index: 0,
        path: "e2.mkv".into(),
        size: 1,
        progress: 0.0,
        included: true,
        sort_key: "e0000000002.mkv".into(),
        ready: None,
    };
    let chunk = |topic, items| Event::SnapshotChunk {
        sub: 1,
        topic,
        rev: 1,
        items,
    };
    chunk(
        Topic::Files(hash.clone()),
        vec![Item::File(file.clone()); MAX_CHUNK_ITEMS],
    )
    .validate()
    .unwrap();
    assert!(chunk(
        Topic::Files(hash.clone()),
        vec![Item::File(file.clone()); MAX_CHUNK_ITEMS + 1]
    )
    .validate()
    .is_err());
    assert!(chunk(Topic::Torrents, vec![Item::File(file)])
        .validate()
        .is_err());
    assert!(Event::State {
        sub: 1,
        topic: Topic::Files(hash),
        rev: 1,
        data: StateData::Network(NetworkState {
            ready: false,
            message: "waiting".into(),
            selection: None,
            interface: None
        })
    }
    .validate()
    .is_err());
}

#[test]
fn gap_fixture_requires_recovery_and_preserves_last_revision() {
    let (_, text) = GROUPS
        .iter()
        .find(|(name, _)| *name == "subscribe-gap")
        .unwrap();
    let lines: Vec<_> = text
        .lines()
        .enumerate()
        .map(|(i, text)| parse("subscribe-gap", i + 1, text).unwrap())
        .collect();
    let mut last = BTreeMap::new();
    let mut gap = None;
    let mut recovery = Vec::new();
    let mut consistent = false;
    for (i, line) in lines.iter().enumerate() {
        match line {
            Line::S2c {
                frame: ServerFrame::Event(event),
            } => {
                let frame = serde_json::to_value(event).unwrap();
                if let (Some(sub), Some(rev)) = (frame["sub"].as_u64(), frame["rev"].as_u64()) {
                    let expected = last.get(&sub).copied().unwrap_or(0) + 1;
                    if rev != expected {
                        assert!(gap.is_none(), "fixture must inject exactly one gap");
                        gap = Some((i, sub, expected - 1));
                    } else {
                        last.insert(sub, rev);
                    }
                }
            }
            Line::C2s { frame } if gap.is_some() => recovery.push(frame.call.op()),
            Line::Expect { model } if gap.is_some() => {
                if let Some(subscriptions) = &model.subscriptions {
                    for (sub, subscription) in subscriptions {
                        if subscription.resync {
                            let (_, original, previous) = gap.unwrap();
                            assert_eq!(sub, &original.to_string());
                            assert_eq!(
                                subscription.rev, previous,
                                "a suspect delta must not advance the store revision"
                            );
                        } else {
                            consistent = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assert!(gap.is_some(), "missing skipped revision");
    assert_eq!(recovery, ["unsubscribe", "subscribe"]);
    assert!(consistent, "missing rebuilt consistent model");
}

#[path = "fixtures/coverage.rs"]
mod coverage;
#[path = "fixtures/negative.rs"]
mod negative;
