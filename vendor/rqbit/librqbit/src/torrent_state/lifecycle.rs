//! Start, pause, cancellation, completion waits and background torrent tasks.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use futures::{FutureExt, StreamExt, future::BoxFuture};
use librqbit_core::spawn_utils::spawn_with_cancel;
use rqbit_runtime::{CancellationToken, ChildRegionSpec, Cx, time::timeout};
use tracing::{debug, debug_span, trace, warn};

use crate::{Session, type_aliases::PeerStream};
use super::{
    ManagedTorrent, ManagedTorrentLocked, ManagedTorrentState,
    TorrentStateInitializing, TorrentStateLive,
};

impl ManagedTorrent {
    pub(super) fn stop_with_error(&self, error: anyhow::Error) {
        let mut g = self.locked.write();

        match g.state.take() {
            ManagedTorrentState::Live(live) => {
                if let Err(err) = live.pause() {
                    warn!(target: super::LOG_TARGET,
                        id = self.shared.id,
                        info_hash = ?self.shared.info_hash,
                        "error pausing live torrent during fatal error handling: {err:#}",
                    );
                }
            }
            ManagedTorrentState::Error(e) => {
                warn!(target: super::LOG_TARGET,
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent already was in error state when trying to stop it. Previous error was: {e:#}",
                );
            }
            ManagedTorrentState::None => {
                warn!(target: super::LOG_TARGET,
                    id = self.shared.id,
                    info_hash = ?self.shared.info_hash,
                    "bug: torrent encountered in None state during fatal error handling"
                )
            }
            _ => {}
        };

        self.state_change_notify.notify_waiters();

        g.state = ManagedTorrentState::Error(error)
    }

    /// peer_rx: the peer stream. If start_paused=false, must be set.
    /// start_paused: if set, the torrent will initialize (check file integrity), but will not start
    pub(crate) async fn start(
        self: &Arc<Self>,
        cx: &Cx,
        _peer_rx: Option<PeerStream>,
        start_paused: bool,
    ) -> anyhow::Result<()> {
        cx.checkpoint()?;
        if self.live().is_some() { bail!("torrent is already live"); }
        self.close_run().await?;
        let region = self.shared.cx.open_child_region(ChildRegionSpec::inherit()).await?;
        let run_cx = region.cx().clone();
        *self.run_region.lock() = Some(region);
        fn _start<'a>(
            cx: &Cx,
            t: &'a Arc<ManagedTorrent>,
            peer_rx: Option<PeerStream>,
            start_paused: bool,
            session: Arc<Session>,
            g: Option<parking_lot::RwLockWriteGuard<'a, ManagedTorrentLocked>>,
            token: CancellationToken,
        ) -> anyhow::Result<()> {
            let mut g = g.unwrap_or_else(|| t.locked.write());

            match &g.state {
                ManagedTorrentState::Live(_) => {
                    bail!("torrent is already live");
                }
                ManagedTorrentState::Initializing(init) => {
                    let init = init.clone();
                    init.clear_pause_request();
                    if !init.try_start_check() {
                        return Ok(());
                    }
                    struct CheckGuard(Arc<TorrentStateInitializing>);
                    impl Drop for CheckGuard { fn drop(&mut self) { self.0.finish_check(); } }
                    // Capture before admission: rejection or cancellation before
                    // the child's first poll must also release the check flag.
                    let check_guard = CheckGuard(init.clone());

                    let t = t.clone();
                    let span = t.shared().span.clone();
                    let token = token.clone();

                    spawn_with_cancel(
                        cx,
                        debug_span!(target: super::LOG_TARGET, parent: span.clone(), "initialize_and_start"),
                        "initialize_and_start",
                        token.clone(),
                        move |cx| async move {
                            let _check_guard = check_guard;
                            let concurrent_init_semaphore =
                                session.concurrent_initialize_semaphore.clone();
                            let _permit = concurrent_init_semaphore
                                .acquire(&cx, 1)
                                .await
                                .context("bug: concurrent init semaphore was closed")?;

                            let check_result = init.clone().check(&cx).await;
                            init.finish_check();

                            match check_result {
                                Ok(paused) => {
                                    let mut g = t.locked.write();
                                    if let ManagedTorrentState::Initializing(_) = &g.state {
                                    } else {
                                        debug!(target: super::LOG_TARGET,
                                            "no need to start torrent anymore, as it switched state from initializing"
                                        );
                                        return Ok(());
                                    }

                                    g.state = ManagedTorrentState::Paused(paused);
                                    t.state_change_notify.notify_waiters();
                                    _start(&cx, &t, peer_rx, start_paused, session, Some(g), token)
                                }
                                Err(err) => {
                                    if init.is_pause_requested() {
                                        debug!(target: super::LOG_TARGET, "initial check paused");
                                        t.state_change_notify.notify_waiters();
                                        return Ok(());
                                    }

                                    let result = anyhow::anyhow!("{:?}", err);
                                    t.locked.write().state = ManagedTorrentState::Error(err);
                                    t.state_change_notify.notify_waiters();
                                    Err(result)
                                }
                            }
                        },
                    );
                    Ok(())
                }
                ManagedTorrentState::Paused(_) => {
                    if start_paused {
                        return Ok(());
                    }
                    let paused = g.state.take().assert_paused();
                    let (tx, rx) = rqbit_runtime::channel::oneshot::channel();
                    let live = TorrentStateLive::new(cx, paused, tx, token.clone())?;
                    g.state = ManagedTorrentState::Live(live.clone());
                    t.state_change_notify.notify_waiters();

                    spawn_fatal_errors_receiver(cx, t, rx, token);
                    if let Some(peer_rx) = peer_rx {
                        spawn_peer_adder(&live, peer_rx);
                    }
                    Ok(())
                }
                ManagedTorrentState::Error(_) => {
                    let metadata = t.metadata.load_full().expect("TODO");
                    let initializing = Arc::new(TorrentStateInitializing::new(
                        t.shared.clone(),
                        metadata.clone(),
                        g.only_files.clone(),
                        t.shared
                            .storage_factory
                            .create_and_init(t.shared(), &metadata)?,
                        true,
                    ));
                    g.state = ManagedTorrentState::Initializing(initializing.clone());
                    t.state_change_notify.notify_waiters();

                    // Recurse.
                    _start(cx, t, peer_rx, start_paused, session, Some(g), token)
                }
                ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
            }
        }

        let session = self
            .shared
            .session
            .upgrade()
            .context("session is dead, cannot start torrent")?;
        let peer_rx = if start_paused { None } else {
            session.make_peer_rx_managed_torrent(&run_cx, self, true)
        };
        let mut g = self.locked.write();
        g.paused = start_paused;
        let cancellation_token = session.cancellation_token().child_token();

        _start(
            &run_cx,
            self,
            peer_rx,
            start_paused,
            session,
            Some(g),
            cancellation_token,
        )
    }

    pub(crate) async fn close_run(&self) -> anyhow::Result<()> {
        let region = self.run_region.lock().take();
        if let Some(region) = region {
            region.cancel(rqbit_runtime::CancelReason::user("torrent stopped"))?;
            region.close().await?;
        }
        let initializing = self.with_state(|s| match s {
            ManagedTorrentState::Initializing(init) => Some(init.clone()),
            _ => None,
        });
        if let Some(init) = initializing { init.wait_for_blocking_work(&self.shared.cx).await?; }
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        self.locked.read().paused
    }

    /// Pause the torrent if it's live.
    pub(crate) fn pause(&self) -> anyhow::Result<()> {
        let mut g = self.locked.write();
        match &g.state {
            ManagedTorrentState::Live(live) => {
                let paused = live.pause()?;
                g.state = ManagedTorrentState::Paused(paused);
                g.paused = true;
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Initializing(init) => {
                let init = init.clone();
                g.paused = true;
                init.request_pause();
                self.state_change_notify.notify_waiters();
                Ok(())
            }
            ManagedTorrentState::Paused(_) => {
                bail!("torrent is already paused");
            }
            ManagedTorrentState::Error(_) => {
                bail!("can't pause torrent in error state")
            }
            ManagedTorrentState::None => bail!("bug: torrent is in empty state"),
        }
    }

    #[inline(never)]
    pub fn wait_until_initialized<'a>(&'a self, cx: &'a Cx) -> BoxFuture<'a, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            loop {
                cx.checkpoint()?;
                let done = self.with_state(|s| match s {
                    ManagedTorrentState::Initializing(_) => Ok(false),
                    ManagedTorrentState::Error(e) => bail!("{:?}", e),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                    _ => Ok(true),
                })?;
                if done {
                    return Ok(());
                }
                let _ = timeout(
                    cx,
                    Duration::from_millis(100),
                    self.state_change_notify.notified(),
                )
                .await;
            }
        }
        .boxed()
    }

    #[inline(never)]
    pub fn wait_until_completed<'a>(&'a self, cx: &'a Cx) -> BoxFuture<'a, anyhow::Result<()>> {
        async move {
            // TODO: rewrite, this polling is horrible
            let live = loop {
                cx.checkpoint()?;
                let live = self.with_state(|s| match s {
                    ManagedTorrentState::Initializing(_) | ManagedTorrentState::Paused(_) => {
                        Ok(None)
                    }
                    ManagedTorrentState::Live(l) => Ok(Some(l.clone())),
                    ManagedTorrentState::Error(e) => bail!("{:?}", e),
                    ManagedTorrentState::None => bail!("bug: torrent state is None"),
                })?;
                if let Some(live) = live {
                    break live;
                }
                let _ = timeout(cx, Duration::from_secs(1), self.state_change_notify.notified()).await;
            };

            live.wait_until_completed(cx).await?;
            Ok(())
        }
        .boxed()
    }

}

fn spawn_fatal_errors_receiver(
    cx: &Cx,
    state: &Arc<ManagedTorrent>,
    mut rx: rqbit_runtime::channel::oneshot::Receiver<anyhow::Error>,
    token: CancellationToken,
) {
    let span = state.shared.span.clone();
    let id = state.shared.id;
    let info_hash = state.shared.info_hash;
    let state = Arc::downgrade(state);
    spawn_with_cancel(
        cx,
        debug_span!(target: super::LOG_TARGET, parent: span, "fatal_errors_receiver"),
        "fatal_errors_receiver",
        token,
        move |cx| async move {
            let e = match rx.recv(&cx).await {
                Ok(e) => e,
                Err(_) => return Ok(()),
            };
            if let Some(state) = state.upgrade() {
                state.stop_with_error(e);
            } else {
                warn!(target: super::LOG_TARGET,
                    ?id,
                    ?info_hash,
                    "tried to stop the torrent with error, but couldn't upgrade the arc"
                );
            }
            Ok::<(), &'static str>(())
        },
    );
}

fn spawn_peer_adder(live: &Arc<TorrentStateLive>, mut peer_rx: PeerStream) {
    live.spawn(
        debug_span!(target: super::LOG_TARGET, parent: live.torrent().span.clone(), "external_peer_adder"),
        format!("[{}]external_peer_adder", live.shared.id),
        {
            let live = live.clone();
            move |cx| async move {
                let live = {
                    let weak = Arc::downgrade(&live);
                    drop(live);
                    weak
                };

                loop {
                    cx.checkpoint()?;
                    match timeout(&cx, Duration::from_secs(5), peer_rx.next()).await {
                        Ok(Some(peer)) => {
                            trace!(target: super::LOG_TARGET, ?peer, "received peer");
                            let live = match live.upgrade() {
                                Some(live) => live,
                                None => return Ok(()),
                            };
                            live.add_peer_from_discovery(peer)?;
                        }
                        Ok(None) => {
                            debug!(target: super::LOG_TARGET, "peer_rx closed, closing peer adder");
                            return Ok(());
                        }
                        // If timeout, check if the torrent is live.
                        Err(_) if live.strong_count() == 0 => {
                            debug!(target: super::LOG_TARGET, "timed out waiting for peers, torrent isn't live, closing peer adder");
                            return Ok(());
                        }
                        Err(_) => continue,
                    }
                }
            }
        },
    );
}

