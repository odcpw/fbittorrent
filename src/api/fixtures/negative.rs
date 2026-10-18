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
