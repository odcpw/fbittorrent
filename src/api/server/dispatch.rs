//! Bounded worker admission and typed operation dispatch.

use super::*;

impl Pool {
    pub(super) fn submit(&self, job: Job) -> Result<(), Job> {
        let mut work = self.work.lock().unwrap();
        if work.closed || work.jobs.len() >= QUEUE {
            return Err(job);
        }
        work.jobs.push_back(job);
        self.changed.notify_one();
        Ok(())
    }
    pub(super) fn next(&self) -> Option<Job> {
        let mut work = self.work.lock().unwrap();
        while work.jobs.is_empty() && !work.closed {
            work = self.changed.wait(work).unwrap();
        }
        work.jobs.pop_front()
    }
    pub(super) fn close(&self) {
        self.work.lock().unwrap().closed = true;
        self.changed.notify_all();
    }
}

impl Shared {
    pub(super) fn url_outcome(
        &self,
        connection: &Connection,
        reference: MutationRef,
        result: ApiResult<Reply>,
    ) {
        let (outcome, error, result) = match result {
            Ok(reply) => (
                MutationOutcome::Done,
                None,
                Some(reply.to_value().unwrap().as_object().unwrap().clone()),
            ),
            Err(error) => (
                if error.code == ErrorCode::Timeout {
                    MutationOutcome::Timeout
                } else {
                    MutationOutcome::Failed
                },
                Some(error),
                None,
            ),
        };
        connection.send(
            serde_json::to_value(Event::MutationResult {
                seq: self.event_sequence.fetch_add(1, Ordering::Relaxed),
                ts: now(),
                reference,
                op: "torrents.add".into(),
                outcome,
                error,
                result,
            })
            .unwrap(),
        );
    }
    pub(super) fn expire_background(&self, connection: &Connection) {
        let mut background = connection.background.lock().unwrap();
        let expired: Vec<_> = background
            .iter()
            .filter(|(_, task)| task.deadline <= Instant::now())
            .map(|(generation, _)| *generation)
            .collect();
        for generation in expired {
            let task = background.remove(&generation).unwrap();
            self.url_outcome(connection, task.reference, Err(add_timeout()));
        }
    }
    pub(super) fn dispatch(&self, job: Job) {
        if !job
            .connection
            .pending
            .lock()
            .unwrap()
            .get(&job.request.id.get())
            .is_some_and(|p| p.generation == job.generation)
        {
            return;
        }
        let reference = job
            .request
            .reference
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("api-{}-{}", job.connection.number, job.generation));
        if !self.daemon.is_stopping() {
            if let Call::TorrentsAdd { args } = &job.request.call {
                if matches!(args.source, AddSource::Url(_)) {
                    if let Err(failure) = ops::torrents::validate_url_add(args) {
                        job.connection
                            .complete(job.request.id.get(), job.generation, Err(failure));
                        return;
                    }
                    let Some(deadline) = job
                        .connection
                        .pending
                        .lock()
                        .unwrap()
                        .get(&job.request.id.get())
                        .filter(|entry| entry.generation == job.generation)
                        .map(|entry| entry.deadline)
                    else {
                        return;
                    };
                    let reference = MutationRef::new(reference).unwrap();
                    if !job.connection.complete(
                        job.request.id.get(),
                        job.generation,
                        Ok(Reply::TorrentsAdd(AddResult {
                            status: MutationResult::Pending {
                                reference: reference.clone(),
                            },
                            hash: None,
                            name: None,
                            duplicate: None,
                            deferred: None,
                        })),
                    ) {
                        return;
                    }
                    job.connection.background.lock().unwrap().insert(
                        job.generation,
                        Background {
                            deadline,
                            reference,
                        },
                    );
                    let result = ops::torrents::dispatch(&self.daemon, &job.request.call);
                    if let Some(task) = job
                        .connection
                        .background
                        .lock()
                        .unwrap()
                        .remove(&job.generation)
                    {
                        let result = if task.deadline <= Instant::now() {
                            Err(add_timeout())
                        } else {
                            result
                        };
                        self.url_outcome(&job.connection, task.reference, result);
                    }
                    return;
                }
            }
        }
        let result = if self.daemon.is_stopping() {
            Err(stopping())
        } else {
            match &job.request.call {
                Call::AppInfo { .. }
                | Call::AppNetwork { .. }
                | Call::AppShutdown { .. }
                | Call::AppRestart { .. } => ops::app::dispatch(
                    &self.daemon,
                    &job.request.call,
                    &reference,
                    serde_json::to_value(&*self.counts.lock().unwrap()).unwrap(),
                    self.began.elapsed().as_secs(),
                ),
                Call::TransferInfo { .. } => ops::transfer::info(&self.daemon),
                _ => ops::torrents::dispatch(&self.daemon, &job.request.call),
            }
        };
        job.connection
            .complete(job.request.id.get(), job.generation, result);
    }
}
