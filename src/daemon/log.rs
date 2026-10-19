//! Process-local bounded history. The stderr copier feeds the same bytes that
//! journald receives; the API only reads this ring and never starts an owner.
use crate::api::{Hash, LogArgs, LogEntry, LogLevel, LogResult};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
type Wake = Arc<dyn Fn() + Send + Sync>;
#[derive(Default)]
struct State {
    next: u64,
    entries: VecDeque<LogEntry>,
}
#[derive(Default)]
pub(crate) struct Ring {
    state: Mutex<State>,
    wake: Mutex<Option<Wake>>,
}
fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}
pub(crate) fn matches(entry: &LogEntry, args: &LogArgs) -> bool {
    args.level
        .is_none_or(|level| rank(entry.level) <= rank(level))
        && args
            .source
            .as_ref()
            .is_none_or(|source| &entry.source == source)
        && args
            .hash
            .as_ref()
            .is_none_or(|hash| entry.hash.as_ref() == Some(hash))
}
impl Ring {
    pub fn set_waker(&self, wake: Wake) {
        *self.wake.lock().unwrap() = Some(wake);
    }
    pub fn line(&self, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut fields = line.splitn(3, ' ');
        let (level, source, message) = match (fields.next(), fields.next(), fields.next()) {
            (Some(level @ ("ERROR" | "WARN" | "INFO" | "DEBUG")), Some(source), Some(message)) => (
                match level {
                    "ERROR" => LogLevel::Error,
                    "WARN" => LogLevel::Warn,
                    "INFO" => LogLevel::Info,
                    _ => LogLevel::Debug,
                },
                source,
                message,
            ),
            _ => (LogLevel::Debug, "daemon", line),
        };
        let hash = message.split("hash=").skip(1).find_map(|part| {
            if part.len() >= 40
                && part.is_char_boundary(40)
                && !part.as_bytes().get(40).is_some_and(u8::is_ascii_hexdigit)
            {
                Hash::new(part[..40].to_owned()).ok()
            } else {
                None
            }
        });
        let entry = LogEntry {
            seq: 0,
            ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            level,
            source: source.chars().take(64).collect(),
            message: message.chars().take(8192).collect(),
            hash,
        };
        let mut state = self.state.lock().unwrap();
        state.next = state.next.saturating_add(1);
        let mut entry = entry;
        entry.seq = state.next;
        if state.entries.len() == 2000 {
            state.entries.pop_front();
        }
        state.entries.push_back(entry);
        drop(state);
        if let Some(wake) = self.wake.lock().unwrap().as_ref() {
            wake();
        }
    }
    pub fn all(&self) -> Vec<LogEntry> {
        self.state.lock().unwrap().entries.iter().cloned().collect()
    }
    pub fn query(&self, args: &LogArgs) -> LogResult {
        let state = self.state.lock().unwrap();
        let limit = args.limit.unwrap_or(200).min(1000) as usize;
        let matching = || {
            state.entries.iter().filter(|entry| {
                args.since_seq.is_none_or(|seq| entry.seq > seq) && matches(entry, args)
            })
        };
        let entries: Vec<_> = if args.since_seq.is_some() {
            matching().take(limit).cloned().collect()
        } else {
            let count = matching().count();
            matching()
                .skip(count.saturating_sub(limit))
                .cloned()
                .collect()
        };
        let next_seq = entries.last().map(|entry| entry.seq).unwrap_or(state.next);
        LogResult { entries, next_seq }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_bound() {
        let ring = Ring::default();
        for _ in 0..2001 {
            ring.line("INFO queue admitted");
        }
        let all = ring.all();
        assert_eq!(all.len(), 2000);
        assert_eq!(all[0].seq, 2);
        assert_eq!(all[1999].seq, 2001);
        assert_eq!(ring.query(&LogArgs::default()).entries.len(), 200);
    }
    #[test]
    fn seq_monotonic() {
        let ring = Ring::default();
        ring.line("INFO daemon ready");
        ring.line("WARN network waiting");
        let first = ring.query(&LogArgs {
            since_seq: Some(0),
            limit: Some(1),
            ..Default::default()
        });
        assert_eq!(first.next_seq, 1);
        let second = ring.query(&LogArgs {
            since_seq: Some(first.next_seq),
            ..Default::default()
        });
        assert_eq!(second.entries[0].seq, 2);
        assert_eq!(second.next_seq, 2);
    }
    #[test]
    fn filter_level_source() {
        let ring = Ring::default();
        let hash = "a".repeat(40);
        ring.line("DEBUG engine idle");
        ring.line("WARN queue full");
        ring.line(&format!("ERROR engine failed hash={hash}"));
        let args = LogArgs {
            level: Some(LogLevel::Warn),
            source: Some("engine".into()),
            hash: Some(Hash::new(hash).unwrap()),
            ..Default::default()
        };
        let result = ring.query(&args);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].level, LogLevel::Error);
    }
}
