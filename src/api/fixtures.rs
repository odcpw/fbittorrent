//! Shared fixture format and contract checks. The daemon and UI replay runners
//! consume these same files; this module makes no claim of live server proof.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const GROUPS: &[(&str, &str)] = &[
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

#[test]
fn roundtrip() {
    for (group, text) in GROUPS {
        let mut calls = BTreeMap::new();
        for (i, text) in text.lines().enumerate() {
            let location = format!("contracts/ipc/v1/{group}.ndjson:{}", i + 1);
            let line = parse(group, i + 1, text).unwrap_or_else(|e| panic!("{e}"));
            if matches!(line, Line::Volatile { .. }) {
                assert_eq!(i, 0, "{location}: volatile must be first");
            }
            match &line {
                Line::C2s { frame } => {
                    calls.insert(frame.id.get(), frame.call.op());
                }
                Line::S2c {
                    frame: ServerFrame::Response(Response::Success(frame)),
                } => {
                    let op = calls
                        .get(&frame.id.get())
                        .unwrap_or_else(|| panic!("{location}: response has no request"));
                    let result = Reply::decode(op, frame.result.clone())
                        .unwrap_or_else(|e| panic!("{location}: {op} result: {e}"));
                    assert_eq!(
                        result.to_value().unwrap(),
                        frame.result,
                        "{location}: typed result differs"
                    );
                    let typed = Success {
                        id: frame.id,
                        ok: Bool,
                        result,
                    };
                    assert_eq!(
                        serde_json::to_value(typed).unwrap(),
                        serde_json::to_value(frame).unwrap(),
                        "{location}: typed success envelope differs"
                    );
                }
                Line::S2c {
                    frame: ServerFrame::Event(event),
                } => {
                    event
                        .validate()
                        .unwrap_or_else(|e| panic!("{location}: {e}"));
                    if let Event::MutationResult {
                        op,
                        result: Some(result),
                        ..
                    } = event
                    {
                        if ops::TABLE.iter().any(|row| row.name == op) {
                            let value = Value::Object(result.clone());
                            let typed = Reply::decode(op, value.clone())
                                .unwrap_or_else(|e| panic!("{location}: {e}"));
                            assert_eq!(
                                typed.to_value().unwrap(),
                                value,
                                "{location}: mutation result differs"
                            );
                        }
                    }
                }
                _ => {}
            }
            match &line {
                Line::C2s { frame } => assert!(
                    serde_json::to_vec(frame).unwrap().len() <= LINE_IN,
                    "{location}: client frame exceeds line cap"
                ),
                Line::S2c { frame } => assert!(
                    serde_json::to_vec(frame).unwrap().len() <= LINE_OUT,
                    "{location}: server frame exceeds line cap"
                ),
                _ => {}
            }
            let original: Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                canonical(serde_json::to_value(line).unwrap()),
                canonical(original),
                "{location}: canonical round-trip differs"
            );
        }
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
                    "eta_s" | "rating" | "uploaded_at" | "free_bytes" => true,
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
                    || key == "bps" && path.ends_with("/args")
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
    assert_eq!(operations, ops::TABLE.iter().map(|r| r.name).collect());
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
    let log = LogEntry {
        seq: 1,
        ts: 1,
        level: LogLevel::Info,
        source: "api".into(),
        hash: None,
        message: "ready".into(),
    };
    let chunk = |topic, items| Event::SnapshotChunk {
        sub: 1,
        topic,
        rev: 1,
        items,
    };
    chunk(Topic::Log, vec![Item::Log(log.clone()); MAX_CHUNK_ITEMS])
        .validate()
        .unwrap();
    assert!(chunk(
        Topic::Log,
        vec![Item::Log(log.clone()); MAX_CHUNK_ITEMS + 1]
    )
    .validate()
    .is_err());
    assert!(chunk(Topic::Torrents, vec![Item::Log(log)])
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
