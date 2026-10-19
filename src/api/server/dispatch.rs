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
            if let Call::TorrentsMove{args}=&job.request.call {
                match ops::moving::begin(&self.daemon,args) {
                    Ok(batch)=>{self.begin_confirmation(&job,Reply::TorrentsMove(MutationResult::Pending{reference:MutationRef::new(&reference).unwrap()}),Confirmation::Move(batch),true);}
                    Err(error)=>self.answer(&job,Err(error)),
                }
                return;
            }
            if matches!(&job.request.call, Call::SearchGrab { .. }) {
                if let Err(failure) = ops::search::validate_grab(&self.daemon) { self.answer(&job, Err(failure)); return; }
            }
            if matches!(&job.request.call, Call::SearchInstall { .. } | Call::SearchGrab { .. }) {
                let pending = PendingResult { state: PendingState::Pending, reference: MutationRef::new(&reference).unwrap() };
                let reply = if matches!(&job.request.call, Call::SearchInstall { .. }) {
                    Reply::SearchInstall(InstallReply::Pending(pending))
                } else { Reply::SearchGrab(AddResult { status: MutationResult::Pending { reference: pending.reference },
                    hash: None, name: None, duplicate: None, deferred: None, disk: None }) };
                if self.begin_confirmation(&job, reply, Confirmation::Worker, false) {
                    let result = ops::search::dispatch(&self.daemon, job.connection.number, &job.request.call);
                    if result.as_ref().ok().is_some_and(|reply| ops::search::awaits_metadata(&self.daemon, reply)) {
                        if let Some(active) = self.mutation_active.lock().unwrap().get_mut(&(job.connection.number, job.generation)) {
                            active.confirmation = Some(Confirmation::Metadata(result.unwrap()));
                        }
                        self.mutation_changed.notify_all();
                    } else { self.confirm((job.connection.number, job.generation), result); }
                }
                return;
            }
            if let Call::TorrentsRecheck { args } = &job.request.call {
                match ops::recheck::begin(&self.daemon, args) {
                    Ok(batch) => {
                        self.begin_confirmation(&job, Reply::TorrentsRecheck(RecheckReply::Pending(PendingResult {
                            state: PendingState::Pending, reference: MutationRef::new(&reference).unwrap(),
                        })), Confirmation::Recheck(batch), true);
                    }
                    Err(failure) => self.answer(&job, Err(failure)),
                }
                return;
            }
            if let Call::ShowsCheck {args}=&job.request.call {
                if let Some(id)=args.as_ref().and_then(|a|a.imdb.as_deref()){if let Err(e)=ops::shows::require(&self.daemon,id){self.answer(&job,Err(e));return;}}
                let pending=Reply::ShowsCheck(ShowCheckResult::Pending(PendingResult {state:PendingState::Pending,reference:MutationRef::new(&reference).unwrap()}));
                if self.begin_confirmation(&job,pending,Confirmation::Worker,false){self.confirm((job.connection.number,job.generation),ops::shows::dispatch(&self.daemon,&job.request.call));}
                return;
            }
            if let Call::RssRefresh { args } = &job.request.call {
                if let Some(id)=args.as_ref().and_then(|a|a.feed.as_ref()) {
                    if let Err(failure)=ops::rss::feed(&self.daemon,id) {self.answer(&job,Err(failure));return;}
                }
                let pending=Reply::RssRefresh(MutationResult::Pending {reference:MutationRef::new(&reference).unwrap()});
                if self.begin_confirmation(&job,pending,Confirmation::Worker,false) {
                    self.confirm((job.connection.number,job.generation),ops::rss::dispatch(&self.daemon,&job.request.call));
                }
                return;
            }
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
                        disk: None,
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
                Call::ShowsList {..}|Call::ShowsFollow {..}|Call::ShowsSetQuality {..}|Call::ShowsUnfollow {..}|Call::ShowsCheck {..}|Call::ShowsReleases {..}|Call::ShowsSetRead {..}|Call::ShowsMarkShowRead {..}|Call::ShowsUndoRead {..}|Call::ShowsGrab {..}=>ops::shows::dispatch(&self.daemon,&job.request.call),
                Call::RssFeeds {..}|Call::RssAdd {..}|Call::RssRemove {..}|Call::RssItems {..}|Call::RssSetRead {..}|Call::RssMarkFeedRead {..}|Call::RssGrab {..}|Call::RssRefresh {..}=>ops::rss::dispatch(&self.daemon,&job.request.call),
                Call::AppClipboardDismiss { .. }
                | Call::AppLog { .. }
                | Call::AppStats { .. }
                | Call::AppFreeSpace { .. }
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
                Call::TorrentsTrackers {..} | Call::TorrentsReannounce {..} => ops::trackers::dispatch(&self.daemon,&job.request.call),
                Call::TorrentsForceStart {..} | Call::TorrentsQueueMove {..} => ops::queue::dispatch(&self.daemon,&job.request.call),
                Call::TransferSetLimits { args } => ops::transfer::set_limits(&self.daemon, args),
                Call::TransferSetAlt { args } => ops::transfer::set_alt(&self.daemon, args.on),
                Call::TransferInfo { .. } => ops::transfer::info(&self.daemon),
                call if call.op().starts_with("search.") => {
                    let result = ops::search::dispatch(&self.daemon, job.connection.number, call);
                    if job.connection.read_done.load(Ordering::Acquire) || job.connection.closed.load(Ordering::Acquire) {
                        self.daemon.search.disconnected(job.connection.number);
                    }
                    self.bus.refresh();
                    result
                },
                _ => ops::torrents::dispatch(&self.daemon, &job.request.call),
            }
        };
        if let Ok(Reply::TorrentsAdd(added)) = &result {
            if matches!(&job.request.call, Call::TorrentsAdd { args } if matches!(args.source, AddSource::Magnet(_)) && args.stage!=Some(true))
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
