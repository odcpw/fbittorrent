//! Invalid but losslessly round-trippable frames must fail the same validator
//! used by `roundtrip`, without panic-catching or weakening the expected gap.
use super::*;
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
    let rows: Vec<Value> = GROUPS
        .iter()
        .find(|(g, _)| *g == "torrents")
        .unwrap()
        .1
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
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
    let mut summary = GROUPS
        .iter()
        .find(|(g, _)| *g == "subscribe")
        .unwrap()
        .1
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
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
    let encode = |frames: &[Value]| {
        frames
            .iter()
            .map(|v| v.to_string() + "\n")
            .collect::<String>()
    };
    check_group("negative", &encode(&frames)).unwrap();
    for rev in [0, 1, 2, 4] {
        let mut bad = frames.clone();
        bad[2]["frame"]["rev"] = json!(rev);
        assert!(
            check_group("negative", &encode(&bad)).is_err(),
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
    check_group("negative", &encode(&pending)).unwrap();
    let mut bad = pending.clone();
    bad.last_mut().unwrap()["frame"]["rev"] = json!(1);
    assert!(check_group("negative", &encode(&bad)).is_err());
    pending.push(json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":{}}}));
    pending.extend(frames.clone());
    check_group("negative", &encode(&pending)).unwrap();
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
    for op in ["app.log", "app.stats"] {
        assert!(serde_json::from_value::<Request>(json!({"id":1,"op":op,"args":{}})).is_err());
        assert!(Reply::decode(op, json!({})).is_err());
    }
}

#[test]
fn watch_capability_checks_are_on_the_roundtrip_path() {
    let subscribe = GROUPS
        .iter()
        .find(|(group, _)| *group == "subscribe")
        .unwrap()
        .1;
    check_group("subscribe", subscribe).unwrap();
    let mut lines: Vec<Value> = subscribe
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let caps = lines[2]["frame"]["result"]["caps"].as_array_mut().unwrap();
    assert!(caps.contains(&json!("watch")));
    caps.retain(|cap| cap != "watch");
    let encode = |lines: &[Value]| {
        lines
            .iter()
            .map(|line| line.to_string() + "\n")
            .collect::<String>()
    };
    let error = check_group("watch-negative", &encode(&lines)).unwrap_err();
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
    check_group("watch-omitted", &encode(&lines)).unwrap();
    // Keep the actual no-watch transcript unchanged, including its deliberate
    // revision gap. Its torrent snapshots/models omit playable throughout.
    let gap = GROUPS
        .iter()
        .find(|(group, _)| *group == "subscribe-gap")
        .unwrap()
        .1;
    assert!(!gap.contains("\"watch\"") && !gap.contains("\"playable\""));
    check_group("subscribe-gap", gap).unwrap();
    println!("watch capability positives passed: negotiated watch; no watch with playable omitted; unchanged subscribe-gap");
}

#[test]
fn watch_capability_covers_torrent_replies_and_subscription_payloads() {
    let lines: Vec<Value> = GROUPS
        .iter()
        .find(|(group, _)| *group == "torrents")
        .unwrap()
        .1
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
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
    let mutations = GROUPS
        .iter()
        .find(|(group, _)| *group == "mutations")
        .unwrap()
        .1;
    check_group("mutations", mutations).unwrap();
    let mut lines: Vec<Value> = mutations
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[10]["frame"]["ev"], "torrent.playable");
    let caps = lines[2]["frame"]["result"]["caps"].as_array_mut().unwrap();
    assert!(caps.contains(&json!("watch")));
    caps.retain(|cap| cap != "watch");
    let no_watch = lines
        .iter()
        .map(|line| line.to_string() + "\n")
        .collect::<String>();
    let error = check_group("mutations", &no_watch).unwrap_err();
    assert_eq!(
        error,
        "contracts/ipc/v1/mutations.ndjson:11: ev: torrent.playable requires the watch capability"
    );
    println!("playable event positive transcript passed; no-watch control rejected: {error}");
}

#[test]
fn watch_capability_gates_playable_events_in_subscription_batches() {
    let lines: Vec<Value> = GROUPS
        .iter()
        .find(|(group, _)| *group == "mutations")
        .unwrap()
        .1
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
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
    let lines: Vec<Value> = GROUPS
        .iter()
        .find(|(group, _)| *group == "mutations")
        .unwrap()
        .1
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
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

#[test]
fn missing_files_restore_errors_require_the_matching_reason() {
    let detail: Value = GROUPS
        .iter()
        .find(|(group, _)| *group == "torrents")
        .unwrap()
        .1
        .lines()
        .nth(54)
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["frame"]["result"]["torrent"].clone()
        })
        .unwrap();
    assert_eq!(detail["name"], "Fixture error");
    assert_eq!(
        detail["error"],
        "Fixture error restore failed: missing files"
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
                "Fixture error restore failed: missing files",
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
