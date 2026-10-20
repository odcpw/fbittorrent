use super::{MAX_MSG_LEN, Message, WriterRequest, request_batch};
use peer_binary_protocol::Request;
use rqbit_runtime::sync::mpsc;

fn request(index: u32) -> Request {
    Request::new(index, index * 16384, 16384)
}

fn decode(mut bytes: &[u8]) -> Vec<(u32, u32, u32)> {
    let mut requests = Vec::new();
    while !bytes.is_empty() {
        let (message, used) = Message::deserialize(bytes, &[]).unwrap();
        match message {
            Message::Request(r) => requests.push((r.index, r.begin, r.length)),
            other => panic!("unexpected message in request batch: {other:?}"),
        }
        bytes = &bytes[used..];
    }
    requests
}

#[test]
fn request_batch_preserves_wire_order_and_fields_across_bounded_writes() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    // First, last, then sequential pieces: batching must not sort these.
    let order: Vec<u32> = [0, 4095].into_iter().chain(1..70).collect();
    for &index in &order[1..] {
        tx.send(WriterRequest::Message(Message::Request(request(index))))
            .unwrap();
    }
    drop(tx);
    let mut first = request(order[0]);
    let mut buffer = [0; MAX_MSG_LEN];
    let mut pending = None;
    let mut actual = Vec::new();
    let mut writes = 0;
    loop {
        let len = request_batch(first, &mut rx, &mut pending, &mut buffer).unwrap();
        let decoded = decode(&buffer[..len]);
        assert!(
            !decoded.is_empty() && decoded.len() < order.len(),
            "bounded work before cancellation check"
        );
        actual.extend(decoded);
        writes += 1;
        assert!(pending.is_none());
        match rx.try_recv() {
            Ok(WriterRequest::Message(Message::Request(next))) => first = next,
            Err(_) => break,
            other => panic!("unexpected queued message: {other:?}"),
        }
    }
    assert!(
        writes > 1 && writes < order.len(),
        "coalesce without draining unbounded work"
    );
    assert_eq!(
        actual,
        order
            .iter()
            .map(|&i| (i, i * 16384, 16384))
            .collect::<Vec<_>>()
    );
}

#[test]
fn request_batch_stops_before_control_or_upload_work() {
    for (index, barrier) in [
        WriterRequest::UploadPolicyChanged,
        WriterRequest::Message(Message::Cancel(request(3))),
        WriterRequest::Message(Message::Unchoke),
        WriterRequest::Disconnect(Ok(())),
    ]
    .into_iter()
    .enumerate()
    {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(WriterRequest::Message(Message::Request(request(1))))
            .unwrap();
        tx.send(barrier).unwrap();
        tx.send(WriterRequest::Message(Message::Request(request(2))))
            .unwrap();
        let mut buffer = [0; MAX_MSG_LEN];
        let mut pending = None;
        let len = request_batch(request(0), &mut rx, &mut pending, &mut buffer).unwrap();
        assert_eq!(
            decode(&buffer[..len]),
            vec![(0, 0, 16384), (1, 16384, 16384)]
        );
        let pending = pending.unwrap();
        assert!(
            match (index, pending) {
                (0, WriterRequest::UploadPolicyChanged) => true,
                (1, WriterRequest::Message(Message::Cancel(r))) =>
                    r.index == 3 && r.begin == 3 * 16384 && r.length == 16384,
                (2, WriterRequest::Message(Message::Unchoke)) => true,
                (3, WriterRequest::Disconnect(Ok(()))) => true,
                _ => false,
            },
            "control message stays intact and next"
        );
        assert!(
            matches!(rx.try_recv(), Ok(WriterRequest::Message(Message::Request(r))) if r.index == 2)
        );
    }
}

#[test]
fn request_batch_never_waits_for_another_request() {
    let (_tx, mut rx) = mpsc::unbounded_channel();
    let mut buffer = [0; MAX_MSG_LEN];
    let mut pending = None;
    let len = request_batch(request(7), &mut rx, &mut pending, &mut buffer).unwrap();
    assert_eq!(decode(&buffer[..len]), vec![(7, 7 * 16384, 16384)]);
    assert!(pending.is_none());
}
