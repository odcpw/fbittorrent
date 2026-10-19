//! UI recovery retries only the socket. Starting an owner requires an explicit action.
use super::*;
use std::{borrow::Cow, time::Instant};

#[derive(Debug)]
pub(in crate::tui) struct Failure {
    pub message: String,
    pub permanent: bool,
    pub absent: bool,
}
impl Failure {
    pub(super) fn capture(error: anyhow::Error) -> Self {
        if let Some(wire::RemoteError(error)) = error.downcast_ref::<wire::RemoteError>() {
            if error.code == crate::api::ErrorCode::ProtoUnsupported {
                let protocol = error.details.as_ref().and_then(|details| details.get("server_proto"));
                return Self {
                    message: format!("This fbittorrent speaks API {}; the running fbittorrent speaks API {}. Restart to update.",
                        crate::api::PROTO, protocol.map_or_else(|| "an incompatible version".into(), ToString::to_string)),
                    permanent: true,
                    absent: false,
                };
            }
        }
        let absent = error.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused)
        });
        Self { message: error.to_string(), permanent: false, absent }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(in crate::tui) enum Notice<'a> {
    Connecting,
    Retrying(u64),
    Stopped,
    Incompatible(&'a str),
    Upgrade(&'a str),
}
impl Notice<'_> {
    pub fn text(self) -> Cow<'static, str> {
        match self {
            Self::Connecting => "Connecting…".into(),
            Self::Retrying(seconds) => format!("Reconnecting… (retry in {seconds} s)").into(),
            Self::Stopped => "fbittorrent stopped · Enter starts it again".into(),
            Self::Incompatible(text) | Self::Upgrade(text) => text.to_owned().into(),
        }
    }
}

#[derive(Default)]
pub(in crate::tui) struct Recovery {
    pub active: bool,
    since: Option<Instant>,
    due: Option<Instant>,
    in_flight: bool,
    attempts: usize,
    absent: bool,
    intentional: bool,
    terminal: Option<String>,
    upgrade: Option<String>,
}
impl Recovery {
    pub fn begin(&mut self, now: Instant) -> bool {
        if self.in_flight { return false; }
        self.active = true;
        self.since = Some(now);
        self.in_flight = true;
        self.attempts = 0;
        self.due = None;
        self.terminal = None;
        self.intentional = false;
        self.absent = false;
        true
    }
    pub fn connected(&mut self, running: &str, installed: &str) {
        self.active = true;
        self.since = None;
        self.due = None;
        self.in_flight = false;
        self.attempts = 0;
        self.terminal = None;
        self.intentional = false;
        self.absent = false;
        self.upgrade = stable_version(running).zip(stable_version(installed))
            .filter(|(old, new)| old < new)
            .map(|_| format!("fbittorrent {running} runs; {installed} is installed. Restart to update."));
    }
    pub fn lost(&mut self, now: Instant, intentional: bool) -> bool {
        self.intentional |= intentional;
        if self.since.is_some() { return false; }
        self.active = true;
        self.since = Some(now);
        self.attempts = 0;
        self.due = Some(now + Duration::from_millis(500));
        self.in_flight = false;
        true
    }
    pub fn failed(&mut self, failure: &Failure, now: Instant) {
        self.active = true;
        self.since.get_or_insert(now);
        self.in_flight = false;
        self.absent = failure.absent;
        if failure.permanent {
            self.terminal = Some(failure.message.clone());
            self.due = None;
        } else {
            const DELAYS: [u64; 6] = [500, 1000, 2000, 4000, 8000, 10_000];
            self.due = Some(now + Duration::from_millis(DELAYS[self.attempts.min(5)]));
        }
    }
    pub fn retry_due(&mut self, now: Instant) -> bool {
        if self.in_flight || self.due.is_none_or(|due| now < due) { return false; }
        self.in_flight = true;
        self.attempts = self.attempts.saturating_add(1);
        self.due = None;
        true
    }
    pub fn stopped(&self, now: Instant) -> bool {
        matches!(self.notice(now), Some(Notice::Stopped))
    }
    pub fn notice(&self, now: Instant) -> Option<Notice<'_>> {
        if let Some(error) = self.terminal.as_deref() { return Some(Notice::Incompatible(error)); }
        if let Some(since) = self.since {
            if self.intentional { return Some(Notice::Stopped); }
            if now.saturating_duration_since(since) < Duration::from_secs(2) { return Some(Notice::Connecting); }
            if self.absent { return Some(Notice::Stopped); }
            let seconds = self.due.map_or(0, |due| due.saturating_duration_since(now).as_millis().div_ceil(1000) as u64);
            return Some(Notice::Retrying(seconds));
        }
        self.upgrade.as_deref().map(Notice::Upgrade)
    }
}
fn stable_version(value: &str) -> Option<[u64; 3]> {
    let mut parts = value.split('.');
    let result = [parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?];
    parts.next().is_none().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_ladder_is_bounded_and_success_resets_it() {
        let mut recovery = Recovery::default();
        let mut now = Instant::now();
        recovery.connected("0.1.0", "0.1.0");
        recovery.lost(now, false);
        let failure = Failure { message: "closed".into(), permanent: false, absent: false };
        for delay in [500, 1000, 2000, 4000, 8000, 10_000, 10_000] {
            now += Duration::from_millis(delay - 1);
            assert!(!recovery.retry_due(now));
            now += Duration::from_millis(1);
            assert!(recovery.retry_due(now));
            assert!(!recovery.retry_due(now + Duration::from_secs(100)), "only one handshake in flight");
            recovery.failed(&failure, now);
        }
        recovery.connected("0.1.0", "0.1.0");
        assert_eq!(recovery.notice(now), None);
        recovery.lost(now, false);
        assert!(!recovery.retry_due(now + Duration::from_millis(499)));
        assert!(recovery.retry_due(now + Duration::from_millis(500)));
    }
    #[test]
    fn connection_banners_and_protocol_failure_do_not_restart_an_owner() {
        let now = Instant::now();
        let mut recovery = Recovery::default();
        assert!(recovery.begin(now));
        assert!(!recovery.begin(now));
        assert_eq!(recovery.notice(now), Some(Notice::Connecting));
        let mut failure = Failure { message: "refused".into(), permanent: false, absent: true };
        recovery.failed(&failure, now);
        assert_eq!(recovery.notice(now + Duration::from_secs(2)), Some(Notice::Stopped));
        failure.absent = false;
        recovery.failed(&failure, now + Duration::from_secs(3));
        assert_eq!(recovery.notice(now + Duration::from_secs(3)), Some(Notice::Retrying(1)));
        failure.permanent = true;
        failure.message = "API mismatch".into();
        recovery.failed(&failure, now);
        assert_eq!(recovery.notice(now), Some(Notice::Incompatible("API mismatch")));
        assert!(!recovery.retry_due(now + Duration::from_secs(1000)));
        recovery.connected("0.2.0", "0.2.1");
        assert_eq!(recovery.notice(now).unwrap().text(), "fbittorrent 0.2.0 runs; 0.2.1 is installed. Restart to update.");
        recovery.connected("0.2.1", "0.2.0");
        assert!(recovery.notice(now).is_none(), "an older installed binary must not suggest an upgrade");
    }
}
