//! Native effects shared by rqbit's crates. Every task belongs to a runtime
//! region; this crate neither creates an executor nor provides Tokio adapters.
pub use asupersync::{cx, fs, io, types, Cx};

pub mod net {
    use crate::Cx;
    pub use asupersync::net::*;
    mod bound_dns;
    mod bound_tcp;
    mod interface;
    pub use bound_dns::{resolve_bound, resolve_host};
    pub use bound_tcp::{BoundReadHalf, BoundTcpListener, BoundTcpStream, BoundWriteHalf};
    pub use interface::InterfaceBinding;
    #[cfg(test)]
    pub(crate) use interface::{ip, isolated_netns};
    use std::net::{SocketAddr, ToSocketAddrs};
    pub async fn lookup_host<A: ToSocketAddrs + Send + 'static>(
        cx: &Cx,
        addr: A,
    ) -> std::io::Result<std::vec::IntoIter<SocketAddr>> {
        cx.checkpoint().map_err(std::io::Error::other)?;
        let mut work = cx
            .spawn_blocking(move |_| addr.to_socket_addrs().map(|it| it.collect::<Vec<_>>()))
            .map_err(std::io::Error::other)?;
        work.join(cx)
            .await
            .map_err(|e| std::io::Error::other(format!("{e:?}")))??
            .into_iter()
            .pipe(Ok)
    }
    trait Pipe: Sized {
        fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
            f(self)
        }
    }
    impl<T> Pipe for T {}
}

pub mod sync {
    pub use crate::cancel::CancellationToken;
    pub use asupersync::channel::{broadcast, mpsc, oneshot, watch};
    pub use asupersync::sync::semaphore::AcquireError;
    pub use asupersync::sync::{Mutex, Notify, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
}

pub mod time {
    use crate::Cx;
    pub use asupersync::time::{
        timeout_at, Elapsed, MissedTickBehavior, TimeSource, TimerDriver, TimerDriverHandle,
    };
    use std::{future::Future, time::Duration};
    pub async fn sleep(cx: &Cx, duration: Duration) {
        if cx.checkpoint().is_err() {
            return;
        }
        asupersync::time::sleep(cx.now(), duration).await;
        let _ = cx.checkpoint();
    }
    pub async fn timeout<F: Future>(
        cx: &Cx,
        duration: Duration,
        future: F,
    ) -> Result<F::Output, Elapsed> {
        asupersync::time::timeout(cx.now(), duration, future).await
    }
    pub struct Interval(asupersync::time::Interval);
    pub fn interval(cx: &Cx, duration: Duration) -> Interval {
        Interval(asupersync::time::interval(cx.now(), duration))
    }
    impl Interval {
        pub async fn tick(&mut self, cx: &Cx) -> crate::types::Time {
            sleep(cx, self.0.remaining(cx.now())).await;
            self.0.tick(cx.now())
        }
        pub fn set_missed_tick_behavior(&mut self, behavior: MissedTickBehavior) {
            self.0.set_missed_tick_behavior(behavior);
        }
    }
}

pub mod task {
    use crate::Cx;
    use asupersync::runtime::JoinError as NativeJoinError;
    pub use asupersync::runtime::{SpawnError, TaskHandle};
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum JoinError {
        Cancelled(crate::CancelReason),
        Panicked(crate::types::PanicPayload),
        PolledAfterCompletion,
        Admission(SpawnError),
    }
    impl From<NativeJoinError> for JoinError {
        fn from(error: NativeJoinError) -> Self {
            match error {
                NativeJoinError::Cancelled(reason) => Self::Cancelled(reason),
                NativeJoinError::Panicked(payload) => Self::Panicked(payload),
                NativeJoinError::PolledAfterCompletion => Self::PolledAfterCompletion,
            }
        }
    }
    impl std::fmt::Display for JoinError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Cancelled(reason) => write!(formatter, "task was cancelled: {reason}"),
                Self::Panicked(payload) => write!(formatter, "task panicked: {payload}"),
                Self::PolledAfterCompletion => {
                    write!(formatter, "join future polled after completion")
                }
                Self::Admission(error) => {
                    write!(formatter, "native task admission failed: {error}")
                }
            }
        }
    }
    impl std::error::Error for JoinError {}
    /// An awaitable view of a native region-owned task. Native Cx spawn's
    /// acknowledged cancellation result is retained. Immediate admission
    /// failure is reported through the handle without invoking its task body.
    pub struct JoinHandle<T> {
        inner: Option<TaskHandle<T>>,
        rejected: Option<SpawnError>,
    }
    impl<T> JoinHandle<T> {
        fn admitted(inner: TaskHandle<T>) -> Self {
            Self {
                inner: Some(inner),
                rejected: None,
            }
        }
        fn rejected(error: SpawnError) -> Self {
            Self {
                inner: None,
                rejected: Some(error),
            }
        }
        pub fn abort(&self) {
            if let Some(inner) = &self.inner {
                inner.abort();
            }
        }
        pub fn is_finished(&self) -> bool {
            self.inner.as_ref().is_none_or(TaskHandle::is_finished)
        }
    }
    impl<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if let Some(error) = self.rejected.take() {
                return Poll::Ready(Err(JoinError::Admission(error)));
            }
            match &mut self.inner {
                Some(inner) => inner
                    .poll_join(cx)
                    .map(|result| result.map_err(JoinError::from)),
                None => Poll::Ready(Err(JoinError::PolledAfterCompletion)),
            }
        }
    }
    pub fn spawn<F>(cx: &Cx, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        spawn_with(cx, move |_| future)
    }
    pub fn try_spawn_with<F, Fut>(cx: &Cx, make: F) -> Result<JoinHandle<Fut::Output>, SpawnError>
    where
        F: FnOnce(Cx) -> Fut + Send + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        cx.spawn(make).map(JoinHandle::admitted)
    }
    pub fn spawn_with<F, Fut>(cx: &Cx, make: F) -> JoinHandle<Fut::Output>
    where
        F: FnOnce(Cx) -> Fut + Send + 'static,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        try_spawn_with(cx, make).unwrap_or_else(JoinHandle::rejected)
    }
    pub fn spawn_blocking<F, R>(cx: &Cx, work: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        cx.spawn_blocking(move |_| work())
            .map(JoinHandle::admitted)
            .unwrap_or_else(JoinHandle::rejected)
    }
    pub use asupersync::runtime::yield_now;
}

mod cancel {
    use crate::{sync::Notify, Cx};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    };
    #[derive(Default)]
    struct State {
        cancelled: AtomicBool,
        wake: Notify,
        children: Mutex<Vec<Weak<State>>>,
    }
    /// Protocol stop signal; the runtime's child regions own and drain tasks.
    #[derive(Clone, Default)]
    pub struct CancellationToken(Arc<State>);
    impl std::fmt::Debug for CancellationToken {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CancellationToken")
                .field("cancelled", &self.is_cancelled())
                .finish()
        }
    }
    pub struct DropGuard(Option<CancellationToken>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            if let Some(token) = self.0.take() {
                token.cancel();
            }
        }
    }
    impl CancellationToken {
        pub fn drop_guard(self) -> DropGuard {
            DropGuard(Some(self))
        }
        pub fn new() -> Self {
            Self::default()
        }
        pub fn is_cancelled(&self) -> bool {
            self.0.cancelled.load(Ordering::Acquire)
        }
        pub fn child_token(&self) -> Self {
            let child = Self::new();
            {
                let mut children = self.0.children.lock().unwrap();
                children.retain(|x| x.strong_count() > 0);
                children.push(Arc::downgrade(&child.0));
            }
            if self.is_cancelled() {
                child.cancel();
            }
            child
        }
        pub fn cancel(&self) {
            if self.0.cancelled.swap(true, Ordering::AcqRel) {
                return;
            }
            let children: Vec<_> = self
                .0
                .children
                .lock()
                .unwrap()
                .iter()
                .filter_map(Weak::upgrade)
                .collect();
            self.0.wake.notify_waiters();
            for child in children {
                Self(child).cancel();
            }
        }
        pub async fn cancelled(&self) {
            self.0.wake.wait_until(|| self.is_cancelled()).await;
        }
        pub async fn cancelled_with(&self, cx: &Cx) {
            self.0
                .wake
                .wait_until(|| self.is_cancelled() || cx.checkpoint().is_err())
                .await;
        }
    }
}

pub mod http;

pub mod runtime {
    pub use asupersync::runtime::*;
}
pub use asupersync::runtime::RuntimeBuilder;
pub use asupersync::types::Budget;

/// Poll protocol futures together. Losing futures are dropped before executing
/// the winner's branch. Task losers must still be cancelled and joined by their
/// region owner; this primitive does not detach or abandon task ownership.
#[macro_export]
macro_rules! select {
    ($($branches:tt)*) => {
        $crate::select_internal!(@parse context
            [(B0,f0,g0),(B1,f1,g1),(B2,f2,g2),(B3,f3,g3),(B4,f4,g4),(B5,f5,g5),(B6,f6,g6),(B7,f7,g7),(B8,f8,g8),(B9,f9,g9),]
            [] [] [] [] ; $($branches)*)
    };
}
#[doc(hidden)]
#[macro_export]
macro_rules! select_internal {
    (@parse $context:ident [$($slots:tt)*] [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ; ) => {{
        enum Selected<$($types)*> { $($types)* }
        // Replaced by the emit arm, whose enum payloads are accumulated below.
        compile_error!("invalid select branch syntax");
    }};
    (@parse $context:ident [($variant:ident,$future_var:ident,$guard_var:ident), $($slots:tt)*]
        [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ;
        $pattern:pat = $future:expr $(, if $guard:expr)? => $body:block, $($rest:tt)*) => {
        $crate::select_internal!(@add $context [$($slots)*] [$($types)*] [$($init)*] [$($poll)*] [$($arms)*]
            ($variant,$future_var,$guard_var) ($pattern) ($future) ($crate::select_internal!(@guard $($guard)?)) ($body) ; $($rest)*)
    };
    (@parse $context:ident [($variant:ident,$future_var:ident,$guard_var:ident), $($slots:tt)*]
        [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ;
        $pattern:pat = $future:expr $(, if $guard:expr)? => $body:block $($rest:tt)*) => {
        $crate::select_internal!(@add $context [$($slots)*] [$($types)*] [$($init)*] [$($poll)*] [$($arms)*]
            ($variant,$future_var,$guard_var) ($pattern) ($future) ($crate::select_internal!(@guard $($guard)?)) ($body) ; $($rest)*)
    };
    (@parse $context:ident [($variant:ident,$future_var:ident,$guard_var:ident), $($slots:tt)*]
        [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ;
        $pattern:pat = $future:expr $(, if $guard:expr)? => $body:expr, $($rest:tt)*) => {
        $crate::select_internal!(@add $context [$($slots)*] [$($types)*] [$($init)*] [$($poll)*] [$($arms)*]
            ($variant,$future_var,$guard_var) ($pattern) ($future) ($crate::select_internal!(@guard $($guard)?)) ($body) ; $($rest)*)
    };
    (@parse $context:ident [($variant:ident,$future_var:ident,$guard_var:ident), $($slots:tt)*]
        [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ;
        $pattern:pat = $future:expr $(, if $guard:expr)? => $body:expr) => {
        $crate::select_internal!(@add $context [$($slots)*] [$($types)*] [$($init)*] [$($poll)*] [$($arms)*]
            ($variant,$future_var,$guard_var) ($pattern) ($future) ($crate::select_internal!(@guard $($guard)?)) ($body) ;)
    };
    (@guard) => { true };
    (@guard $guard:expr) => { $guard };
    (@add $context:ident [$($slots:tt)*] [$(($types:ident))*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*]
        ($variant:ident,$future_var:ident,$guard_var:ident) ($pattern:pat) ($future:expr) ($guard:expr) ($body:expr) ; $($rest:tt)*) => {
        $crate::select_internal!(@next $context [$($slots)*] [$(($types))* ($variant)]
            [$($init)* let mut $guard_var = $guard; let mut $future_var = ::std::boxed::Box::pin($future);]
            [$($poll)* if $guard_var { if let ::std::task::Poll::Ready(output) = ::std::future::Future::poll($future_var.as_mut(), $context) {
                $guard_var = false;
                #[allow(unused_variables)]
                if ::std::matches!(&output, $pattern) { return ::std::task::Poll::Ready(Selected::$variant(output)); }
            } }]
            [$($arms)* Selected::$variant($pattern) => $body,] ; $($rest)*)
    };
    (@next $context:ident [$($slots:tt)*] [$(($types:ident))*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ; ) => {{
        enum Selected<$($types,)*> { $($types($types),)* }
        let selected = {
            $($init)*
            ::std::future::poll_fn(|$context| { $($poll)* ::std::task::Poll::Pending }).await
        };
        #[allow(unreachable_patterns)]
        match selected { $($arms)* _ => unreachable!("selection pattern checked during polling"), }
    }};
    (@next $context:ident [$($slots:tt)*] [$($types:tt)*] [$($init:tt)*] [$($poll:tt)*] [$($arms:tt)*] ; $($rest:tt)+) => {
        $crate::select_internal!(@parse $context [$($slots)*] [$($types)*] [$($init)*] [$($poll)*] [$($arms)*] ; $($rest)+)
    };
}
pub use asupersync::types::CancelReason;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    fn runtime() -> asupersync::runtime::Runtime {
        RuntimeBuilder::new()
            .worker_threads(2)
            .blocking_threads(1, 2)
            .build()
            .unwrap()
    }
    #[test]
    fn deterministic_select_disables_guards_and_mismatched_patterns() {
        let rt = runtime();
        let value = rt.block_on(async {
            select! {
                Some(value) = std::future::ready(None::<u8>) => value,
                value = std::future::ready(99), if false => value,
                value = std::future::ready(42) => { value },
            }
        });
        assert_eq!(value, 42);
        assert!(rt.shutdown_timeout(Duration::from_secs(1)));
    }
    #[test]
    fn guarded_block_branches_keep_trailing_commas() {
        let rt = runtime();
        let value = rt.block_on(async {
            select! {
                _ = std::future::pending::<()>() => { 9 },
                Some(Ok((left, right))) = std::future::ready(Some(Ok::<_, ()>((1, 2)))), if true => { left + right },
            }
        });
        assert_eq!(value, 3);
        assert!(rt.shutdown_timeout(Duration::from_secs(1)));
    }
    #[test]
    fn hierarchical_stop_does_not_cancel_siblings() {
        let parent = sync::CancellationToken::new();
        let first = parent.child_token();
        let second = parent.child_token();
        first.cancel();
        assert!(!parent.is_cancelled());
        assert!(!second.is_cancelled());
        parent.cancel();
        assert!(second.is_cancelled());
        assert!(parent.child_token().is_cancelled());
    }
    #[test]
    fn closed_native_region_rejects_async_and_blocking_bodies() {
        let rt = runtime();
        let cx = rt.request_cx_with_budget(Budget::INFINITE);
        let ran = Arc::new(AtomicBool::new(false));
        rt.block_on(async {
            let region = cx
                .open_child_region(ChildRegionSpec::inherit())
                .await
                .unwrap();
            let closed = region.cx().clone();
            region
                .cancel(CancelReason::user("admission fixture stopped"))
                .unwrap();
            region.close().await.unwrap();
            let async_ran = ran.clone();
            let task = task::spawn_with(&closed, move |_| async move {
                async_ran.store(true, Ordering::Release);
            });
            // Mailbox admission can resolve asynchronously through native
            // Cancelled; an unavailable gateway reports Admission directly.
            assert!(matches!(
                task.await,
                Err(task::JoinError::Cancelled(_)) | Err(task::JoinError::Admission(_))
            ));
            let blocking_ran = ran.clone();
            let work = task::spawn_blocking(&closed, move || {
                blocking_ran.store(true, Ordering::Release);
            });
            assert!(matches!(
                work.await,
                Err(task::JoinError::Cancelled(_)) | Err(task::JoinError::Admission(_))
            ));
        });
        assert!(!ran.load(Ordering::Acquire));
        assert!(rt.shutdown_timeout(Duration::from_secs(1)));
    }
    #[test]
    fn lab_pause_drains_a_channel_waiter_and_releases_its_resources() {
        for seed in [7, 19, 73] {
            let (released, report) = asupersync::lab::run_async_under_lab(seed, |cx| async move {
                struct Held(Arc<AtomicBool>);
                impl Drop for Held {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let region = cx
                    .open_child_region(ChildRegionSpec::inherit())
                    .await
                    .unwrap();
                let released = Arc::new(AtomicBool::new(false));
                let witness = released.clone();
                let (commands, mut receive) = sync::mpsc::channel(1);
                let (parked, mut observed) = sync::oneshot::channel();
                let task = task::spawn_with(region.cx(), move |cx| async move {
                    let _held = Held(witness);
                    assert_eq!(receive.recv(&cx).await.unwrap(), 7);
                    parked.send(&cx, ()).unwrap();
                    receive.recv(&cx).await
                });
                commands.send(&cx, 7).await.unwrap();
                observed.recv(&cx).await.unwrap();
                region
                    .cancel(CancelReason::user("paused torrent fixture"))
                    .unwrap();
                region.close().await.unwrap();
                assert!(matches!(
                    task.await,
                    Ok(Err(sync::mpsc::RecvError::Cancelled))
                ));
                released.load(Ordering::Acquire)
            });
            assert!(released);
            assert!(report.lab_test_passed(), "seed {seed}: {report:?}");
            assert!(report.oracle_report.all_passed());
        }
    }
    #[test]
    fn native_region_close_drains_a_parked_task_and_releases_resources() {
        struct Witness(Arc<AtomicBool>);
        impl Drop for Witness {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let rt = runtime();
        let cx = rt.request_cx_with_budget(Budget::INFINITE);
        let released = Arc::new(AtomicBool::new(false));
        rt.block_on(async {
            let region = cx
                .open_child_region(cx::ChildRegionSpec::inherit())
                .await
                .unwrap();
            let (tx, mut rx) = sync::oneshot::channel();
            let release = released.clone();
            let task = task::spawn_with(region.cx(), move |cx| async move {
                let _held = Witness(release);
                let mut sleep = Box::pin(time::sleep(&cx, Duration::from_secs(120)));
                let mut parked = Some(tx);
                std::future::poll_fn(|poll_cx| {
                    let result = std::future::Future::poll(sleep.as_mut(), poll_cx);
                    if result.is_pending() {
                        // Publish only after Sleep registered and returned Pending.
                        if let Some(tx) = parked.take() {
                            tx.send(&cx, ()).unwrap();
                        }
                    }
                    result
                })
                .await;
                cx.checkpoint()?;
                Ok::<_, AsupError>(7)
            });
            rx.recv(&cx).await.unwrap();
            region.close().await.unwrap();
            assert!(
                matches!(task.await, Ok(Err(_))),
                "retain the typed cancellation after acknowledgement"
            );
        });
        assert!(released.load(Ordering::Acquire));
        assert!(rt.shutdown_timeout(Duration::from_secs(1)));
    }
}
pub use asupersync::channel;
pub use asupersync::cx::{ChildRegion, ChildRegionSpec};
pub use asupersync::error::Error as AsupError;
pub use cancel::{CancellationToken, DropGuard};
