use super::*;
#[test]
fn subscription_operations_are_routed_in_p1_08() {
    for (op, args) in [
        ("subscribe", json!({"topics":["torrents"]})),
        ("unsubscribe", json!({"sub":1})),
    ] {
        assert!(decode(&serde_json::to_vec(&json!({"id":1,"op":op,"args":args})).unwrap()).is_ok());
    }
    let failure = decode(br#"{"id":1,"op":"torrents.lis"}"#).unwrap_err();
    assert_eq!(
        failure.error.details.unwrap()["nearest"],
        json!(["torrents.list"])
    );
}

#[test]
fn oversized_collection_is_an_id_bearing_error_and_does_not_end_connection() {
    let (connection, _) = super::tests::connection();
    let request = decode(br#"{"id":1,"op":"torrents.files","args":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#).unwrap();
    let generation = connection.reserve(&request).unwrap();
    let files = (0..2000)
        .map(|index| File {
            index,
            path: "x".repeat(1024),
            sort_key: "x".repeat(1024),
            size: 1,
            progress: 0.0,
            included: true,
            ready: None,
        })
        .collect();
    connection.complete(
        1,
        generation,
        Ok(Reply::TorrentsFiles(FilesResult { files })),
    );
    let next = decode(br#"{"id":2,"op":"app.client","args":{}}"#).unwrap();
    let generation = connection.reserve(&next).unwrap();
    connection.complete(2, generation, Ok(Reply::AppClient(EmptyResult {})));
    let out = connection.out.lock().unwrap();
    assert!(!out.ending);
    assert!(out.terminal.is_none());
    assert_eq!(out.frames.len(), 2);
    assert_eq!(out.frames[0]["id"], 1);
    assert_eq!(out.frames[0]["error"]["code"], "invalid_message");
    assert_eq!(out.frames[1]["id"], 2);
    assert_eq!(out.frames[1]["ok"], true);
    println!("oversized files: id=1 invalid_message; id=2 succeeds; no bye or close");
}

// Python owns process lifecycle and E.1 records. This opt-in adapter only
// runs the frozen P1-05 Rust decoder/validator on captured live frames.
fn captured_torrents(line: &Value) -> Vec<&Value> {
    let mut rows = Vec::new();
    if let Some(torrent) = line.pointer("/frame/result/torrent") {
        rows.push(torrent);
    }
    if let Some(torrents) = line
        .pointer("/frame/result/torrents")
        .and_then(Value::as_array)
    {
        rows.extend(torrents);
    }
    rows
}

fn validate_live_capture(text: &str) -> Result<(), String> {
    use crate::api::validation::{Revisions, TorrentFileSizes};
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    // A recheck changes the measurement through checking and cancellation.
    // Exclude it from cross-time agreement for that hash's whole history;
    // every original value still receives raw type/bounds validation, and
    // Python's non-checking fixture comparisons remain exact.
    let checking: std::collections::BTreeSet<String> = lines
        .iter()
        .flat_map(captured_torrents)
        .filter(|row| row["state"] == "checking")
        .filter_map(|row| row["hash"].as_str().map(str::to_owned))
        .collect();
    let mut calls = BTreeMap::<u64, (Value, Call)>::new();
    let mut hello = Vec::<Value>::new();
    let mut geometry = TorrentFileSizes::default();
    let mut revisions = Revisions::default();
    let mut subscription_history = BTreeMap::<u64, Vec<Value>>::new();
    for (index, line) in lines.iter().enumerate() {
        let location = format!("live capture line {}", index + 1);
        let validate = |context: &[Value]| {
            let text = context
                .iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            crate::api::fixtures::check_group("live-P1-06", &text)
                .map_err(|error| format!("{location}: {error}"))
        };
        match line["dir"].as_str() {
            Some("c2s") => {
                validate(std::slice::from_ref(line))?;
                let request: Request = serde_json::from_value(line["frame"].clone())
                    .map_err(|error| format!("{location}: {error}"))?;
                calls.insert(request.id.get(), (line.clone(), request.call));
            }
            Some("s2c") => {
                let frame = &line["frame"];
                let call = frame["id"].as_u64().and_then(|id| calls.get(&id));
                let mut context = hello.clone();
                if let Some((request, _)) = call {
                    context.push(request.clone());
                }
                if let Some(sub) = frame["sub"].as_u64() {
                    if let Some(history) = subscription_history.get(&sub) {
                        context.extend(history.iter().cloned());
                    }
                }
                context.push(line.clone());
                // The original frame, including every volatile member,
                // round-trips through the unchanged frozen checker first.
                validate(&context)?;
                for row in captured_torrents(line) {
                    if row["state"] == "checking" {
                        if row["checked"] == true {
                            return Err(format!("{location}: /frame/result/torrent/checked: active check must be false"));
                        }
                        if let (Some(size), Some(completed), Some(progress)) = (
                            row["size"].as_u64(),
                            row["completed_bytes"].as_u64(),
                            row["progress"].as_f64(),
                        ) {
                            if completed > size {
                                return Err(format!("{location}: /frame/result/torrent/completed_bytes: exceeds size"));
                            }
                            let ratio = if size == 0 {
                                0.0
                            } else {
                                completed as f64 / size as f64
                            };
                            if !progress.is_finite() || (progress - ratio).abs() > 1e-9 {
                                return Err(format!("{location}: /frame/result/torrent/progress: must equal completed_bytes/size within 1e-9"));
                            }
                        }
                    }
                }
                if frame["ok"] == true {
                    let (request, call) =
                        call.ok_or_else(|| format!("{location}: response has no request"))?;
                    let mut reply = Reply::decode(call.op(), frame["result"].clone())
                        .map_err(|error| format!("{location}: {error}"))?;
                    if matches!(reply, Reply::Hello(_)) {
                        hello = vec![request.clone(), line.clone()];
                    }
                    if let Call::Unsubscribe { args } = call {
                        revisions.unsubscribe(args.sub);
                        subscription_history.remove(&args.sub);
                    }
                    if let (Call::TorrentsRemove { args }, Reply::TorrentsRemove(result)) = (call, &reply) {
                        if matches!(result.status, MutationResult::Done) {
                            for hash in args.hashes.as_slice() {
                                geometry.removed(hash);
                            }
                        }
                    }
                    let strip_measurement = |row: &mut Torrent| {
                        if checking.contains(row.hash.as_str()) {
                            row.completed_bytes = None;
                        }
                    };
                    match &mut reply {
                        Reply::TorrentsList(result) => {
                            result.torrents.iter_mut().for_each(strip_measurement)
                        }
                        Reply::TorrentsGet(result) => {
                            strip_measurement(&mut result.torrent.torrent)
                        }
                        _ => {}
                    }
                    // The hook itself is unchanged. Size and file totals
                    // are still checked across all captured replies.
                    geometry
                        .reply(call, &reply)
                        .map_err(|error| format!("{location}: {error}"))?;
                } else if frame.get("ev").is_some() {
                    let event: Event = serde_json::from_value(frame.clone())
                        .map_err(|error| format!("{location}: {error}"))?;
                    if let Some((sub, rev)) = event.subscription_revision() {
                        revisions
                            .observe(sub, rev)
                            .map_err(|error| format!("{location}: {error}"))?;
                        subscription_history.entry(sub).or_default().push(line.clone());
                    }
                }
            }
            _ => {
                return Err(format!(
                    "{location}: live trace must contain c2s/s2c frames"
                ))
            }
        }
    }
    Ok(())
}

fn validate_optional_fields(text: &str, checks: &Value) -> Result<(), String> {
    validate_live_capture(text)?;
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for check in checks
        .as_array()
        .ok_or("optional checks must be an array")?
    {
        let line = check["live_line"].as_u64().ok_or("live_line must be u64")? as usize;
        let pointer = check["pointer"]
            .as_str()
            .ok_or("pointer must be a string")?;
        let (parent, key) = pointer.rsplit_once('/').ok_or("member pointer required")?;
        let key = key.replace("~1", "/").replace("~0", "~");
        let mut changed = lines.clone();
        let frame = changed
            .get_mut(line.checked_sub(1).ok_or("line starts at one")?)
            .ok_or("line out of range")?
            .get_mut("frame")
            .ok_or("frame required")?;
        let object = frame
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .ok_or("object parent required")?;
        if object.remove(&key).is_none() {
            return Err(format!("missing optional candidate {pointer}"));
        }
        let modified = changed
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        validate_live_capture(&modified).map_err(|e| {
            format!("extra optional field {pointer} is required or invalid: {e}")
        })?;
        println!("extra optional field live_line={line} pointer={pointer}: present and absent forms pass frozen P1-05 validation");
    }
    Ok(())
}

#[test]
#[ignore = "invoked by daemon.py with FBT_API_CONTRACT_TRACE"]
fn validate_live_transcript() {
    let path = std::env::var("FBT_API_CONTRACT_TRACE").expect("live transcript path");
    let text = std::fs::read_to_string(&path).unwrap();
    let kind = std::env::var("FBT_API_CONTRACT_KIND").unwrap_or_else(|_| "live".into());
    match kind.as_str() {
        "fixture" => crate::api::fixtures::check_group("fixture-P1-06", &text).unwrap(),
        "live" => validate_live_capture(&text).unwrap(),
        _ => panic!("unknown transcript kind: {kind}"),
    }
    if let Ok(optional) = std::env::var("FBT_API_CONTRACT_OPTIONALS") {
        let checks = serde_json::from_str(&std::fs::read_to_string(optional).unwrap()).unwrap();
        validate_optional_fields(&text, &checks).unwrap();
    }
    println!("P1-05 types, bounds, negotiated capabilities and canonical round trips validated ({kind}): {path}");
}

#[test]
fn optional_extras_require_the_frozen_types_in_both_forms() {
    let text = concat!(
        "{\"dir\":\"c2s\",\"frame\":{\"id\":1,\"op\":\"transfer.info\"}}\n",
        "{\"dir\":\"s2c\",\"frame\":{\"id\":1,\"ok\":true,\"result\":{\"session_downloaded\":0,\"session_uploaded\":0,\"dl_bps\":0}}}\n"
    );
    validate_optional_fields(text, &json!([{"live_line":2,"pointer":"/result/dl_bps"}]))
        .unwrap();
    assert!(validate_optional_fields(
        text,
        &json!([{"live_line":2,"pointer":"/result/session_downloaded"}])
    )
    .is_err());
    assert!(validate_optional_fields(
        &text.replace("\"dl_bps\":0", "\"dl_bps\":\"bad\""),
        &json!([])
    )
    .is_err());
    assert!(validate_optional_fields(
        &text.replace("\"dl_bps\":0", "\"invented\":0"),
        &json!([])
    )
    .is_err());
    println!("optional extra: valid present/absent accepted; required removal, invalid type and undeclared field rejected");
}

fn checking_capture() -> Vec<Value> {
    let hash = "ab".repeat(20);
    let paused = json!({"hash":hash,"name":"checking fixture","state":"paused",
        "intent":"pause","staged":false,"checked":true,"seed":false,
        "upload_policy":"default_cap","save_path":"/tmp/fbt-fixture",
        "added_at":1,"force":false,"size":100,"completed_bytes":50,"progress":0.5});
    let mut checking = paused.clone();
    checking["state"] = json!("checking");
    checking["checked"] = json!(false);
    checking["completed_bytes"] = json!(25);
    checking["progress"] = json!(0.25);
    checking["private"] = json!(false);
    let mut cancelled = checking.clone();
    cancelled["state"] = json!("paused");
    cancelled["completed_bytes"] = json!(60);
    cancelled["progress"] = json!(0.6);
    vec![
        json!({"dir":"c2s","frame":{"id":1,"op":"torrents.list","args":{}}}),
        json!({"dir":"s2c","frame":{"id":1,"ok":true,"result":{"torrents":[paused]}}}),
        json!({"dir":"c2s","frame":{"id":2,"op":"torrents.get","args":{"hash":hash}}}),
        json!({"dir":"s2c","frame":{"id":2,"ok":true,"result":{"torrent":checking}}}),
        json!({"dir":"c2s","frame":{"id":3,"op":"torrents.get","args":{"hash":hash}}}),
        json!({"dir":"s2c","frame":{"id":3,"ok":true,"result":{"torrent":cancelled}}}),
    ]
}

fn capture_text(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

#[test]
fn live_checking_measurements_preserve_static_geometry_and_strict_fixtures() {
    let rows = checking_capture();
    let text = capture_text(&rows);
    validate_live_capture(&text).unwrap();
    let strict = crate::api::fixtures::check_group("fixture-P1-06", &text).unwrap_err();
    assert!(
        strict.contains(":4:") && strict.contains("/completed_bytes"),
        "{strict}"
    );
    let mut changed_size = rows.clone();
    changed_size[3]["frame"]["result"]["torrent"]["size"] = json!(200);
    changed_size[3]["frame"]["result"]["torrent"]["progress"] = json!(0.125);
    assert!(validate_live_capture(&capture_text(&changed_size))
        .unwrap_err()
        .contains("/size"));
    let mut nonchecking = rows.clone();
    nonchecking[3]["frame"]["result"]["torrent"]["state"] = json!("paused");
    nonchecking[3]["frame"]["result"]["torrent"]["checked"] = json!(true);
    assert!(validate_live_capture(&capture_text(&nonchecking))
        .unwrap_err()
        .contains("/completed_bytes"));
    let mut wrong_files = rows;
    wrong_files.push(json!({"dir":"c2s","frame":{"id":4,"op":"torrents.files","args":{"hash":"ab".repeat(20)}}}));
    wrong_files.push(json!({"dir":"s2c","frame":{"id":4,"ok":true,"result":{"files":[{
        "index":0,"path":"file","sort_key":"file","size":99,"progress":0.0,"included":true}]}}}));
    assert!(validate_live_capture(&capture_text(&wrong_files))
        .unwrap_err()
        .contains("/frame/result/files"));
    println!("ruling 7: live check/cancellation measurements change; static sizes/file totals and full fixture agreement remain strict");
}

#[test]
fn live_checking_measurements_reject_invalid_raw_values() {
    for (key, value, pointer) in [
        ("checked", json!(true), "torrent.checked: must be false while state is checking"),
        ("completed_bytes", json!(101), "/completed_bytes"),
        ("progress", json!(0.5), "/progress"),
    ] {
        let mut rows = checking_capture();
        rows[3]["frame"]["result"]["torrent"][key] = value;
        let error = validate_live_capture(&capture_text(&rows)).unwrap_err();
        assert!(error.contains(pointer), "{key}: {error}");
    }
    for value in [json!(-1), json!(true), json!("25")] {
        let mut rows = checking_capture();
        rows[3]["frame"]["result"]["torrent"]["completed_bytes"] = value;
        assert!(validate_live_capture(&capture_text(&rows)).is_err());
    }
    let mut zero = checking_capture();
    for row in &mut zero {
        if let Some(torrent) = row.pointer_mut("/frame/result/torrent") {
            torrent["size"] = json!(0);
            torrent["completed_bytes"] = json!(0);
            torrent["progress"] = json!(0.0);
        }
        if let Some(torrents) = row
            .pointer_mut("/frame/result/torrents")
            .and_then(Value::as_array_mut)
        {
            for torrent in torrents {
                torrent["size"] = json!(0);
                torrent["completed_bytes"] = json!(0);
                torrent["progress"] = json!(0.0);
            }
        }
    }
    validate_live_capture(&capture_text(&zero)).unwrap();
    println!("ruling 7: checking checked=true, invalid byte types/bounds and inconsistent ratios reject before agreement projection; zero size is valid");
}

#[test]
fn ruling_four_error_catalogue_text_and_details_are_exact() {
    let rows: Vec<Value> = include_str!("../../../contracts/ipc/v1/errors.ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (line, failure) in [
            (11, decode(br#"{"id":5,"op":"torrents.stpo"}"#).unwrap_err().error),
            (13, decode(br#"{"id":6,"op":"torrents.remove","args":{"hashes":["ffffffffffffffffffffffffffffffffffffffff"],"delete_files":"bad"}}"#).unwrap_err().error),
            (33, busy("inflight")),
            (37, line_overflow(65537)),
        ] {
            assert_eq!(serde_json::to_value(failure).unwrap(), rows[line - 1]["frame"]["error"], "catalogue line {line}");
            println!("ruling 4 catalogue line {line}: error is byte-equivalent through the frozen type");
        }
    let failure = ops::engine_error(anyhow::anyhow!(
        "engine_unavailable state=starting retryable=true"
    ));
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        rows[24]["frame"]["error"]
    );
    println!("ruling 4 catalogue line 25: starting carries engine is starting in details.message");
}

#[test]
fn live_transcript_validator_rejects_invalid_payloads() {
    let request = json!({"dir":"c2s","frame":{"id":1,"op":"torrents.list","args":{}}});
    for frame in [
        json!({"id":1,"ok":true,"result":{"torrents":[]}}),
        json!({"id":1,"ok":false,"error":{"code":"not_found","message":"missing","retryable":false}}),
    ] {
        let text = format!("{request}\n{}\n", json!({"dir":"s2c","frame":frame}));
        crate::api::fixtures::check_group("live-P1-06", &text).unwrap();
    }
    for frame in [
        json!({"id":1,"ok":true,"result":{"torrents":42}}),
        json!({"id":1,"ok":false,"error":{"code":"not_found","message":"missing"}}),
    ] {
        let text = format!("{request}\n{}\n", json!({"dir":"s2c","frame":frame}));
        assert!(crate::api::fixtures::check_group("live-P1-06", &text).is_err());
    }
}

#[test]
fn strict_type_errors_name_the_argument() {
    for (op, args, key) in [
        ("app.client", json!({"interactive":"yes"}), "interactive"),
        ("torrents.start", json!({"hashes":["bad"]}), "hashes"),
        ("torrents.add", json!({"source":{"path":3}}), "source"),
        (
            "torrents.set_upload_limit",
            json!({"hashes":["ab".repeat(20)],"bps":4}),
            "bps",
        ),
        ("app.restart", json!({"scope":"unknown"}), "scope"),
        ("app.info", Value::Null, "args"),
    ] {
        let failure =
            decode(&serde_json::to_vec(&json!({"id":1,"op":op,"args":args})).unwrap()).unwrap_err();
        assert_eq!(failure.error.code, ErrorCode::InvalidArgs);
        assert_eq!(failure.error.details.unwrap()["key"], key);
    }
    let clear = json!({"id":1,"op":"torrents.set_upload_limit","args":{"hashes":["ab".repeat(20)],"bps":null}});
    assert!(decode(&serde_json::to_vec(&clear).unwrap()).is_ok());
}

#[test]
fn live_subscription_validation_retains_revision_history_and_rejects_gaps() {
    let mut lines: Vec<Value> = include_str!("../../../contracts/ipc/v1/subscribe.ndjson")
        .lines().map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| matches!(line["dir"].as_str(), Some("c2s" | "s2c"))).collect();
    let text = |lines: &[Value]| lines.iter().map(|line| format!("{line}\n")).collect::<String>();
    validate_live_capture(&text(&lines)).unwrap();
    let frame = lines.iter_mut().find(|line| line["frame"]["rev"] == 2).unwrap();
    frame["frame"]["rev"] = json!(3);
    let failure = validate_live_capture(&text(&lines)).unwrap_err();
    assert!(failure.contains("must increase by exactly one"), "{failure}");
}

#[test]
fn live_readded_completion_starts_after_confirmed_removal() {
    let hash = "ab".repeat(20);
    let initial = checking_capture();
    let mut readded = initial[1]["frame"]["result"]["torrents"][0].clone();
    readded["completed_bytes"] = json!(100);
    readded["progress"] = json!(1.0);
    readded["private"] = json!(false);
    let rows = vec![
        initial[0].clone(), initial[1].clone(),
        json!({"dir":"c2s","frame":{"id":2,"op":"torrents.remove","args":{"hashes":[hash],"delete_files":false}}}),
        json!({"dir":"s2c","frame":{"id":2,"ok":true,"result":{"state":"done"}}}),
        json!({"dir":"c2s","frame":{"id":3,"op":"torrents.add","args":{"source":{"magnet":format!("magnet:?xt=urn:btih:{hash}")},"stopped":true}}}),
        json!({"dir":"s2c","frame":{"id":3,"ok":true,"result":{"state":"done","hash":hash,"duplicate":false}}}),
        json!({"dir":"c2s","frame":{"id":4,"op":"torrents.get","args":{"hash":hash}}}),
        json!({"dir":"s2c","frame":{"id":4,"ok":true,"result":{"torrent":readded}}}),
    ];
    validate_live_capture(&capture_text(&rows)).unwrap();
    assert!(crate::api::fixtures::check_group("fixture-P1-06", &capture_text(&rows))
        .unwrap_err().contains("/completed_bytes"));
    for response in [
        json!({"id":2,"ok":false,"error":{"code":"internal","message":"remove failed","retryable":false}}),
        json!({"id":2,"ok":true,"result":{"state":"pending","ref":"remove-pending"}}),
    ] {
        let mut unconfirmed = rows.clone();
        unconfirmed[3]["frame"] = response;
        assert!(validate_live_capture(&capture_text(&unconfirmed)).unwrap_err().contains("/completed_bytes"));
    }
    let mut geometry = rows.clone();
    geometry[7]["frame"]["result"]["torrent"]["size"] = json!(200);
    geometry[7]["frame"]["result"]["torrent"]["progress"] = json!(0.5);
    assert!(validate_live_capture(&capture_text(&geometry)).unwrap_err().contains("/size"));
    let mut unchanged_instance = rows.clone();
    unchanged_instance[2]["frame"]["args"]["hashes"] = json!(["cd".repeat(20)]);
    assert!(validate_live_capture(&capture_text(&unchanged_instance)).unwrap_err().contains("/completed_bytes"));
    let mut later = rows;
    later.push(json!({"dir":"c2s","frame":{"id":5,"op":"torrents.get","args":{"hash":hash}}}));
    let mut changed = later[7]["frame"].clone();
    changed["id"] = json!(5);
    changed["result"]["torrent"]["completed_bytes"] = json!(50);
    changed["result"]["torrent"]["progress"] = json!(0.5);
    later.push(json!({"dir":"s2c","frame":changed}));
    assert!(validate_live_capture(&capture_text(&later)).unwrap_err().contains("/completed_bytes"));
    println!("confirmed removal resets only completion; pending/failed/unrelated removal, later drift and changed geometry still fail");
}
