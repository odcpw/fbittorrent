//! Event-bus access stays on the engine runtime. Standard threads never acquire
//! the async ledger mutex. All requested collections share one ledger guard.
use super::*;
use std::{future::Future, pin::Pin, task::Poll};

/// librqbit's Speed::as_bytes for an estimator's MiB/s value.
fn speed_bytes(mbps: f64) -> u64 {
    (mbps * 1024f64 * 1024f64) as u64
}

impl Inner {
    /// Sum of the live handles' estimator rates: the same values Info reports
    /// as each Live row's dl_bps/ul_bps, without building any row.
    pub(super) fn record_rates(&self) {
        let Some(sink) = self.rates.lock().unwrap().clone() else { return };
        let (down, up) = self.session.with_torrents(|torrents| {
            torrents.filter_map(|(_, handle)| handle.live()).fold((0u64, 0u64), |(down, up), live| {
                (down.saturating_add(speed_bytes(live.down_speed_estimator().mbps())),
                 up.saturating_add(speed_bytes(live.up_speed_estimator().mbps())))
            })
        });
        sink(down, up);
    }
}

impl EngineDispatcher {
    pub(crate) fn transfer_rates(&self) -> (u64, u64) {
        let stats = self.inner.session.stats_snapshot();
        (stats.download_speed.as_bytes(), stats.upload_speed.as_bytes())
    }

    pub(crate) fn transfer_totals(&self)->(u64,u64,u64) {
        let counters=self.inner.session.stats_snapshot().counters;
        (counters.fetched_bytes,counters.uploaded_bytes,self.inner.completed_count.load(Ordering::Acquire))
    }

    /// The owner's rate history is fed from the engine tick, not a sample.
    pub(crate) fn attach_rates(&self, sink: RateSink) {
        *self.inner.rates.lock().unwrap() = Some(sink);
    }

    /// Ledger record hashes, exactly the rows Info would list.
    pub(crate) fn record_hashes(&self) -> Result<std::collections::BTreeSet<String>> {
        self.runtime.block_on(async {
            let ledger = OwnedMutexGuard::lock(self.inner.ledger.clone(), self.runtime.cx()).await?;
            Ok(ledger.records.iter().map(|record| record.hash.clone()).collect())
        })
    }

    pub(crate) fn take_completions(&self) -> Vec<serde_json::Value> {
        self.inner.completions.lock().unwrap().drain(..).collect()
    }

    pub(crate) fn bus_sample(
        &self,
        files: &[String],
    ) -> Result<(Vec<view::TorrentView>, HashMap<String, Vec<TorrentFile>>, bool)> {
        self.runtime.block_on(async {
            let ledger =
                OwnedMutexGuard::lock(self.inner.ledger.clone(), self.runtime.cx()).await?;
            // The sampler's presence checks are bounded; see saved::PRESENCE_TTL.
            let rows = self.info_observed(&ledger, false);
            let mut details = HashMap::new();
            for hash in files {
                let Some(record) = ledger.records.iter().find(|r| &r.hash == hash) else {
                    continue;
                };
                let rows = if let Ok(handle) = self.inner.handle(hash) {
                    if let Some(metadata) = handle.metadata.load_full() {
                        let stats = handle.stats();
                        let selected = handle.only_files();
                        metadata
                            .file_infos
                            .iter()
                            .enumerate()
                            .map(|(index, file)| TorrentFile {
                                index: index as i64,
                                name: file.relative_filename.to_string_lossy().into(),
                                size: file.len as i64,
                                progress: if file.len == 0 {
                                    1.0
                                } else {
                                    stats.file_progress.get(index).copied().unwrap_or(0) as f64
                                        / file.len as f64
                                },
                                progress_known: true,
                                priority: i64::from(
                                    selected.as_ref().is_none_or(|ids| ids.contains(&index)),
                                ),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    self.inner.saved_files(record).unwrap_or_default()
                };
                details.insert(hash.clone(), rows);
            }
            let restoring = self.has_pending_restore(&ledger);
            Ok((rows, details, restoring))
        })
    }

    /// One region-owned observer per engine, independent of subscriber count.
    /// Arm the engine notification before discovering handles; arm every handle
    /// before waking the sampler. Repeating that order after each wake covers
    /// admissions, resolver failures and state/error broadcasts before assignment.
    pub(crate) fn bus_watch(&self, wake: Arc<dyn Fn() + Send + Sync>) -> task::JoinHandle<()> {
        let inner = self.inner.clone();
        task::spawn_with(self.runtime.cx(), move |cx| async move {
            loop {
                if cx.checkpoint().is_err() {
                    break;
                }
                let mut admission = std::pin::pin!(inner.bus_changed.notified());
                let mut waits: Vec<Pin<Box<dyn Future<Output = ()> + Send>>> = Vec::new();
                let mut armed = false;
                let changed = std::future::poll_fn(|context| {
                    // SAFETY: abort cancels this task's Cx, not the session.
                    // Check on every poll as well as at the loop boundary so
                    // an aborted, otherwise silent notifier cannot retain a task
                    // or monopolize the sole worker through a ready cancel path.
                    if cx.checkpoint().is_err() {
                        return Poll::Ready(());
                    }
                    let mut changed = admission.as_mut().poll(context).is_ready();
                    if !armed {
                        let handles: Vec<_> = inner
                            .session
                            .with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
                        waits = handles
                            .into_iter()
                            .map(|handle| {
                                Box::pin(async move { handle.state_changed().await })
                                    as Pin<Box<dyn Future<Output = ()> + Send>>
                            })
                            .collect();
                    }
                    for wait in &mut waits {
                        if wait.as_mut().poll(context).is_ready() {
                            changed = true;
                        }
                    }
                    if !armed {
                        armed = true;
                        wake();
                    }
                    if changed {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                });
                rqbit_runtime::select! {
                    _ = inner.session.cancellation_token().cancelled() => break,
                    _ = changed => {},
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fbt-bus-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn observer(app: &AppEngine) -> (task::JoinHandle<()>, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::sync_channel(8);
        let observer = app.dispatcher.bus_watch(Arc::new(move || {
            let _ = sender.try_send(());
        }));
        receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        (observer, receiver)
    }
    #[test]
    fn admitted_handle_wakes_without_rate_tick() {
        let root = directory();
        let app = super::super::tests::test_app(
            &root.join("engine"),
            &root.join("downloads"),
            Ledger::default(),
            None,
        );
        let (watch, receiver) = observer(&app);
        let mut bytes =
            b"d4:infod6:lengthi5e4:name9:hello.txt12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[
            0xaa, 0xf4, 0xc6, 0x1d, 0xdc, 0xc5, 0xe8, 0xa2, 0xda, 0xbe, 0xde, 0x0f, 0x3b, 0x48,
            0x2c, 0xd9, 0xae, 0xa9, 0x43, 0x4d,
        ]);
        bytes.extend_from_slice(b"7:privatei1eee");
        let hash = librqbit::torrent_from_bytes(&bytes)
            .unwrap()
            .info_hash
            .as_string();
        let record = super::super::tests::review_record(&hash, &root.join("downloads"));
        let cx = app.runtime.as_ref().unwrap().cx();
        let began = std::time::Instant::now();
        app.runtime.as_ref().unwrap().block_on(async {
            let AddTorrentResponse::ListOnly(metadata) = app
                .dispatcher
                .inner
                .session
                .add_torrent(
                    cx,
                    AddTorrent::from_bytes(bytes),
                    Some(AddTorrentOptions {
                        list_only: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap()
            else {
                panic!("list-only preflight")
            };
            app.dispatcher
                .inner
                .admit_record(cx, &record, true, metadata)
                .await
                .unwrap();
        });
        receiver
            .recv_timeout(Duration::from_millis(100))
            .expect("late admission must wake the armed sampler without a rate tick");
        println!("late handle admission signaled in {:?}", began.elapsed());
        watch.abort();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn resolver_failure_wakes_without_handle_or_rate_tick() {
        let root = directory();
        let app = super::super::tests::test_app(
            &root.join("engine"),
            &root.join("downloads"),
            Ledger::default(),
            None,
        );
        let (watch, receiver) = observer(&app);
        let mut record =
            super::super::tests::review_record(&"a".repeat(40), &root.join("downloads"));
        record.source.clear();
        app.runtime.as_ref().unwrap().block_on(async {
            app.dispatcher
                .inner
                .spawn_restore(app.runtime.as_ref().unwrap().cx(), record.clone())
                .unwrap();
        });
        receiver
            .recv_timeout(Duration::from_millis(100))
            .expect("metadata failure must wake even when no managed handle exists");
        assert!(matches!(
            app.dispatcher.restore_outcomes()[&record.hash],
            RestoreOutcome::Failed { .. }
        ));
        watch.abort();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn idle_observer_abort_is_acknowledged_while_session_lives() {
        let root = directory();
        let app = super::super::tests::test_app(
            &root.join("engine"),
            &root.join("downloads"),
            Ledger::default(),
            None,
        );
        let (watch, _receiver) = observer(&app);
        watch.abort();
        let runtime = app.runtime.as_ref().unwrap();
        let joined = runtime.block_on(time::timeout(
            runtime.cx(),
            Duration::from_millis(100),
            watch,
        ));
        assert!(
            joined.is_ok(),
            "an idle aborted observer must finish without requiring a session event: {joined:?}"
        );
        assert!(!app
            .dispatcher
            .inner
            .session
            .cancellation_token()
            .is_cancelled());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
