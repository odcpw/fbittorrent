#[test]
fn piece_coalescing_first_key_fence_does_not_poll_input() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        let mut batch = coalescing_batch(&cx, &[]).await;
        let (mut src, polls) = probe(ProbeEnd::Pending);
        cx.apply_child_capability_budget(
            rqbit_runtime::types::CapabilityBudget::UNSPECIFIED.with_io_bytes(65_536),
            rqbit_runtime::types::CapabilityBudgetRequirements::NONE,
        )
        .unwrap();
        assert!(cx.checkpoint().is_ok());
        assert!(cx.reusable_region_owner_key().is_none());
        batch.intake_coalescing(&cx, &key, &mut src).await.unwrap();
        assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
        assert!(batch.deferred_read_error.is_none());
        assert_eq!(
            [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
            b"first"
        );
    });
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn piece_coalescing_ready_control_parse_eof_and_fault_do_not_wait() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        let mut batch = coalescing_batch(&cx, &[0, 0, 0, 5, 4, 0, 0, 0, 99]).await;
        let (mut src, polls) = probe(ProbeEnd::Pending);
        // A single manual poll must finish: the control fence cannot park.
        {
            let mut receiving = std::pin::pin!(batch.intake_coalescing(&cx, &key, &mut src));
            assert!(matches!(
                std::future::poll_fn(|task| Poll::Ready(receiving.as_mut().poll(task))).await,
                Poll::Ready(Ok(()))
            ));
        }
        assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(batch.remaining_len(), 1);
        batch.complete_current();
        let outcome = batch.into_receive_outcome();
        assert!(outcome.deferred_read_error.is_none());
        let mut returned = outcome.read_buf;
        assert!(matches!(
            returned
                .read_message(&cx, &mut src, Duration::from_secs(1))
                .await
                .unwrap(),
            Message::Have(99)
        ));

        for (end, expected) in [(ProbeEnd::Eof, "eof"), (ProbeEnd::Fault, "fault")] {
            let mut batch = coalescing_batch(&cx, &[]).await;
            let (mut src, polls) = probe(end);
            {
                let mut receiving =
                    std::pin::pin!(batch.intake_coalescing(&cx, &key, &mut src));
                assert!(matches!(
                    std::future::poll_fn(|task| Poll::Ready(receiving.as_mut().poll(task)))
                        .await,
                    Poll::Ready(Ok(()))
                ));
            }
            assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 1);
            assert_eq!(batch.remaining_len(), 1);
            match (expected, &batch.deferred_read_error) {
                ("eof", Some(crate::Error::PeerDisconnected)) => {}
                ("fault", Some(crate::Error::Read(error))) => {
                    assert_eq!(error.to_string(), "coalescing input fault")
                }
                _ => panic!("expected deferred {expected}"),
            }
        }

        let mut batch = coalescing_batch(&cx, &piece_wire(&[])).await;
        let (mut src, polls) = probe(ProbeEnd::Pending);
        batch.intake_coalescing(&cx, &key, &mut src).await.unwrap();
        assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(batch.remaining_len(), 1);
        assert!(matches!(
            batch.deferred_read_error,
            Some(crate::Error::Deserialize(
                peer_binary_protocol::MessageDeserializeError::IncorrectMsgLen { .. }
            ))
        ));
    });
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn piece_coalescing_timeout_preserves_prefix_and_absolute_deadline() {
    let runtime = rqbit_runtime::RuntimeBuilder::current_thread()
        .blocking_threads(1, 2)
        .build()
        .unwrap();
    let cx = runtime.request_cx_with_budget(rqbit_runtime::Budget::INFINITE);
    let key = cx.reusable_region_owner_key().unwrap();
    runtime.block_on(async {
        assert_eq!(super::PIECE_COALESCING_WINDOW, Duration::from_micros(1_500));
        let mut batch = coalescing_batch(&cx, &[]).await;
        let (mut src, polls) = probe(ProbeEnd::Pending);
        let deadline = cx.now() + super::PIECE_COALESCING_WINDOW;
        batch
            .intake_until(&cx, &key, &mut src, deadline)
            .await
            .unwrap();
        assert!(
            cx.now() >= deadline,
            "a healthy Pending input reaches its original cutoff"
        );
        assert!(polls.load(std::sync::atomic::Ordering::Relaxed) >= 1);
        assert_eq!((batch.cursor(), batch.remaining_len()), (0, 1));
        assert!(batch.deferred_read_error.is_none());
        assert_eq!(
            [batch.as_piece(0).data().0, batch.as_piece(0).data().1].concat(),
            b"first"
        );

        // No synthetic clock: use an already-expired native cutoff while
        // a second complete Piece is buffered. Parsing it cannot reset
        // that cutoff and permit a further socket read.
        let mut batch = coalescing_batch(&cx, &piece_wire(b"second")).await;
        let (mut src, polls) = probe(ProbeEnd::Pending);
        let cutoff = rqbit_runtime::types::Time::ZERO;
        batch
            .intake_until(&cx, &key, &mut src, cutoff)
            .await
            .unwrap();
        assert_eq!(polls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(batch.remaining_len(), 2);
        assert_eq!(
            [batch.as_piece(1).data().0, batch.as_piece(1).data().1].concat(),
            b"second"
        );
        assert_eq!(cutoff, rqbit_runtime::types::Time::ZERO);
    });
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
}
