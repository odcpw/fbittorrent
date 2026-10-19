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
fn validate_optional_fields(text: &str, checks: &Value) -> Result<(), String> {
    crate::api::fixtures::check_group("live-P1-06", text)?;
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
        crate::api::fixtures::check_group("optional-P1-06", &modified)
            .map_err(|e| format!("extra optional field {pointer} is required or invalid: {e}"))?;
        println!("extra optional field live_line={line} pointer={pointer}: present and absent forms pass frozen P1-05 validation");
    }
    Ok(())
}

#[test]
#[ignore = "invoked by daemon.py with FBT_API_CONTRACT_TRACE"]
fn validate_live_transcript() {
    let path = std::env::var("FBT_API_CONTRACT_TRACE").expect("live transcript path");
    let text = std::fs::read_to_string(&path).unwrap();
    crate::api::fixtures::check_group("live-P1-06", &text).unwrap();
    if let Ok(optional) = std::env::var("FBT_API_CONTRACT_OPTIONALS") {
        let checks = serde_json::from_str(&std::fs::read_to_string(optional).unwrap()).unwrap();
        validate_optional_fields(&text, &checks).unwrap();
    }
    println!(
        "P1-05 types, bounds, negotiated capabilities and canonical round trips validated: {path}"
    );
}

#[test]
fn optional_extras_require_the_frozen_types_in_both_forms() {
    let text = concat!(
        "{\"dir\":\"c2s\",\"frame\":{\"id\":1,\"op\":\"transfer.info\"}}\n",
        "{\"dir\":\"s2c\",\"frame\":{\"id\":1,\"ok\":true,\"result\":{\"session_downloaded\":0,\"session_uploaded\":0,\"dl_bps\":0}}}\n"
    );
    validate_optional_fields(text, &json!([{"live_line":2,"pointer":"/result/dl_bps"}])).unwrap();
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
    assert!(
        validate_optional_fields(&text.replace("\"dl_bps\":0", "\"invented\":0"), &json!([]))
            .is_err()
    );
    println!("optional extra: valid present/absent accepted; required removal, invalid type and undeclared field rejected");
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
