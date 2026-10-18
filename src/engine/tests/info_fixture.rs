// Characterize the existing wire JSON before moving engine code. Only the
// unique temporary root is replaced; every field, order and byte is retained.
fn fixture_info_json() -> String {
    let directory = review_directory("info-isomorphism");
    let downloads = directory.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let mut records = Vec::new();
    for (index, name) in ["complete.txt", "incomplete.txt"].into_iter().enumerate() {
        let mut bytes = format!("d4:infod6:lengthi5e4:name{}:{}12:piece lengthi16384e6:pieces20:", name.len(), name).into_bytes();
        // SHA-1 of the real five-byte payload below.
        bytes.extend_from_slice(&[0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48, 0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
        std::fs::write(directory.join(format!("{hash}.torrent")), bytes).unwrap();
        if index == 0 { std::fs::write(downloads.join(name), b"hello").unwrap(); }
        let mut record = review_record(&hash, &downloads);
        record.name = name.into();
        record.paused = true;
        record.added_on = 10 + index as i64;
        record.completion_on = if index == 0 { 7 } else { 0 };
        record.category = "fixture".into();
        record.tags = "alpha,beta".into();
        records.push(record);
    }
    let app = test_app(&directory, &downloads, Ledger { generation: Default::default(), migrated_legacy: true, records }, None);
    let dispatcher = app.dispatcher();
    dispatcher.runtime.block_on(async {
        let cx = dispatcher.runtime.cx();
        let mut ledger = OwnedMutexGuard::lock(dispatcher.inner.ledger.clone(), cx).await.unwrap();
        for record in &ledger.records {
            time::timeout(cx, Duration::from_secs(5), dispatcher.inner.handle(&record.hash).unwrap().wait_until_initialized(cx))
                .await.unwrap().unwrap();
        }
        // These are controlled unit-fixture restore outcomes, not live network
        // evidence. Existing native.py separately exercises actual restores.
        for (index, (name, outcome)) in [
            ("pending-check", RestoreOutcome::Checking),
            ("pending-metadata", RestoreOutcome::Metadata),
            ("stopped-unknown", RestoreOutcome::Stopped),
            ("restore-error", RestoreOutcome::Failed { reason: "restore_failed", message: "cached metadata is corrupt".into() }),
        ].into_iter().enumerate() {
            let hash = format!("{:040x}", index + 1);
            let mut record = review_record(&hash, &downloads);
            record.name = name.into();
            record.added_on = 20 + index as i64;
            record.paused = matches!(outcome, RestoreOutcome::Stopped);
            record.completion_on = if record.paused { -1 } else { 0 };
            let last_error = match &outcome { RestoreOutcome::Failed { message, .. } => Some(message.clone()), _ => None };
            dispatcher.inner.restores.lock().unwrap().insert(hash, RestoreTask {
                generation: 1, outcome, last_error, task: None, cancel: CancellationToken::new(),
            });
            ledger.records.push(record);
        }
        dispatcher.inner.save(cx, &ledger).await.unwrap();
    });
    let raw = dispatcher.dispatch(&request("torrents/info", &[])).unwrap().unwrap();
    let actual = raw.replace(directory.to_str().unwrap(), "<FIXTURE>");
    let rows: Vec<Torrent> = serde_json::from_str(&actual).unwrap();
    assert_eq!(rows.iter().map(|row| row.state.as_str()).collect::<Vec<_>>(),
        ["error", "stoppedDL", "metaDL", "checkingDL", "stoppedDL", "stoppedUP"]);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
    actual
}

fn first_info_difference(expected: &serde_json::Value, actual: &serde_json::Value, pointer: &str) -> Option<String> {
    use serde_json::Value;
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: std::collections::BTreeSet<_> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let next = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => if let Some(diff) = first_info_difference(left, right, &next) { return Some(diff); },
                    _ => return Some(next),
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                let next = format!("{pointer}/{index}");
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => if let Some(diff) = first_info_difference(left, right, &next) { return Some(diff); },
                    _ => return Some(next),
                }
            }
            None
        }
        _ if expected == actual => None,
        _ => Some(pointer.into()),
    }
}

#[test]
fn info_json_identical_after_split() {
    let actual = fixture_info_json();
    let expected = include_str!("info-before-split.json");
    if expected.as_bytes() != actual.as_bytes() {
        let left = serde_json::from_str(expected).unwrap();
        let right = serde_json::from_str(&actual).unwrap();
        let pointer = first_info_difference(&left, &right, "");
        panic!("torrents/info differs at JSON pointer {:?}; expected {expected}; actual {actual}",
            pointer.as_deref().unwrap_or("(values equal; serialized bytes differ)"));
    }
    println!("torrents/info is byte-identical: {} bytes", actual.len());
}

#[test]
fn info_difference_reports_the_first_json_pointer() {
    let expected = serde_json::json!([{"a/b~": [1, 2]}]);
    let actual = serde_json::json!([{"a/b~": [1, 3]}]);
    assert_eq!(first_info_difference(&expected, &actual, "").as_deref(), Some("/0/a~1b~0/1"));
    assert_eq!(first_info_difference(&expected, &expected, ""), None);
    assert_eq!(first_info_difference(&serde_json::json!({"missing": 1}), &serde_json::json!({}), "").as_deref(), Some("/missing"));
}
