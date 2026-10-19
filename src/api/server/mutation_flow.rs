use super::*;

// P1-09 integration boundary. P1-08 may drain confirmed_mutations into its
// ordered event history; connection delivery remains owned by these receipts.
impl Server {
    pub(crate) fn confirmed_mutations(&self) -> Vec<Event> {
        self.shared
            .mutation_events
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }
}
impl Shared {
    pub(super) fn deliver_mutation(&self, delivery: mutations::Delivery) {
        match delivery {
            mutations::Delivery::Response(waiter, answer) => {
                let connection = self
                    .clients
                    .lock()
                    .unwrap()
                    .get(&waiter.connection)
                    .cloned();
                if let Some(connection) = connection {
                    connection.complete(waiter.id, waiter.generation, answer);
                }
            }
            mutations::Delivery::Event(number, event) => {
                let connection = self.clients.lock().unwrap().get(&number).cloned();
                if let Some(connection) = connection {
                    connection.send(serde_json::to_value(event).unwrap());
                }
            }
        }
    }
    pub(super) fn detach_mutation_client(&self, connection: u64) {
        self.mutation_ring.lock().unwrap().detach(connection);
        for entry in self.mutation_active.lock().unwrap().values() {
            entry.ticket.detach(connection);
        }
    }
    pub(super) fn prepare_mutation(&self, job: &mut Job) -> bool {
        if !ops::TABLE
            .iter()
            .any(|op| op.name == job.request.call.op() && op.mutating)
        {
            return true;
        }
        let name = job
            .connection
            .client
            .lock()
            .unwrap()
            .client
            .as_ref()
            .unwrap()
            .name
            .to_string();
        let reference = job.request.reference.clone().unwrap_or_else(|| {
            MutationRef::new(format!(
                "srv-{}-{}",
                &self.daemon.instance[..8],
                self.mutation_sequence.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap()
        });
        let claimed = self
            .mutation_ring
            .lock()
            .unwrap()
            .claim(&name, reference, &job.request.call);
        let (ticket, fresh) = match claimed {
            Ok(value) => value,
            Err(failure) => {
                job.connection
                    .complete(job.request.id.get(), job.generation, Err(failure));
                return false;
            }
        };
        ticket.attach(
            mutations::Waiter {
                connection: job.connection.number,
                id: job.request.id.get(),
                generation: job.generation,
            },
            |delivery| self.deliver_mutation(delivery),
        );
        if !fresh {
            return false;
        }
        job.mutation = Some(ticket.clone());
        let deadline = job
            .connection
            .pending
            .lock()
            .unwrap()
            .get(&job.request.id.get())
            .and_then(|entry| (!entry.no_timeout).then_some(entry.deadline));
        {
            let mut active = self.mutation_active.lock().unwrap();
            let controls = active
                .values()
                .filter(|entry| entry.op.starts_with("app."))
                .count();
            let saturated = match &job.request.call {
                // Shutdown remains available even when background work and
                // restart receipts have filled their separate allowances.
                Call::AppShutdown { .. } => false,
                Call::AppRestart { .. } => controls >= CONNECTIONS,
                _ => active.len() - controls >= QUEUE + DISPATCH,
            };
            if saturated {
                ticket.answer(Err(busy("dispatch")), |delivery| {
                    self.deliver_mutation(delivery)
                });
                return false;
            }
            active.insert(
                (job.connection.number, job.generation),
                Background {
                    ticket,
                    op: job.request.call.op().into(),
                    deadline,
                    confirmation: None,
                },
            );
        }
        self.mutation_changed.notify_one();
        if let Err(failure) = mutations::stale(
            &job.request.call,
            job.request.instance.as_ref(),
            &self.daemon.instance,
        ) {
            self.answer(job, Err(failure));
            return false;
        }
        true
    }
    pub(super) fn answer(&self, job: &Job, answer: ApiResult<Reply>) {
        if let Some(ticket) = &job.mutation {
            let active = self
                .mutation_active
                .lock()
                .unwrap()
                .remove(&(job.connection.number, job.generation));
            let answer = if active
                .as_ref()
                .and_then(|entry| entry.deadline)
                .is_some_and(|deadline| deadline <= Instant::now())
            {
                Err(mutation_timeout(job.request.call.op()))
            } else {
                answer
            };
            ticket.answer(answer, |delivery| self.deliver_mutation(delivery));
        } else {
            job.connection
                .complete(job.request.id.get(), job.generation, answer);
        }
    }
    pub(super) fn begin_confirmation(
        &self,
        job: &Job,
        pending: Reply,
        confirmation: Confirmation,
        no_timeout: bool,
    ) -> bool {
        let mut active = self.mutation_active.lock().unwrap();
        let Some(entry) = active.get_mut(&(job.connection.number, job.generation)) else {
            return false;
        };
        if entry
            .deadline
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            return false;
        }
        if no_timeout {
            entry.deadline = None;
        }
        entry
            .ticket
            .answer(Ok(pending), |delivery| self.deliver_mutation(delivery));
        entry.confirmation = Some(confirmation);
        self.mutation_changed.notify_one();
        true
    }
    pub(super) fn confirm(&self, key: (u64, u64), result: ApiResult<Reply>) {
        let Some(task) = self.mutation_active.lock().unwrap().remove(&key) else {
            return;
        };
        let result = if task
            .deadline
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            Err(mutation_timeout(&task.op))
        } else {
            result
        };
        let (outcome, failure, result) = match result {
            Ok(reply) => (
                MutationOutcome::Done,
                None,
                Some(reply.to_value().unwrap().as_object().unwrap().clone()),
            ),
            Err(failure) => (
                if failure.code == ErrorCode::Timeout {
                    MutationOutcome::Timeout
                } else {
                    MutationOutcome::Failed
                },
                Some(failure),
                None,
            ),
        };
        let event = Event::MutationResult {
            seq: self.event_sequence.fetch_add(1, Ordering::Relaxed),
            ts: now(),
            reference: task.ticket.reference.clone(),
            op: task.op,
            outcome,
            error: failure,
            result,
        };
        task.ticket
            .finish(event.clone(), |delivery| self.deliver_mutation(delivery));
        let mut events = self.mutation_events.lock().unwrap();
        if events.len() == 256 {
            events.pop_front();
        }
        events.push_back(event);
    }
    pub(super) fn observe_mutations(&self) {
        let tasks: Vec<_> = self
            .mutation_active
            .lock()
            .unwrap()
            .iter()
            .map(|(key, entry)| (*key, entry.clone()))
            .collect();
        for (key, entry) in tasks {
            if entry
                .deadline
                .is_some_and(|deadline| deadline <= Instant::now())
            {
                if entry.confirmation.is_some() {
                    self.confirm(key, Err(mutation_timeout(&entry.op)));
                } else if let Some(task) = self.mutation_active.lock().unwrap().remove(&key) {
                    task.ticket
                        .answer(Err(mutation_timeout(&task.op)), |delivery| {
                            self.deliver_mutation(delivery)
                        });
                }
                continue;
            }
            let result = match entry.confirmation {
                Some(Confirmation::Metadata(Reply::TorrentsAdd(mut added))) => {
                    let hash = added.hash.as_ref().unwrap().as_str();
                    // Reading the restore receipt never waits on a runtime or on
                    // payload I/O, including while the owner replaces a slot.
                    let Ok(slot) = self.daemon.slot.try_read() else {
                        continue;
                    };
                    let crate::daemon::slot::Slot::Running(app) = &slot.slot else {
                        continue;
                    };
                    match app.dispatcher().restore_outcome(hash) {
                        Some(crate::engine::RestoreOutcome::Restored) => {
                            let dispatcher = app.dispatcher();
                            drop(slot);
                            if let Ok(crate::engine::ops::EngineResult::Detail(detail)) = dispatcher
                                .execute(crate::engine::ops::EngineOp::Get {
                                    hash: hash.to_string(),
                                })
                            {
                                added.name = Some(detail.torrent.name);
                            }
                            added.status = MutationResult::Done;
                            Some(Ok(Reply::TorrentsAdd(added)))
                        }
                        Some(crate::engine::RestoreOutcome::Failed { message, .. }) => {
                            Some(Err(error(
                                ErrorCode::Upstream,
                                &message.chars().take(80).collect::<String>(),
                                None,
                            )))
                        }
                        Some(crate::engine::RestoreOutcome::Stopped) => Some(Err(error(
                            ErrorCode::InvalidState,
                            "metadata resolution stopped",
                            None,
                        ))),
                        _ => None,
                    }
                }
                Some(Confirmation::Engine(epoch)) => {
                    let Ok(slot) = self.daemon.slot.try_read() else {
                        continue;
                    };
                    (slot.engine_epoch > epoch
                        && matches!(slot.slot, crate::daemon::slot::Slot::Running(_)))
                    .then(|| Ok(Reply::TorrentsSetUploadLimit(MutationResult::Done)))
                }
                _ => None,
            };
            if let Some(result) = result {
                self.confirm(key, result);
            }
        }
    }
}
pub(super) fn mutation_timeout(op: &str) -> Error {
    let mut failure = error(
        ErrorCode::Timeout,
        "operation timed out; outcome is unknown",
        None,
    );
    failure.details = Some(serde_json::Map::from_iter([(
        "after_s".into(),
        json!(if op == "torrents.add" { 95 } else { 15 }),
    )]));
    failure
}
