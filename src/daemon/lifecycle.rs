//! Per-application lifecycle requests shared by the engine and its owner.
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Signal(i32),
    Idle,
    Panic,
    Upgrade,
    Quit,
}

impl StopReason {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Signal(_) | Self::Idle | Self::Quit => 0,
            Self::Panic => 70,
            Self::Upgrade => 75,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartReason {
    Settings,
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Stop(StopReason),
    Restart(RestartReason),
}

#[derive(Default)]
struct State {
    requests: VecDeque<Request>,
}

#[derive(Clone, Default)]
pub struct Lifecycle(Arc<(Mutex<State>, Condvar)>);

impl Lifecycle {
    /// Stopping supersedes restarts which the owner has not handled yet.
    /// Preserve the first pending stop and its reason; a late restart cannot
    /// reopen an application that is already closing.
    pub fn request_stop(&self, reason: StopReason) {
        let (mutex, changed) = &*self.0;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        state
            .requests
            .retain(|request| matches!(request, Request::Stop(_)));
        if state.requests.is_empty() {
            state.requests.push_back(Request::Stop(reason));
        }
        changed.notify_one();
    }

    pub fn request_restart(&self, reason: RestartReason) {
        let (mutex, changed) = &*self.0;
        let mut state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        if !state
            .requests
            .iter()
            .any(|request| matches!(request, Request::Stop(_)))
        {
            state.requests.push_back(Request::Restart(reason));
            changed.notify_one();
        }
    }

    /// Take the oldest pending request, waiting at most `timeout`. The predicate
    /// handles spurious wakeups without extending the timeout.
    pub fn wait(&self, timeout: Duration) -> Option<Request> {
        let (mutex, changed) = &*self.0;
        let state = mutex.lock().unwrap_or_else(|error| error.into_inner());
        let (mut state, _) = changed
            .wait_timeout_while(state, timeout, |state| state.requests.is_empty())
            .unwrap_or_else(|error| error.into_inner());
        state.requests.pop_front()
    }

    /// The current TUI observes requests while its outer owner consumes them
    /// after teardown. Observation must not steal the owner's restart reason.
    #[cfg(test)]
    pub fn pending_request(&self) -> Option<Request> {
        let (mutex, _) = &*self.0;
        mutex
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requests
            .front()
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_wins() {
        let lifecycle = Lifecycle::default();
        lifecycle.request_restart(RestartReason::Settings);
        lifecycle.request_stop(StopReason::Panic);
        lifecycle.request_restart(RestartReason::Explicit);
        lifecycle.request_stop(StopReason::Quit);
        let observed = [
            lifecycle.wait(Duration::ZERO),
            lifecycle.wait(Duration::ZERO),
        ];
        println!("stop supersedes pending restart, observed: {observed:?}");
        assert_eq!(observed, [Some(Request::Stop(StopReason::Panic)), None]);
    }

    #[test]
    fn restart_then_stop() {
        let lifecycle = Lifecycle::default();
        lifecycle.request_restart(RestartReason::Settings);
        let restart = lifecycle.wait(Duration::ZERO).unwrap();
        lifecycle.request_stop(StopReason::Idle);
        let stop = lifecycle.wait(Duration::ZERO).unwrap();
        let observed = [restart, stop];
        println!("handled restart then stop, observed: {observed:?}");
        assert_eq!(
            observed,
            [
                Request::Restart(RestartReason::Settings),
                Request::Stop(StopReason::Idle)
            ]
        );
        assert_eq!(lifecycle.wait(Duration::ZERO), None);
    }

    #[test]
    fn pending_restarts_are_fifo_and_observation_does_not_consume() {
        let lifecycle = Lifecycle::default();
        lifecycle.request_restart(RestartReason::Explicit);
        lifecycle.request_restart(RestartReason::Settings);
        let network = Some(Request::Restart(RestartReason::Explicit));
        assert_eq!(lifecycle.pending_request(), network);
        assert_eq!(lifecycle.pending_request(), network);
        let observed = [
            lifecycle.wait(Duration::ZERO),
            lifecycle.wait(Duration::ZERO),
        ];
        println!("oldest pending requests, observed: {observed:?}");
        assert_eq!(
            observed,
            [network, Some(Request::Restart(RestartReason::Settings))]
        );
    }

    #[test]
    fn wait_wakes_on_request_and_times_out_without_one() {
        let lifecycle = Lifecycle::default();
        let (entered, ready) = std::sync::mpsc::channel();
        let receiver = lifecycle.clone();
        let waiter = std::thread::spawn(move || {
            entered.send(()).unwrap();
            receiver.wait(Duration::from_secs(5))
        });
        ready.recv().unwrap();
        let began = std::time::Instant::now();
        // Whether the waiter is already asleep or still entering wait, the
        // notification/predicate pair must not lose the request.
        lifecycle.request_stop(StopReason::Quit);
        assert_eq!(
            waiter.join().unwrap(),
            Some(Request::Stop(StopReason::Quit))
        );
        assert!(
            began.elapsed() < Duration::from_secs(4),
            "request must wake the waiter before its five-second timeout"
        );
        let began = std::time::Instant::now();
        assert_eq!(lifecycle.wait(Duration::from_millis(20)), None);
        assert!(began.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn stop_exit_codes() {
        for reason in [
            StopReason::Signal(libc::SIGTERM),
            StopReason::Idle,
            StopReason::Quit,
        ] {
            assert_eq!(reason.exit_code(), 0);
        }
        assert_eq!(StopReason::Panic.exit_code(), 70);
        assert_eq!(StopReason::Upgrade.exit_code(), 75);
    }
}
