use std::{
    collections::VecDeque,
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use rqbit_runtime::sync::Notify;

#[derive(Clone, Copy)]
struct ProgressSnapshot {
    progress_bytes: u64,
    instant: Instant,
}

/// Estimates download/upload speed in a sliding time window.
pub struct SpeedEstimator {
    latest_per_second_snapshots: Mutex<VecDeque<ProgressSnapshot>>,
    bytes_per_second: AtomicU64,
    time_remaining_millis: AtomicU64,
    activity_revision: AtomicU64,
    activity_notify: Notify,
}

impl Default for SpeedEstimator {
    fn default() -> Self {
        Self::new(5)
    }
}

impl SpeedEstimator {
    pub fn new(window_seconds: usize) -> Self {
        assert!(window_seconds > 1);
        Self {
            latest_per_second_snapshots: Mutex::new(VecDeque::with_capacity(window_seconds)),
            bytes_per_second: Default::default(),
            time_remaining_millis: Default::default(),
            activity_revision: Default::default(),
            activity_notify: Notify::new(),
        }
    }

    pub fn time_remaining(&self) -> Option<Duration> {
        let tr = self.time_remaining_millis.load(Ordering::Relaxed);
        if tr == 0 {
            return None;
        }
        Some(Duration::from_millis(tr))
    }

    pub fn bps(&self) -> u64 {
        self.bytes_per_second.load(Ordering::Relaxed)
    }

    pub fn mbps(&self) -> f64 {
        self.bps() as f64 / 1024f64 / 1024f64
    }

    /// Wake only when the rate crosses zero. Capture the revision at creation
    /// and arm before checking it so a transition before the first poll is kept.
    pub fn activity_changed(&self) -> impl std::future::Future<Output = ()> + Send + '_ {
        let revision = self.activity_revision.load(Ordering::Acquire);
        async move {
            let mut notified = std::pin::pin!(self.activity_notify.notified());
            std::future::poll_fn(|cx| {
                let notification = notified.as_mut().poll(cx);
                if self.activity_revision.load(Ordering::Acquire) != revision {
                    std::task::Poll::Ready(())
                } else {
                    notification
                }
            })
            .await;
        }
    }

    pub fn add_snapshot(
        &self,
        progress_bytes: u64,
        remaining_bytes: Option<u64>,
        instant: Instant,
    ) {
        let first = {
            let mut g = self.latest_per_second_snapshots.lock();

            let current = ProgressSnapshot {
                progress_bytes,
                instant,
            };

            if g.is_empty() {
                g.push_back(current);
                return;
            } else if g.len() < g.capacity() {
                g.push_back(current);
                g.front().copied().unwrap()
            } else {
                let first = g.pop_front().unwrap();
                g.push_back(current);
                first
            }
        };

        let downloaded_bytes_diff = progress_bytes - first.progress_bytes;
        let elapsed = instant - first.instant;
        let bps = downloaded_bytes_diff as f64 / elapsed.as_secs_f64();

        let time_remaining_millis_rounded: u64 = if downloaded_bytes_diff > 0 {
            let time_remaining_secs = remaining_bytes.unwrap_or_default() as f64 / bps;
            (time_remaining_secs * 1000f64) as u64
        } else {
            0
        };
        self.time_remaining_millis
            .store(time_remaining_millis_rounded, Ordering::Relaxed);
        let bps = bps as u64;
        let previous = self.bytes_per_second.swap(bps, Ordering::Relaxed);
        if (previous == 0) != (bps == 0) {
            self.activity_revision.fetch_add(1, Ordering::Release);
            self.activity_notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    #[derive(Default)]
    struct Wakes(AtomicU64);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn activity_wakes_only_on_zero_crossings_and_retains_unpolled_transition() {
        let estimator = SpeedEstimator::new(2);
        let began = Instant::now();
        let sample = |bytes, seconds| {
            estimator.add_snapshot(bytes, None, began + Duration::from_secs(seconds))
        };
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        sample(0, 0);
        {
            let mut change = std::pin::pin!(estimator.activity_changed());
            assert!(change.as_mut().poll(&mut cx).is_pending());
            sample(100, 1);
            assert_eq!(
                wakes.0.load(Ordering::Relaxed),
                1,
                "must wake without manual polling"
            );
            assert_eq!(change.as_mut().poll(&mut cx), Poll::Ready(()));
        }
        {
            let mut change = std::pin::pin!(estimator.activity_changed());
            assert!(change.as_mut().poll(&mut cx).is_pending());
            sample(200, 2);
            sample(200, 3);
            assert_eq!(
                wakes.0.load(Ordering::Relaxed),
                1,
                "rate magnitude changes stay silent"
            );
            assert!(change.as_mut().poll(&mut cx).is_pending());
            sample(200, 4);
            assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
            assert!(change.as_mut().poll(&mut cx).is_ready());
        }
        let mut change = std::pin::pin!(estimator.activity_changed());
        sample(300, 5);
        assert!(
            change.as_mut().poll(&mut cx).is_ready(),
            "construction-to-poll gap must not lose a transition"
        );
    }
}
