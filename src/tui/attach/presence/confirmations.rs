//! Bounded completion cache closes the pending-reply/event race for callers with side effects.
use super::*;
use std::{collections::VecDeque, sync::Condvar, time::Instant};
#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<(crate::api::MutationRef, Result<Value, String>, usize)>,
    bytes: usize,
}
#[derive(Default)]
pub(super) struct Confirmations {
    cache: Mutex<Cache>,
    changed: Condvar,
}
impl Confirmations {
    pub fn observe(&self, event: &crate::api::Event) {
        let crate::api::Event::MutationResult {
            reference,
            outcome,
            error,
            result,
            ..
        } = event
        else {
            return;
        };
        let value = match outcome {
            crate::api::MutationOutcome::Done => Ok(result
                .clone()
                .map(Value::Object)
                .unwrap_or_else(|| serde_json::json!({"state":"done"}))),
            crate::api::MutationOutcome::Failed => Err(error
                .as_ref()
                .map(|error| error.message.clone())
                .unwrap_or_else(|| "mutation failed".into())),
            crate::api::MutationOutcome::Timeout => {
                Err("API mutation timed out; result unknown; check the list".into())
            }
        };
        let bytes = match &value {
            Ok(value) => {
                serde_json::to_vec(value).map_or(crate::api::LINE_OUT, |bytes| bytes.len())
            }
            Err(error) => error.len(),
        };
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        while cache.entries.len() >= 512
            || cache.bytes.saturating_add(bytes) > 2 * crate::api::LINE_OUT
        {
            let Some((_, _, bytes)) = cache.entries.pop_front() else {
                break;
            };
            cache.bytes -= bytes;
        }
        cache.bytes += bytes;
        cache.entries.push_back((reference.clone(), value, bytes));
        self.changed.notify_all();
    }
    pub fn closed(&self) {
        let _guard = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        self.changed.notify_all();
    }
    pub fn wait(
        &self,
        reference: &crate::api::MutationRef,
        deadline: Instant,
        closed: &AtomicBool,
    ) -> Result<Value> {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some((_, value, _)) = cache.entries.iter().find(|(key, _, _)| key == reference) {
                return value.clone().map_err(anyhow::Error::msg);
            }
            if closed.load(Ordering::Acquire) {
                bail!("API connection lost; result unknown; check the list");
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("API mutation timed out; result unknown; check the list");
            }
            cache = self
                .changed
                .wait_timeout(cache, remaining)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_before_waiting_still_controls_the_callers_side_effect() {
        let cache = Confirmations::default();
        let reference = crate::api::MutationRef::new("early-url-result").unwrap();
        let event = crate::api::Event::MutationResult {
            seq: 1,
            ts: 1,
            reference: reference.clone(),
            op: "torrents.add".into(),
            outcome: crate::api::MutationOutcome::Done,
            error: None,
            result: Some(
                serde_json::json!({"state":"done","hash":"a".repeat(40)})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        };
        cache.observe(&event);
        let result = cache
            .wait(&reference, Instant::now(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(result["hash"], "a".repeat(40));
        let missing = crate::api::MutationRef::new("not-confirmed").unwrap();
        assert!(cache
            .wait(&missing, Instant::now(), &AtomicBool::new(false))
            .is_err());
        let failed = crate::api::Event::MutationResult {
            seq: 2,
            ts: 2,
            reference: missing.clone(),
            op: "torrents.add".into(),
            outcome: crate::api::MutationOutcome::Failed,
            error: Some(crate::api::Error {
                code: crate::api::ErrorCode::Upstream,
                message: "local source failed".into(),
                retryable: true,
                hint: None,
                details: None,
            }),
            result: None,
        };
        cache.observe(&failed);
        assert!(cache
            .wait(&missing, Instant::now(), &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("local source failed"));
    }
}
