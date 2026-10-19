use super::*;
#[test]
fn collection_transport_splits_by_bytes_and_preserves_items_and_revisions() {
    let (connection, peer) = super::tests::connection();
    let connection = Arc::new(connection);
    let sink = EventSink(connection.clone());
    let item = |index| {
        Item::File(File {
            index,
            path: "💾".repeat(2000),
            size: 1,
            progress: 0.0,
            included: true,
            sort_key: index.to_string(),
            ready: None,
        })
    };
    let topic = Topic::Files(Hash::new("ab".repeat(20)).unwrap());
    let end = sink
        .snapshot(
            7,
            topic.clone(),
            3,
            (0..1000).map(item).collect(),
            SnapshotReason::Subscribe,
        )
        .unwrap();
    let last = sink
        .added(7, topic, end, (1000..1100).map(item).collect())
        .unwrap();
    let frames: Vec<_> = connection.out.lock().unwrap().frames.drain(..).collect();
    let read = thread::spawn(move || {
        let mut found = Vec::new();
        let mut revisions = Vec::new();
        let mut max = 0;
        for line in BufReader::new(peer).lines() {
            let line = line.unwrap();
            max = max.max(line.len() + 1);
            assert!(line.len() + 1 <= LINE_OUT);
            let event: Event = serde_json::from_str(&line).unwrap();
            event.validate().unwrap();
            revisions.push(event.subscription_revision().unwrap().1);
            match event {
                Event::SnapshotChunk { items, .. }
                | Event::Delta {
                    added: Some(items), ..
                } => {
                    assert!(items.len() <= 256);
                    for item in items {
                        let Item::File(file) = item else {
                            panic!("file payload")
                        };
                        found.push(file.index);
                    }
                }
                _ => {}
            }
        }
        (found, revisions, max)
    });
    let mut stream = connection.stream.try_clone().unwrap();
    for frame in &frames {
        write_frame(&mut stream, frame).unwrap();
    }
    stream.shutdown(Shutdown::Write).unwrap();
    let (found, revisions, max) = read.join().unwrap();
    assert_eq!(found, (0..1100).collect::<Vec<_>>());
    assert_eq!(revisions, (4..=last).collect::<Vec<_>>());
    println!("chunk transport: 1100 items, {} real Unix frames, max={} bytes including newline; contiguous rev=4..={last}",frames.len(),max);
}

#[cfg(test)]
mod defaults_tests {
    use super::*;
    #[test]
    fn summary_default_follows_bar_visibility() {
        let (connection, _peer) = super::super::tests::connection();
        let sink = EventSink(Arc::new(connection));
        sink.0.client.lock().unwrap().client = Some(Client {
            kind: ClientKind::Bar,
            name: ClientName::new("visibility-test").unwrap(),
            version: "test".into(),
        });
        assert_eq!(sink.summary_interval(), 5000, "idle bar default");
        sink.0.client.lock().unwrap().glance = true;
        assert_eq!(sink.summary_interval(), 1000, "visible popup default");
        sink.0.client.lock().unwrap().glance = false;
        sink.0.client.lock().unwrap().interactive = true;
        assert_eq!(sink.summary_interval(), 1000, "interactive client default");
    }
}
