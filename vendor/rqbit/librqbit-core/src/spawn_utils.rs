use std::{borrow::Cow, fmt::Display, future::Future, task::Poll};

use rqbit_runtime::{Cx, sync::CancellationToken, task::JoinHandle};
use tracing::{Instrument, debug, error, trace};

/// Starts traced work in the caller's native Asupersync region.
///
/// The factory receives the task's own context, so cancellation checkpoints and
/// operations belong to the child task rather than its creator.
#[track_caller]
pub fn spawn<E, F, Fut>(
    cx: &Cx,
    span: tracing::Span,
    name: impl Into<Cow<'static, str>>,
    make_fut: F,
) -> JoinHandle<()>
where
    E: Display + Send + 'static,
    F: FnOnce(Cx) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
{
    let name = name.into();
    rqbit_runtime::task::spawn_with(cx, move |cx| {
        async move {
            if cx.checkpoint().is_err() {
                debug!("task cancelled before starting");
                return;
            }
            trace!("started");
            match make_fut(cx).await {
                Ok(()) => trace!("finished"),
                Err(e) => error!("{name} finished with error: {e:#}"),
            }
        }
        .instrument(span)
    })
}

/// Starts region-owned work with an additional application cancellation token.
#[track_caller]
pub fn spawn_with_cancel<E, F, Fut>(
    cx: &Cx,
    span: tracing::Span,
    name: impl Into<Cow<'static, str>>,
    cancellation_token: CancellationToken,
    make_fut: F,
) -> JoinHandle<()>
where
    E: Display + Send + 'static,
    F: FnOnce(Cx) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
{
    spawn(cx, span, name, move |cx| async move {
        let mut fut = std::pin::pin!(make_fut(cx.clone()));
        let mut cancelled = std::pin::pin!(cancellation_token.cancelled_with(&cx));
        std::future::poll_fn(|task_cx| {
            if cancelled.as_mut().poll(task_cx).is_ready() {
                let _ = cx.checkpoint();
                debug!("task cancelled");
                return Poll::Ready(Ok(()));
            }
            fut.as_mut().poll(task_cx)
        })
        .await
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use rqbit_runtime::{
        Budget, RuntimeBuilder,
        sync::{CancellationToken, oneshot},
    };

    use super::spawn_with_cancel;

    struct Cleanup(Arc<AtomicBool>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn cancellation_joins_a_parked_task_and_releases_its_state() {
        let runtime = RuntimeBuilder::current_thread().build().unwrap();
        let cx = runtime.request_cx_with_budget(Budget::INFINITE);
        let cleaned_up = Arc::new(AtomicBool::new(false));
        let witness = cleaned_up.clone();
        let cancel = CancellationToken::new();
        let (started, mut observed) = oneshot::channel();
        let handle = spawn_with_cancel(
            &cx,
            tracing::Span::none(),
            "parked-cancellation-test",
            cancel.clone(),
            move |cx| async move {
                let _cleanup = Cleanup(witness);
                started.send(&cx, ()).unwrap();
                std::future::pending::<Result<(), std::io::Error>>().await
            },
        );
        runtime.block_on(async {
            observed.recv(&cx).await.unwrap();
            assert!(!cleaned_up.load(Ordering::Acquire));
            cancel.cancel();
            rqbit_runtime::time::timeout(&cx, std::time::Duration::from_secs(2), handle)
                .await
                .unwrap()
                .unwrap();
        });
        assert!(cleaned_up.load(Ordering::Acquire));
    }
}
