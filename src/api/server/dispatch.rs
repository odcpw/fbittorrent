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
    pub(super) fn dispatch(&self, job: Job) {
        if job.mutation.is_none()
            && !job
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
            .mutation
            .as_ref()
            .map(|ticket| ticket.reference.to_string())
            .unwrap_or_default();
        if !self.daemon.is_stopping() {
            if let Call::TorrentsAdd { args } = &job.request.call {
                if matches!(args.source, AddSource::Url(_)) {
                    if let Err(failure) = ops::torrents::validate_url_add(args) {
                        self.answer(&job, Err(failure));
                        return;
                    }
                    let parked = matches!(&self.daemon.slot.read().unwrap().slot, crate::daemon::slot::Slot::Parked(_));
                    if parked {
                        self.answer(&job, Err(ops::engine_error(anyhow::anyhow!(
                            "connect ProtonVPN before adding a URL; engine_unavailable state=parked"
                        ))));
                        return;
                    }
                    let pending = Reply::TorrentsAdd(AddResult {
                        status: MutationResult::Pending {
                            reference: MutationRef::new(&reference).unwrap(),
                        },
                        hash: None,
                        name: None,
                        duplicate: None,
                        deferred: None,
                    });
                    if self.begin_confirmation(&job, pending, Confirmation::Worker, false) {
                        let result = ops::torrents::dispatch(&self.daemon, &job.request.call);
                        self.confirm((job.connection.number, job.generation), result);
                    }
                    return;
                }
            }
            if matches!(&job.request.call, Call::TorrentsRemove { args } if args.delete_files) {
                let pending = Reply::TorrentsRemove(RemoveResult {
                    status: MutationResult::Pending {
                        reference: MutationRef::new(&reference).unwrap(),
                    },
                    undo_until: None,
                });
                if self.begin_confirmation(&job, pending, Confirmation::Worker, true) {
                    let result = ops::torrents::dispatch(&self.daemon, &job.request.call);
                    self.confirm((job.connection.number, job.generation), result);
                }
                return;
            }
        }
        let epoch = self.daemon.info()["engine_epoch"].as_u64().unwrap_or(0);
        let result = if self.daemon.is_stopping() {
            Err(stopping())
        } else {
            match &job.request.call {
                Call::AppLog { .. }
                | Call::AppStats { .. }
                | Call::AppSettingsGet { .. }
                | Call::AppSettingsSet { .. }
                | Call::AppInfo { .. }
                | Call::AppNetwork { .. }
                | Call::AppShutdown { .. }
                | Call::AppRestart { .. } => ops::app::dispatch(
                    &self.daemon,
                    &job.request.call,
                    &reference,
                    serde_json::to_value(&*self.connections_since_start.lock().unwrap()).unwrap(),
                    serde_json::to_value(&*self.counts.lock().unwrap()).unwrap(),
                    self.began.elapsed().as_secs(),
                    self.bus.counts(),
                ),
                Call::Subscribe { args } => self.bus.subscribe(EventSink(job.connection.clone()), args),
                Call::Unsubscribe { args } => Ok(self.bus.unsubscribe(job.connection.number, args.sub)),
                Call::TransferSetLimits { args } => ops::transfer::set_limits(&self.daemon, args),
                Call::TransferSetAlt { args } => ops::transfer::set_alt(&self.daemon, args.on),
                Call::TransferInfo { .. } => ops::transfer::info(&self.daemon),
                _ => ops::torrents::dispatch(&self.daemon, &job.request.call),
            }
        };
        if let Ok(Reply::TorrentsAdd(added)) = &result {
            if matches!(&job.request.call, Call::TorrentsAdd { args } if matches!(args.source, AddSource::Magnet(_)))
                && added.duplicate != Some(true)
                && added.deferred != Some(true)
            {
                let mut pending = added.clone();
                pending.status = MutationResult::Pending {
                    reference: MutationRef::new(&reference).unwrap(),
                };
                self.begin_confirmation(
                    &job,
                    Reply::TorrentsAdd(pending),
                    Confirmation::Metadata(result.clone().unwrap()),
                    false,
                );
                return;
            }
        }
        if let Ok(Reply::AppRestart(_)) = &result {
            if matches!(&job.request.call, Call::AppRestart { args } if args.scope == RestartScope::Engine)
            {
                self.begin_confirmation(&job, result.unwrap(), Confirmation::Engine(epoch), false);
                return;
            }
        }
        if let Ok(Reply::Subscribe(subscription)) = &result {
            let sub = subscription.sub;
            if job.connection.complete(job.request.id.get(), job.generation, result) {
                self.bus.activate(job.connection.number, sub);
            } else {
                self.bus.unsubscribe(job.connection.number, sub);
            }
        } else {
            self.answer(&job, result);
        }
    }
}
