//! Invalid but losslessly round-trippable frames must fail the same validator
//! used by `roundtrip`, without panic-catching or weakening the expected gap.
use super::*;

fn fixture(group: &str) -> &'static str {
    GROUPS.iter().find(|(name, _)| *name == group).unwrap().1
}

fn fixture_rows(group: &str) -> impl Iterator<Item = Value> {
    fixture(group)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn encode_frames(frames: &[Value]) -> String {
    frames.iter().map(|frame| format!("{frame}\n")).collect()
}

fn transcript(op: &str, result: Value) -> String {
    format!(
        "{}\n{}\n",
        json!({"dir":"c2s","frame":{"id":1,"op":op,"args":args(op)}}),
        json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":result}})
    )
}
fn args(op: &str) -> Value {
    match op {
        "torrents.add" => json!({"source":{"url":"https://example.invalid/fixture.torrent"}}),
        "torrents.remove" => json!({"hashes":["a".repeat(40)],"delete_files":true}),
        "torrents.set_upload_limit" => json!({"hashes":["a".repeat(40)],"bps":16384}),
        "app.restart" => json!({"scope":"engine"}),
        _ => json!({}),
    }
}
#[test]
fn pending_ref_checks_are_on_the_roundtrip_path() {
    for op in [
        "app.shutdown",
        "app.restart",
        "torrents.add",
        "torrents.remove",
        "torrents.set_upload_limit",
    ] {
        let valid = transcript(op, json!({"state":"pending","ref":"fixture"}));
        check_group("negative", &valid).unwrap();
        let invalid = transcript(op, json!({"state":"pending"}));
        let error = check_group("negative", &invalid).unwrap_err();
        assert!(error.contains("negative.ndjson:2:"), "{error}");
    }
}
#[test]
fn bounds_cannot_pass_just_by_roundtripping() {
    for args in [
        json!({"topics":[]}),
        json!({"topics":vec!["summary";17]}),
        json!({"topics":["summary"],"since_seq":1}),
    ] {
        let line = json!({"dir":"c2s","frame":{"id":1,"op":"subscribe","args":args}}).to_string();
        assert!(check_group("negative", &line).is_err());
    }
    let bad_upload=json!({"dir":"c2s","frame":{"id":1,"op":"torrents.set_upload_limit","args":{"hashes":["a".repeat(40)],"bps":16383}}}).to_string();
    assert!(check_group("negative", &bad_upload).is_err());
    let rows: Vec<_> = fixture_rows("torrents").collect();
    let list = rows
        .iter()
        .find_map(|r| {
            r["frame"]["result"]
                .get("torrents")
                .map(|v| json!({"torrents":v}))
        })
        .unwrap();
    for (field, value) in [
        ("progress", json!(1.1)),
        ("progress", json!(-0.1)),
        ("name", json!("x".repeat(1025))),
    ] {
        let mut bad = list.clone();
        bad["torrents"][0][field] = value;
        assert!(check_group("negative", &transcript("torrents.list", bad)).is_err());
    }
    let file = json!({"index":0,"path":"a","size":1,"progress":1.1,"included":true,"sort_key":"a"});
    let f = json!({"dir":"s2c","frame":{"ev":"snapshot.chunk","sub":1,"topic":format!("torrent:{}.files","a".repeat(40)),"rev":1,"items":[file]}});
    assert!(check_group("negative", &f.to_string()).is_err());
    let mut summary = fixture_rows("subscribe")
        .find(|r| r["frame"]["topic"] == "summary")
        .unwrap();
    summary["frame"]["rev"] = json!(1);
    summary["frame"]["data"]["top"][0]["progress"] = json!(1.1);
    assert!(check_group("negative", &summary.to_string()).is_err());
    summary["frame"]["data"]["top"][0]["progress"] = json!(0.5);
    summary["frame"]["data"]["top"] = json!(vec![summary["frame"]["data"]["top"][0].clone(); 6]);
    assert!(check_group("negative", &summary.to_string()).is_err());
    let bad_error = json!({"dir":"s2c","frame":{"id":1,"ok":false,"error":{"code":"internal","message":"x".repeat(201),"retryable":false}}});
    assert!(check_group("negative", &bad_error.to_string()).is_err());
}
#[test]
fn revision_and_typed_loss_checks_remain_on_roundtrip_path() {
    let frames = vec![
        json!({"dir":"s2c","frame":{"ev":"snapshot.begin","sub":1,"topic":"torrents","rev":1,"count":0,"reason":"subscribe"}}),
        json!({"dir":"s2c","frame":{"ev":"snapshot.chunk","sub":1,"topic":"torrents","rev":2,"items":[]}}),
        json!({"dir":"s2c","frame":{"ev":"snapshot.end","sub":1,"topic":"torrents","rev":3}}),
    ];
    check_group("negative", &encode_frames(&frames)).unwrap();
    for rev in [0, 1, 2, 4] {
        let mut bad = frames.clone();
        bad[2]["frame"]["rev"] = json!(rev);
        assert!(
            check_group("negative", &encode_frames(&bad)).is_err(),
            "accepted rev {rev}"
        );
    }
    // Client intent alone must not reset the counter: an in-flight server
    // frame before the unsubscribe reply still belongs to the old revision.
    let mut pending = frames.clone();
    pending.push(json!({"dir":"c2s","frame":{"id":1,"op":"unsubscribe","args":{"sub":1}}}));
    pending.push(
        json!({"dir":"s2c","frame":{"ev":"delta","sub":1,"topic":"torrents","rev":4,"removed":[]}}),
    );
    check_group("negative", &encode_frames(&pending)).unwrap();
    let mut bad = pending.clone();
    bad.last_mut().unwrap()["frame"]["rev"] = json!(1);
    assert!(check_group("negative", &encode_frames(&bad)).is_err());
    pending.push(json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":{}}}));
    pending.extend(frames.clone());
    check_group("negative", &encode_frames(&pending)).unwrap();
    let extra = transcript(
        "torrents.add",
        json!({"state":"done","unknown_typed_field":true}),
    );
    assert!(check_group("negative", &extra)
        .unwrap_err()
        .contains("typed success envelope differs"));
    let mutation = json!({"dir":"s2c","frame":{"ev":"mutation.result","seq":1,"ts":1,"ref":"fixture","op":"torrents.add","outcome":"done","result":{"state":"done","unknown_typed_field":true}}});
    assert!(check_group("negative", &mutation.to_string())
        .unwrap_err()
        .contains("typed mutation result differs"));
    for op in ["app.not_a_bead", "app.unassigned"] {
        assert!(serde_json::from_value::<Request>(json!({"id":1,"op":op,"args":{}})).is_err());
        assert!(Reply::decode(op, json!({})).is_err());
    }
}

#[test]
fn watch_capability_checks_are_on_the_roundtrip_path() {
    let subscribe = fixture("subscribe");
    check_group("subscribe", subscribe).unwrap();
    let mut lines: Vec<_> = fixture_rows("subscribe").collect();
    let caps = lines[2]["frame"]["result"]["caps"].as_array_mut().unwrap();
    assert!(caps.contains(&json!("watch")));
    caps.retain(|cap| cap != "watch");
    let error = check_group("watch-negative", &encode_frames(&lines)).unwrap_err();
    assert!(error.contains("watch-negative.ndjson:6:"), "{error}");
    assert!(
        error.contains("summary.top[0].playable: requires the watch capability"),
        "{error}"
    );
    println!("watch capability negative control rejected: {error}");

    assert!(lines[5]["frame"]["data"]["top"][0]
        .as_object_mut()
        .unwrap()
        .remove("playable")
        .is_some());
    check_group("watch-omitted", &encode_frames(&lines)).unwrap();
    // Canonical hello lists track the live caps. Derive an explicit no-watch
    // control while preserving every payload and deliberate revision gap.
    let gap = fixture("subscribe-gap");
    check_group("subscribe-gap",gap).unwrap();
    let mut gap: Vec<_> = fixture_rows("subscribe-gap").collect();
    for line in &mut gap {if let Some(caps)=line.get_mut("frame").and_then(|frame|frame.get_mut("result")).and_then(|result|result.get_mut("caps")).and_then(Value::as_array_mut){caps.retain(|cap|cap!="watch");}}
    let gap = encode_frames(&gap);
    assert!(!gap.contains("\"watch\"") && !gap.contains("\"playable\""));
    check_group("subscribe-gap", &gap).unwrap();
    println!("watch capability positives passed: live-cap canonical gap and no-watch gap with all payloads unchanged");
}

#[test]
fn watch_capability_covers_torrent_replies_and_subscription_payloads() {
    let lines: Vec<_> = fixture_rows("torrents").collect();
    let hello = format!("{}\n{}\n", lines[1], lines[2]);
    assert!(lines[2]["frame"]["result"]["caps"]
        .as_array()
        .unwrap()
        .contains(&json!("watch")));
    let result = lines
        .iter()
        .find_map(|line| line["frame"]["result"].get("torrents"))
        .unwrap();
    let torrent = result[0].clone();
    assert!(torrent.get("playable").is_some());
    let detail = lines
        .iter()
        .find_map(|line| line["frame"]["result"].get("torrent"))
        .unwrap();
    assert!(detail.get("playable").is_some());
    let hash = torrent["hash"].as_str().unwrap();
    let get = format!(
        "{}\n{}\n",
        json!({"dir":"c2s","frame":{"id":1,"op":"torrents.get","args":{"hash":hash}}}),
        json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":{"torrent":detail}}})
    );
    let mut payloads = vec![
        transcript("torrents.list", json!({"torrents":[torrent.clone()]})),
        get,
    ];
    for event in [
        json!({"ev":"snapshot.chunk","sub":1,"topic":"torrents","rev":1,"items":[torrent.clone()]}),
        json!({"ev":"delta","sub":1,"topic":"torrents","rev":1,"added":[torrent.clone()]}),
        json!({"ev":"delta","sub":1,"topic":"torrents","rev":1,"changed":{hash:{"playable":torrent["playable"]}}}),
    ] {
        payloads.push(json!({"dir":"s2c","frame":event}).to_string() + "\n");
    }
    for payload in payloads {
        check_group("watch-present", &(hello.clone() + &payload)).unwrap();
        let error = check_group("watch-absent", &payload).unwrap_err();
        assert!(
            error.contains("playable") && error.contains("requires the watch capability"),
            "{error}"
        );
    }
    let removal = json!({"dir":"s2c","frame":{"ev":"delta","sub":1,"topic":"torrents","rev":1,"changed":{hash:{"playable":null}}}});
    check_group("watch-deletion", &removal.to_string()).unwrap();
}

#[test]
fn watch_capability_gates_playable_events_on_the_roundtrip_path() {
    let mutations = fixture("mutations");
    check_group("mutations", mutations).unwrap();
    let mut lines: Vec<_> = fixture_rows("mutations").collect();
    assert_eq!(lines[10]["frame"]["ev"], "torrent.playable");
    let caps = lines[2]["frame"]["result"]["caps"].as_array_mut().unwrap();
    assert!(caps.contains(&json!("watch")));
    caps.retain(|cap| cap != "watch");
    let no_watch = encode_frames(&lines);
    let error = check_group("mutations", &no_watch).unwrap_err();
    assert_eq!(
        error,
        "contracts/ipc/v1/mutations.ndjson:11: ev: torrent.playable requires the watch capability"
    );
    println!("playable event positive transcript passed; no-watch control rejected: {error}");
}

#[test]
fn watch_capability_gates_playable_events_in_subscription_batches() {
    let lines: Vec<_> = fixture_rows("mutations").collect();
    let hello = format!("{}\n{}\n", lines[1], lines[2]);
    let mut no_watch_hello = lines[2].clone();
    no_watch_hello["frame"]["result"]["caps"]
        .as_array_mut()
        .unwrap()
        .retain(|cap| cap != "watch");
    let no_watch_hello = format!("{}\n{no_watch_hello}\n", lines[1]);
    let ordinary = lines[8]["frame"].clone();
    assert_eq!(ordinary["ev"], "torrent.added");
    let playable = lines[10]["frame"].clone();
    assert_eq!(playable["ev"], "torrent.playable");
    for (name, field, mut frame) in [
        (
            "snapshot",
            "items",
            json!({"ev":"snapshot.chunk","sub":1,"topic":"events","rev":1}),
        ),
        (
            "delta",
            "added",
            json!({"ev":"delta","sub":1,"topic":"events","rev":1}),
        ),
    ] {
        // The first item requires no watch cap. Check later batch members too.
        frame[field] = json!([ordinary, playable]);
        let payload = json!({"dir":"s2c","frame":frame}).to_string() + "\n";
        check_group(name, &(hello.clone() + &payload)).unwrap();
        let error = check_group(name, &(no_watch_hello.clone() + &payload)).unwrap_err();
        assert_eq!(error, format!("contracts/ipc/v1/{name}.ndjson:3: {field}[1].ev: torrent.playable requires the watch capability"));
        println!("playable {name} batch positive passed; no-watch control rejected: {error}");

        // Ordinary event batches remain valid without watch.
        frame[field] = json!([ordinary]);
        let payload = json!({"dir":"s2c","frame":frame}).to_string() + "\n";
        check_group(name, &(no_watch_hello.clone() + &payload)).unwrap();
    }
}

#[test]
fn events_changed_entries_are_rejected_with_and_without_watch() {
    let lines: Vec<_> = fixture_rows("mutations").collect();
    let playable = lines[10]["frame"].clone();
    assert_eq!(playable["ev"], "torrent.playable");
    for watch in [true, false] {
        let mut hello = lines[2].clone();
        let caps = hello["frame"]["result"]["caps"].as_array_mut().unwrap();
        assert!(caps.contains(&json!("watch")));
        if !watch {
            caps.retain(|cap| cap != "watch");
        }
        let prefix = format!("{}\n{hello}\n", lines[1]);
        let group = if watch {
            "events-changed-watch"
        } else {
            "events-changed-no-watch"
        };
        // Even an entry with no fields violates append-only event history.
        for fields in [playable.clone(), json!({})] {
            let frame = json!({"dir":"s2c","frame":{"ev":"delta","sub":1,"topic":"events","rev":1,"changed":{"3":fields}}});
            let error =
                check_group(group, &(prefix.clone() + &frame.to_string() + "\n")).unwrap_err();
            assert_eq!(
                error,
                format!("contracts/ipc/v1/{group}.ndjson:3: changed: events topic is append-only")
            );
            println!("append-only event changes watch={watch}: rejected {error}");
        }
        // An explicitly empty map has no event updates and remains valid.
        let empty = json!({"dir":"s2c","frame":{"ev":"delta","sub":1,"topic":"events","rev":1,"changed":{}}});
        check_group(group, &(prefix + &empty.to_string() + "\n")).unwrap();
    }
}

fn file_size_frames(op: &str) -> (Vec<Value>, Vec<Value>) {
    let rows: Vec<_> = fixture_rows("torrents").collect();
    let mut value = if op == "torrents.list" {
        rows[12]["frame"]["result"]["torrents"][0].clone()
    } else {
        rows[14]["frame"]["result"]["torrent"].clone()
    };
    // These controls exercise sizes without requiring a hello/watch prefix.
    value.as_object_mut().unwrap().remove("playable");
    let hash = value["hash"].clone();
    let (args, result) = if op == "torrents.list" {
        (json!({}), json!({"torrents":[value]}))
    } else {
        (json!({"hash":hash}), json!({"torrent":value}))
    };
    (
        vec![
            json!({"dir":"c2s","frame":{"id":1,"op":op,"args":args}}),
            json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":result}}),
        ],
        vec![
            json!({"dir":"c2s","frame":{"id":2,"op":"torrents.files","args":{"hash":hash}}}),
            json!({"dir":"s2c","frame":{"id":2,"ok":true,"result":rows.iter().find(|row|row["frame"]["result"]["files"].is_array()).unwrap()["frame"]["result"]}}),
        ],
    )
}

#[test]
fn inconsistent_catalogue_file_lengths_fail_at_the_exact_line_and_pointer() {
    let text = fixture("torrents");
    check_group("torrents", text).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let index=lines.iter().position(|line|serde_json::from_str::<Value>(line).unwrap()["frame"]["result"]["files"].is_array()).unwrap();
    assert_eq!(lines[index].matches("\"size\":2048").count(), 2);
    // Recreate precisely the original inconsistent files reply, preserving all
    // other records, and send it through the ordinary round-trip checker.
    lines[index] = lines[index].replace("\"size\":2048", "\"size\":4096");
    let error = check_group("files-size-negative", &(lines.join("\n") + "\n")).unwrap_err();
    assert_eq!(error, format!("contracts/ipc/v1/files-size-negative.ndjson:{}: /frame/result/files: sum of file lengths must equal the torrent size for the same hash",index+1));
    println!("original file-length mismatch rejected: {error}");
}

#[test]
fn file_lengths_match_list_and_detail_sizes_in_both_reply_orders() {
    for op in ["torrents.list", "torrents.get"] {
        for files_first in [false, true] {
            let (torrent, files) = file_size_frames(op);
            let frames = if files_first {
                [files.clone(), torrent.clone()].concat()
            } else {
                [torrent.clone(), files.clone()].concat()
            };
            check_group("files-size-positive", &encode_frames(&frames)).unwrap();
            let mut wrong_files = files;
            wrong_files[1]["frame"]["result"]["files"][0]["size"] = json!(2049);
            let (frames, pointer, rule) = if files_first {
                (
                    [wrong_files, torrent].concat(),
                    if op == "torrents.list" {
                        "/frame/result/torrents/0/size"
                    } else {
                        "/frame/result/torrent/size"
                    },
                    "must equal the sum of file lengths for the same hash",
                )
            } else {
                (
                    [torrent, wrong_files].concat(),
                    "/frame/result/files",
                    "sum of file lengths must equal the torrent size for the same hash",
                )
            };
            let error =
                check_group("files-size-negative", &encode_frames(&frames)).unwrap_err();
            assert_eq!(
                error,
                format!("contracts/ipc/v1/files-size-negative.ndjson:4: {pointer}: {rule}")
            );
            println!("{op} files_first={files_first}: positive passed; negative rejected {error}");
        }
    }
}

#[test]
fn file_size_checks_preserve_earlier_observations_and_hash_scope() {
    let (mut list, files) = file_size_frames("torrents.list");
    let (mut get, _) = file_size_frames("torrents.get");
    for frame in &mut get {
        frame["frame"]["id"] = json!(3);
    }
    list[1]["frame"]["result"]["torrents"][0]["size"] = json!(4097);
    let frames = [list.clone(), get.clone(), files.clone()].concat();
    let error = check_group("files-size-history", &encode_frames(&frames)).unwrap_err();
    assert_eq!(error, "contracts/ipc/v1/files-size-history.ndjson:4: /frame/result/torrent/size: must agree with earlier list/get replies for the same hash");

    // A file reply can arrive before any torrent reply as well. Retain its
    // total even if a later file reply happens to have the correct total.
    let mut wrong_files = files.clone();
    for frame in &mut wrong_files {
        frame["frame"]["id"] = json!(4);
    }
    wrong_files[1]["frame"]["result"]["files"][0]["size"] = json!(2049);
    let frames = [wrong_files, files.clone(), get.clone()].concat();
    let error = check_group("files-size-history", &encode_frames(&frames)).unwrap_err();
    assert_eq!(error, "contracts/ipc/v1/files-size-history.ndjson:6: /frame/result/torrent/size: must equal the sum of file lengths for the same hash");

    list[1]["frame"]["result"]["torrents"][0]["hash"] = json!("f".repeat(40));
    // Different hashes may have different sizes: correlate the files request,
    // not merely the most recent list/get response or response arrival order.
    let frames = [list, files, get].concat();
    check_group("files-size-hashes", &encode_frames(&frames)).unwrap();
    println!("earlier list/files observations retained; different hashes remain independent");
}

#[test]
fn file_size_sum_checks_empty_lists_and_overflow() {
    let (mut torrent, mut files) = file_size_frames("torrents.list");
    torrent[1]["frame"]["result"]["torrents"][0]["size"] = json!(u64::MAX);
    files[1]["frame"]["result"]["files"][0]["size"] = json!(u64::MAX);
    files[1]["frame"]["result"]["files"][1]["size"] = json!(0);
    let frames = [torrent.clone(), files.clone()].concat();
    check_group("files-size-boundary", &encode_frames(&frames)).unwrap();
    files[1]["frame"]["result"]["files"][1]["size"] = json!(1);
    let frames = [torrent.clone(), files.clone()].concat();
    let error = check_group("files-size-overflow", &encode_frames(&frames)).unwrap_err();
    assert_eq!(error, "contracts/ipc/v1/files-size-overflow.ndjson:4: /frame/result/files/1/size: sum of file lengths exceeds u64");
    torrent[1]["frame"]["result"]["torrents"][0]["size"] = json!(0);
    files[1]["frame"]["result"]["files"] = json!([]);
    let frames = [torrent.clone(), files.clone()].concat();
    check_group("files-size-empty", &encode_frames(&frames)).unwrap();
    torrent[1]["frame"]["result"]["torrents"][0]["size"] = json!(1);
    let frames = [torrent, files].concat();
    let error = check_group("files-size-empty", &encode_frames(&frames)).unwrap_err();
    assert_eq!(error, "contracts/ipc/v1/files-size-empty.ndjson:4: /frame/result/files: sum of file lengths must equal the torrent size for the same hash");
    println!("file sums: u64::MAX and empty zero-size lists pass; overflow and mismatched empty lists reject");
}

#[test]
fn list_get_reply_agreement_covers_both_fields_and_all_reply_orders() {
    for earlier_op in ["torrents.list", "torrents.get"] {
        for later_op in ["torrents.list", "torrents.get"] {
            let (earlier, _) = file_size_frames(earlier_op);
            let (mut later, _) = file_size_frames(later_op);
            for frame in &mut later {
                frame["frame"]["id"] = json!(3);
            }
            let frames = [earlier.clone(), later.clone()].concat();
            check_group(
                "reply-agreement-positive",
                &encode_frames(&frames),
            )
            .unwrap();
            for field in ["size", "completed_bytes"] {
                let mut wrong = later.clone();
                let value = if later_op == "torrents.list" {
                    &mut wrong[1]["frame"]["result"]["torrents"][0]
                } else {
                    &mut wrong[1]["frame"]["result"]["torrent"]
                };
                value[field] = json!(value[field].as_u64().unwrap() + 1);
                let frames = [earlier.clone(), wrong].concat();
                let error = check_group(
                    "reply-agreement-negative",
                    &encode_frames(&frames),
                )
                .unwrap_err();
                let object = if later_op == "torrents.list" {
                    "torrents/0"
                } else {
                    "torrent"
                };
                assert_eq!(error, format!("contracts/ipc/v1/reply-agreement-negative.ndjson:4: /frame/result/{object}/{field}: must agree with earlier list/get replies for the same hash"));
                println!("{earlier_op} then {later_op}, {field}: matching replies pass; negative rejected {error}");
            }
        }
    }
}

#[test]
fn catalogue_list_get_disagreements_fail_without_a_files_reply() {
    for (hash, field, wrong_value) in [
        ("2".repeat(40), "size", 4096),
        ("2".repeat(40), "completed_bytes", 2048),
        ("a".repeat(40), "size", 4096),
        ("a".repeat(40), "completed_bytes", 1),
    ] {
        let mut frames: Vec<_> = fixture_rows("torrents").collect();
        let line=frames.iter().position(|row|row["frame"]["result"]["torrent"]["hash"]==hash).unwrap()+1;
        let torrent = frames[12]["frame"]["result"]["torrents"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|torrent| torrent["hash"] == hash)
            .unwrap();
        torrent[field] = json!(wrong_value);
        let error = check_group(
            "reply-agreement-catalogue",
            &encode_frames(&frames),
        )
        .unwrap_err();
        assert_eq!(error, format!("contracts/ipc/v1/reply-agreement-catalogue.ndjson:{line}: /frame/result/torrent/{field}: must agree with earlier list/get replies for the same hash"));
        println!("catalogue stale {field} rejected without files: {error}");
    }
}

#[test]
fn missing_files_restore_errors_require_the_matching_reason() {
    let detail = fixture_rows("torrents")
        .map(|row| row["frame"]["result"]["torrent"].clone())
        .find(|row|row["name"]=="Fixture error")
        .unwrap();
    assert_eq!(detail["name"], "Fixture error");
    assert_eq!(
        detail["error"],
        "Files missing: Fixture error restore failed: missing files"
    );
    assert_eq!(detail["reason"], "missing_files");
    for op in ["torrents.list", "torrents.get"] {
        let encode = |mut torrent: Value| {
            let (args, result) = if op == "torrents.list" {
                torrent.as_object_mut().unwrap().remove("private");
                (json!({}), json!({"torrents":[torrent]}))
            } else {
                (json!({"hash":torrent["hash"]}), json!({"torrent":torrent}))
            };
            format!(
                "{}\n{}\n",
                json!({"dir":"c2s","frame":{"id":1,"op":op,"args":args}}),
                json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":result}})
            )
        };
        check_group("missing-files-positive", &encode(detail.clone())).unwrap();
        for (message, reason) in [
            (
                "Files missing: Fixture error restore failed: missing files",
                Some("restore_failed"),
            ),
            ("Fixture error restore failed: missing files", None),
            // Reproduce the entire original error/reason combination, including
            // its truncated name, rather than testing just the corrected text.
            (
                "Fixture restore failed: missing files",
                Some("restore_failed"),
            ),
        ] {
            let mut invalid = detail.clone();
            invalid["error"] = json!(message);
            if let Some(reason) = reason {
                invalid["reason"] = json!(reason);
            } else {
                invalid.as_object_mut().unwrap().remove("reason");
            }
            let error = check_group("missing-files-negative", &encode(invalid)).unwrap_err();
            assert_eq!(error, format!("contracts/ipc/v1/missing-files-negative.ndjson:2: {op} result: torrent.reason: missing-files restore errors require missing_files"));
            println!("{op} rejected message={message:?}, reason={reason:?}: {error}");
        }
        let mut other_failure = detail.clone();
        other_failure["error"] = json!("Fixture error could not restore torrent: invalid metadata");
        other_failure["reason"] = json!("restore_failed");
        check_group("other-restore-positive", &encode(other_failure)).unwrap();
        println!("{op}: missing_files positive and unrelated restore_failed positive passed");
    }
}

#[test]
fn checking_torrents_cannot_claim_a_completed_hash_check() {
    let detail = fixture_rows("torrents")
        .map(|row| row["frame"]["result"]["torrent"].clone())
        .find(|row|row["name"]=="Fixture checking")
        .unwrap();
    assert_eq!(detail["state"], "checking");
    assert_eq!(detail["checked"], false);
    assert_eq!(detail["completed_bytes"], 33_554_432);
    assert_eq!(detail["progress"], 0.5);
    for op in ["torrents.list", "torrents.get"] {
        let encode = |mut torrent: Value| {
            let (args, result) = if op == "torrents.list" {
                torrent.as_object_mut().unwrap().remove("private");
                (json!({}), json!({"torrents":[torrent]}))
            } else {
                (json!({"hash":torrent["hash"]}), json!({"torrent":torrent}))
            };
            format!(
                "{}\n{}\n",
                json!({"dir":"c2s","frame":{"id":1,"op":op,"args":args}}),
                json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":result}})
            )
        };
        check_group("checking-positive", &encode(detail.clone())).unwrap();
        let mut invalid = detail.clone();
        invalid["checked"] = json!(true);
        let error = check_group("checking-negative", &encode(invalid)).unwrap_err();
        assert_eq!(error, format!("contracts/ipc/v1/checking-negative.ndjson:2: {op} result: torrent.checked: must be false while state is checking"));
        println!("{op}: checking/unchecked passed; checking/checked rejected {error}");

        let mut paused = detail.clone();
        paused["state"] = json!("paused");
        paused["checked"] = json!(true);
        check_group("checked-paused-positive", &encode(paused)).unwrap();
        println!("{op}: paused/checked positive passed");
    }
}
