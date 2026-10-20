//! Owned Piece frames and bounded intake into their retained receive allocation.

use super::*;

impl OwnedPieceFrame {
    pub(crate) fn as_piece(&self) -> Piece<ByteBuf<'_>> {
        Piece::from_data_slices(
            self.index,
            self.begin,
            &self.read_buf.buf[self.payload[0].clone()],
            &self.read_buf.buf[self.payload[1].clone()],
        )
    }

    pub(crate) fn into_read_buf(self) -> ReadBuf {
        self.read_buf
    }

    pub(crate) fn into_batch(self) -> OwnedPieceBatch {
        let wire_len = self
            .payload
            .iter()
            .map(|range| range.end - range.start)
            .sum::<usize>()
            + PIECE_PREAMBLE_LEN;
        let mut read_buf = self.read_buf;
        assert_eq!(read_buf.retained_wire_len, 0);
        assert!(wire_len <= read_buf.buf.len() - read_buf.len);
        read_buf.retained_wire_len = wire_len;
        OwnedPieceBatch {
            read_buf,
            frames: [
                Some(PieceFrameMetadata {
                    index: self.index,
                    begin: self.begin,
                    payload: self.payload,
                }),
                None,
                None,
                None,
            ],
            cursor: 0,
            count: 1,
            deferred_read_error: None,
        }
    }
}

impl PieceReceiveOutcome {
    pub(crate) fn completed(read_buf: ReadBuf) -> Self {
        Self {
            read_buf,
            deferred_read_error: None,
        }
    }
}

impl OwnedPieceBatch {
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }
    pub(crate) fn remaining_len(&self) -> usize {
        self.count - self.cursor
    }
    pub(crate) fn is_complete(&self) -> bool {
        self.cursor == self.count
    }

    pub(crate) fn as_piece(&self, index: usize) -> Piece<ByteBuf<'_>> {
        let frame = self.frames[index].as_ref().expect("parsed Piece metadata");
        Piece::from_data_slices(
            frame.index,
            frame.begin,
            &self.read_buf.buf[frame.payload[0].clone()],
            &self.read_buf.buf[frame.payload[1].clone()],
        )
    }

    pub(crate) fn complete_current(&mut self) {
        assert!(self.cursor < self.count);
        self.cursor += 1;
    }

    pub(crate) fn into_receive_outcome(mut self) -> PieceReceiveOutcome {
        assert!(self.is_complete());
        self.read_buf.retained_wire_len = 0;
        PieceReceiveOutcome {
            read_buf: self.read_buf,
            deferred_read_error: self.deferred_read_error,
        }
    }

    fn scan_next_piece(&mut self) -> PieceIntake {
        if self.count == MAX_BATCH_FRAMES || self.deferred_read_error.is_some() {
            return PieceIntake::Fence;
        }
        let parsed = {
            let (first, second) = self.read_buf.frame_slices();
            let mut header = DoubleBufHelper::new(first, second);
            let Ok(prefix) = header.read_u32_be() else {
                return PieceIntake::NeedMore;
            };
            if prefix == 0 {
                return PieceIntake::Fence;
            }
            match header.read_u8() {
                Some(7) => {}
                Some(_) => return PieceIntake::Fence,
                None => return PieceIntake::NeedMore,
            }
            match Message::deserialize(first, second) {
                Ok((Message::Piece(piece), wire_len)) => Ok((
                    piece.index,
                    piece.begin,
                    piece.len(),
                    piece.data().0.len(),
                    wire_len,
                )),
                Ok(_) => return PieceIntake::Fence,
                Err(error) => Err(error),
            }
        };
        match parsed {
            Ok((index, begin, payload_len, first_len, wire_len)) => {
                self.read_buf.advance(wire_len);
                let payload = match self.read_buf.piece_payload_ranges(payload_len, first_len) {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.deferred_read_error = Some(error);
                        return PieceIntake::Fence;
                    }
                };
                self.read_buf.retained_wire_len += wire_len;
                assert!(
                    self.read_buf.retained_wire_len + self.read_buf.len <= self.read_buf.buf.len()
                );
                self.frames[self.count] = Some(PieceFrameMetadata {
                    index,
                    begin,
                    payload,
                });
                self.count += 1;
                #[cfg(feature = "chunk-diagnostics")]
                crate::chunk_diagnostics::received_piece_frame();
                PieceIntake::Added
            }
            Err(MessageDeserializeError::NotEnoughData(missing, ..)) => {
                if self.read_buf.len >= BUFLEN {
                    self.deferred_read_error = Some(ReadBuf::frame_full_error(missing));
                    PieceIntake::Fence
                } else {
                    PieceIntake::NeedMore
                }
            }
            Err(error) => {
                self.deferred_read_error = Some(Error::Deserialize(error));
                PieceIntake::Fence
            }
        }
    }

    fn poll_intake(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
        task_cx: &mut std::task::Context<'_>,
        ready_reads: &mut usize,
        deadline: Option<Time>,
    ) -> Poll<anyhow::Result<PieceIntakeEnd>> {
        loop {
            cx.checkpoint()?;
            if cx.reusable_region_owner_key().as_ref() != Some(expected_key) {
                return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
            }
            match self.scan_next_piece() {
                PieceIntake::Added => continue,
                PieceIntake::Fence => return Poll::Ready(Ok(PieceIntakeEnd::Stopped)),
                PieceIntake::NeedMore => {}
            }
            if *ready_reads == MAX_READY_READ_ATTEMPTS || self.read_buf.is_full() {
                return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
            }
            if deadline.is_some_and(|deadline| cx.now() >= deadline) {
                return Poll::Ready(Ok(PieceIntakeEnd::Deadline));
            }
            match Pin::new(&mut **input)
                .poll_read_vectored(task_cx, &mut self.read_buf.unfilled_ioslices())
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(0)) => {
                    self.deferred_read_error = Some(Error::PeerDisconnected);
                    return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
                }
                Poll::Ready(Ok(bytes)) => {
                    *ready_reads += 1;
                    self.read_buf.len += bytes;
                }
                Poll::Ready(Err(error)) => {
                    self.deferred_read_error = Some(Error::Read(error));
                    return Poll::Ready(Ok(PieceIntakeEnd::Stopped));
                }
            }
        }
    }

    /// Poll only already-ready socket data. A pending socket future never
    /// delays the prepared frame or creates a coalescing timer.
    pub(crate) async fn intake_ready(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
    ) -> anyhow::Result<()> {
        let mut ready_reads = 0;
        match std::future::poll_fn(|task_cx| {
            Poll::Ready(self.poll_intake(cx, expected_key, input, task_cx, &mut ready_reads, None))
        })
        .await
        {
            Poll::Pending => Ok(()),
            Poll::Ready(result) => result.map(|_| ()),
        }
    }

    /// After a single-frame attempt, use a bounded number of passive batches
    /// before probing again. A passive multi-frame batch enables waiting for
    /// the next batch. Passive attempts add no entry-clock read or timeout.
    /// Timed attempts use one unchanged absolute coalescing window; already
    /// ready work remains immediate, and scheduler delay can overshoot it.
    pub(crate) async fn intake_coalescing(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
    ) -> anyhow::Result<()> {
        if self.read_buf.coalescing_passive_batches_left != 0 {
            self.read_buf.coalescing_passive_batches_left -= 1;
            self.intake_ready(cx, expected_key, input).await?;
            if self.count >= 2 {
                self.read_buf.coalescing_passive_batches_left = 0;
            }
            return Ok(());
        }
        let deadline = cx.now() + PIECE_COALESCING_WINDOW;
        self.intake_until(cx, expected_key, input, deadline).await?;
        if self.count == 1 {
            self.read_buf.coalescing_passive_batches_left = SINGLE_PIECE_PASSIVE_BATCHES;
        }
        Ok(())
    }

    pub(super) async fn intake_until(
        &mut self,
        cx: &Cx,
        expected_key: &RegionOwnerKey,
        input: &mut BoxAsyncReadVectored,
        deadline: Time,
    ) -> anyhow::Result<()> {
        #[cfg(feature = "chunk-diagnostics")]
        let mut observation = crate::chunk_diagnostics::CoalescingObservation::new(
            Duration::from_nanos(deadline.duration_since(cx.now())),
        );
        let mut ready_reads = 0;
        let initial = std::future::poll_fn(|task_cx| {
            Poll::Ready(self.poll_intake(
                cx,
                expected_key,
                input,
                task_cx,
                &mut ready_reads,
                Some(deadline),
            ))
        })
        .await;
        if let Poll::Ready(result) = initial {
            #[cfg(feature = "chunk-diagnostics")]
            if matches!(&result, Ok(PieceIntakeEnd::Deadline)) {
                observation.deadline_stop();
            }
            return result.map(|_| ());
        }
        let remaining = Duration::from_nanos(deadline.duration_since(cx.now()));
        if remaining.is_zero() {
            cx.checkpoint()?;
            #[cfg(feature = "chunk-diagnostics")]
            observation.deadline_stop();
            return Ok(());
        }
        // One timeout wraps the entire remaining intake, rather than one
        // timeout per read or per Piece. The poll helper also checks the same
        // absolute deadline before reading after a timer/scheduler wakeup.
        let mut timeout = std::pin::pin!(rqbit_runtime::time::timeout_at(
            deadline,
            std::future::poll_fn(|task_cx| {
                self.poll_intake(
                    cx,
                    expected_key,
                    input,
                    task_cx,
                    &mut ready_reads,
                    Some(deadline),
                )
            }),
        ));
        let waited = std::future::poll_fn(|task_cx| {
            let progress = timeout.as_mut().poll(task_cx);
            #[cfg(feature = "chunk-diagnostics")]
            if progress.is_pending() {
                observation.pending();
            }
            progress
        })
        .await;
        // Timeout/sleep cancellation must remain an original-caller error;
        // explicit cancellation never turns into a successful partial batch.
        cx.checkpoint()?;
        match waited {
            Ok(result) => {
                #[cfg(feature = "chunk-diagnostics")]
                if matches!(&result, Ok(PieceIntakeEnd::Deadline)) {
                    observation.deadline_stop();
                }
                result.map(|_| ())
            }
            Err(_) => {
                #[cfg(feature = "chunk-diagnostics")]
                observation.deadline_stop();
                Ok(())
            }
        }
    }
}
