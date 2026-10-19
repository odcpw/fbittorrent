use super::*;
use serde_json::json;

fn list() -> Value {
    include_str!("../../../contracts/ipc/v1/torrents.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find_map(|r| {
            r["frame"]["result"]
                .get("torrents")
                .map(|v| json!({"torrents":v}))
        })
        .unwrap()
}
fn summary() -> Value {
    include_str!("../../../contracts/ipc/v1/subscribe.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find_map(|r| {
            (r["frame"]["ev"] == "state" && r["frame"]["topic"] == "summary")
                .then(|| r["frame"]["data"].clone())
        })
        .unwrap()
}
#[test]
fn topics_bounds_since_seq_and_interval_normalization() {
    for (n, valid) in [(0, false), (1, true), (16, true), (17, false)] {
        let raw = json!({"topics":vec!["summary";n]});
        assert_eq!(
            serde_json::from_value::<SubscribeArgs>(raw).is_ok(),
            valid,
            "topics={n}"
        );
    }
    for topics in [json!(["summary"]), json!(["summary", "events"])] {
        let valid = topics.as_array().unwrap().contains(&json!("events"));
        let request = json!({"id":1,"op":"subscribe","args":{"topics":topics,"since_seq":1}});
        assert_eq!(serde_json::from_value::<Request>(request).is_ok(), valid);
    }
    for (raw, effective) in [
        (0, 250),
        (249, 250),
        (250, 250),
        (10_000, 10_000),
        (10_001, 10_000),
        (u64::MAX, 10_000),
    ] {
        let args: SubscribeArgs =
            serde_json::from_value(json!({"topics":["summary"],"interval_ms":raw})).unwrap();
        args.validate().unwrap();
        assert_eq!(args.effective_interval_ms(1000), effective);
        assert_eq!(
            serde_json::to_value(args).unwrap()["interval_ms"],
            raw,
            "normalization must preserve the wire value"
        );
    }
    let args: SubscribeArgs = serde_json::from_value(json!({"topics":["summary"]})).unwrap();
    assert_eq!(args.effective_interval_ms(1000), 1000);
    assert_eq!(args.effective_interval_ms(5000), 5000);
}
#[test]
fn numeric_upload_limits_and_nullable_updates() {
    let hashes = json!(["0123456789abcdef0123456789abcdef01234567"]);
    for (value, valid) in [
        (json!(0), false),
        (json!(16383), false),
        (json!(16384), true),
        (json!(u64::MAX), true),
        (json!(-1), false),
        (json!(16384.5), false),
        (Value::Null, true),
    ] {
        let parsed =
            serde_json::from_value::<UploadLimitArgs>(json!({"hashes":hashes,"bps":value}));
        assert_eq!(parsed.is_ok(), valid, "bps={value}");
        if let Ok(args) = parsed {
            args.validate().unwrap();
        }
    }
    let too_large =
        r#"{"hashes":["0123456789abcdef0123456789abcdef01234567"],"bps":18446744073709551616}"#;
    assert!(serde_json::from_str::<UploadLimitArgs>(too_large).is_err());
    let omitted: UploadLimitArgs = serde_json::from_value(json!({"hashes":hashes})).unwrap();
    assert_eq!(omitted.bps, Update::Unset);
    let null: UploadLimitArgs =
        serde_json::from_value(json!({"hashes":hashes,"bps":null})).unwrap();
    assert_eq!(null.bps, Update::Clear);
    let direct = UploadLimitArgs {
        hashes: omitted.hashes,
        bps: Update::Set(16383),
    };
    assert!(
        direct.validate().is_err(),
        "typed producers must be checked too"
    );
}
#[test]
fn progress_name_message_and_summary_bounds() {
    for (p, valid) in [
        (-f64::EPSILON, false),
        (0.0, true),
        (1.0, true),
        (1.0 + f64::EPSILON, false),
    ] {
        let mut values = list();
        values["torrents"][0]["progress"] = json!(p);
        assert_eq!(
            Reply::decode("torrents.list", values).is_ok(),
            valid,
            "Torrent progress={p}"
        );
        let file =
            json!({"index":0,"path":"a","size":1,"progress":p,"included":true,"sort_key":"a"});
        assert_eq!(
            Reply::decode("torrents.files", json!({"files":[file]})).is_ok(),
            valid,
            "File progress={p}"
        );
        let mut top = summary();
        top["top"][0]["progress"] = json!(p);
        let event: Event = serde_json::from_value(
            json!({"ev":"state","sub":1,"topic":"summary","rev":1,"data":top}),
        )
        .unwrap();
        assert_eq!(event.validate().is_ok(), valid, "TopTorrent progress={p}");
    }
    for p in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(progress(p).is_err());
    }
    for (len, valid) in [(1023, true), (1024, true), (1025, false)] {
        let mut values = list();
        values["torrents"][0]["name"] = json!("é".repeat(len));
        assert_eq!(
            Reply::decode("torrents.list", values).is_ok(),
            valid,
            "list name chars={len}"
        );
    }
    for (len, valid) in [(199, true), (200, true), (201, false)] {
        let error = Error {
            code: ErrorCode::Internal,
            message: "é".repeat(len),
            hint: None,
            retryable: false,
            details: None,
        };
        assert_eq!(error.validate().is_ok(), valid, "Error.message chars={len}");
    }
    for (n, valid) in [(0, true), (5, true), (6, false)] {
        let mut value = summary();
        value["top"] = json!(vec![value["top"][0].clone(); n]);
        let typed: Summary = serde_json::from_value(value).unwrap();
        assert_eq!(typed.validate().is_ok(), valid, "Summary.top={n}");
    }
}
#[test]
fn identifier_hash_list_and_frame_boundaries() {
    for (value, valid) in [
        ("a".repeat(39), false),
        ("a".repeat(40), true),
        ("a".repeat(41), false),
        ("A".repeat(40), false),
        ("é".repeat(40), false),
    ] {
        assert_eq!(Hash::new(value).is_ok(), valid);
    }
    for (len, valid) in [(63, true), (64, true), (65, false)] {
        assert_eq!(MutationRef::new("a".repeat(len)).is_ok(), valid);
        assert_eq!(ClientName::new("é".repeat(len)).is_ok(), valid);
    }
    assert!(MutationRef::new("é").is_err());
    let h = Hash::new("a".repeat(40)).unwrap();
    for (n, valid) in [(0, false), (1, true), (256, true), (257, false)] {
        assert_eq!(Hashes::new(vec![h.clone(); n]).is_ok(), valid);
    }
    for cap in [LINE_IN, LINE_OUT] {
        let exact = Value::String("a".repeat(cap - 3)); // quotes + newline = 3 bytes
        frame(&exact, cap).unwrap();
        let too_long = Value::String("a".repeat(cap - 2));
        assert!(frame(&too_long, cap).is_err());
    }
}
#[test]
fn required_pending_refs_and_app_info_fields() {
    for op in [
        "app.shutdown",
        "app.restart",
        "torrents.add",
        "torrents.remove",
        "torrents.set_upload_limit",
    ] {
        for bad in [
            json!({"state":"pending"}),
            json!({"state":"pending","ref":null}),
            json!({"state":"pending","ref":"x".repeat(65)}),
        ] {
            assert!(
                Reply::decode(op, bad).is_err(),
                "{op} accepted pending without a valid ref"
            );
        }
        Reply::decode(op, json!({"state":"pending","ref":"fixture"})).unwrap();
    }
    assert!(serde_json::from_value::<PendingResult>(json!({"state":"pending"})).is_err());
    assert!(serde_json::from_value::<AddResult>(json!({"state":"pending"})).is_err());
    assert!(serde_json::from_value::<RemoveResult>(json!({"state":"pending"})).is_err());
    assert!(serde_json::from_value::<MutationResult>(json!({"state":"pending"})).is_err());
    let info = include_str!("../../../contracts/ipc/v1/torrents.ndjson")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["frame"]["result"].get("uptime_s").is_some())
        .unwrap()["frame"]["result"]
        .clone();
    Reply::decode("app.info", info.clone()).unwrap();
    for field in ["proto", "caps", "limits", "policy"] {
        let mut missing = info.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            Reply::decode("app.info", missing).is_err(),
            "app.info missing {field}"
        );
    }
}
#[test]
fn revisions_reject_gaps_duplicates_backwards_and_overflow() {
    let mut revisions = Revisions::default();
    assert!(revisions.observe(1, 0).is_err());
    assert!(revisions.observe(1, 2).is_err());
    revisions.observe(1, 1).unwrap();
    assert!(revisions.observe(1, 1).is_err());
    assert!(revisions.observe(1, 3).is_err());
    assert_eq!(revisions.last(1), 1);
    revisions.observe(1, 2).unwrap();
    revisions.observe(2, 1).unwrap();
    revisions.unsubscribe(1);
    revisions.observe(1, 1).unwrap();
    revisions.0.insert(1, u64::MAX - 1);
    revisions.observe(1, u64::MAX).unwrap();
    assert!(revisions.observe(1, 0).is_err());
    assert_eq!(revisions.last(1), u64::MAX);
}

#[test]
fn settings_motion_is_optional_boolean_with_false_default() {
    let transcript = include_str!("../../../contracts/ipc/v1/settings-motion.ndjson");
    super::super::fixtures::check_group("settings-motion", transcript).unwrap();
    let states: Vec<Value> = transcript
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["dir"] == "s2c" && row["frame"]["topic"] == "settings")
        .map(|row| row["frame"].clone())
        .collect();
    assert_eq!(states.len(), 4);
    for (frame, expected) in states.iter().zip([false, true, false, false]) {
        let settings: Settings = serde_json::from_value(frame["data"].clone()).unwrap();
        assert_eq!(settings.reduce_motion(), expected);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            frame["data"],
            "explicit false and absent must each preserve wire presence"
        );
        let event: Event = serde_json::from_value(frame.clone()).unwrap();
        event.validate().unwrap();
        println!(
            "settings rev={} reduce_motion={:?} effective={expected}",
            frame["rev"], settings.reduce_motion
        );
    }
    for invalid in [
        Value::Null,
        json!(0),
        json!(1),
        json!("true"),
        json!([]),
        json!({}),
    ] {
        let mut frame = states[1].clone();
        frame["data"]["reduce_motion"] = invalid.clone();
        assert!(
            serde_json::from_value::<Settings>(frame["data"].clone()).is_err(),
            "Settings.reduce_motion accepted {invalid}"
        );
        assert!(
            serde_json::from_value::<Event>(frame.clone()).is_err(),
            "settings event accepted reduce_motion={invalid}"
        );
        let poisoned = transcript
            .lines()
            .map(|line| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                if row["frame"]["topic"] == "settings" && row["frame"]["rev"] == 2 {
                    row["frame"] = frame.clone();
                }
                serde_json::to_string(&row).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            super::super::fixtures::check_group("settings-motion", &poisoned).is_err(),
            "fixture checker accepted reduce_motion={invalid}"
        );
        println!("rejected reduce_motion={invalid} via Settings, event and fixture decoder");
    }
}
