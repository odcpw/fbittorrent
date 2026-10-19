//! Per-instance mutation receipts. The ring owns replay history, not execution:
//! an evicted receipt can remain alive in its worker without changing a newer ref.
use super::{
    ops::{error, ApiResult},
    Call, ErrorCode, Event, MutationRef, Reply,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const CAPACITY: usize = 256;
const TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Waiter {
    pub connection: u64,
    pub id: u64,
    pub generation: u64,
}
pub(crate) enum Delivery {
    Response(Waiter, ApiResult<Reply>),
    Event(u64, Event),
}
struct Receipt {
    call: Value,
    stored: Instant,
    answer: Option<ApiResult<Reply>>,
    waiting: Vec<Waiter>,
    watchers: HashSet<u64>,
    outcome: Option<Event>,
}
#[derive(Clone)]
pub(crate) struct Ticket {
    pub reference: MutationRef,
    receipt: Arc<Mutex<Receipt>>,
    clock: fn() -> Instant,
}
impl Ticket {
    // Delivery runs while holding this receipt, keeping an acknowledgement ahead
    // of its event even when completion races a reconnect/replay.
    pub fn attach(&self, waiter: Waiter, mut deliver: impl FnMut(Delivery)) {
        let mut receipt = self.receipt.lock().unwrap();
        receipt.watchers.insert(waiter.connection);
        if let Some(answer) = &receipt.answer {
            deliver(Delivery::Response(waiter, answer.clone()));
            if let Some(outcome) = &receipt.outcome {
                deliver(Delivery::Event(waiter.connection, outcome.clone()));
            }
        } else {
            receipt.waiting.push(waiter);
        }
    }
    pub fn answer(&self, answer: ApiResult<Reply>, mut deliver: impl FnMut(Delivery)) {
        let mut receipt = self.receipt.lock().unwrap();
        if receipt.answer.is_some() {
            return;
        }
        receipt.stored = (self.clock)();
        receipt.answer = Some(answer.clone());
        for waiter in receipt.waiting.drain(..) {
            deliver(Delivery::Response(waiter, answer.clone()));
        }
    }
    pub fn detach(&self, connection: u64) {
        let mut receipt = self.receipt.lock().unwrap();
        receipt.watchers.remove(&connection);
        receipt
            .waiting
            .retain(|waiter| waiter.connection != connection);
    }
    pub fn finish(&self, event: Event, mut deliver: impl FnMut(Delivery)) {
        let mut receipt = self.receipt.lock().unwrap();
        if receipt.outcome.is_some() {
            return;
        }
        receipt.outcome = Some(event.clone());
        for &connection in &receipt.watchers {
            deliver(Delivery::Event(connection, event.clone()));
        }
    }
}

pub struct Ring {
    clients: HashMap<String, VecDeque<Ticket>>,
    clock: fn() -> Instant,
    swept: Instant,
}
impl Default for Ring {
    fn default() -> Self {
        Self::with_clock(Instant::now)
    }
}
impl Ring {
    pub fn with_clock(clock: fn() -> Instant) -> Self {
        Self {
            clients: HashMap::new(),
            clock,
            swept: clock(),
        }
    }
    pub(crate) fn detach(&self, connection: u64) {
        for entries in self.clients.values() {
            for ticket in entries {
                ticket.detach(connection);
            }
        }
    }
    /// Returns (receipt, first admission). Call excludes transport id/instance/ref:
    /// only the operation and its arguments identify a mutation.
    pub(crate) fn claim(
        &mut self,
        client: &str,
        reference: MutationRef,
        call: &Call,
    ) -> ApiResult<(Ticket, bool)> {
        let now = (self.clock)();
        if now.saturating_duration_since(self.swept) >= Duration::from_secs(60) {
            self.clients.retain(|_, entries| {
                entries.retain(|entry| {
                    now.saturating_duration_since(entry.receipt.lock().unwrap().stored) < TTL
                });
                !entries.is_empty()
            });
            self.swept = now;
        }
        let entries = self.clients.entry(client.into()).or_default();
        entries.retain(|entry| {
            now.saturating_duration_since(entry.receipt.lock().unwrap().stored) < TTL
        });
        let call = serde_json::to_value(call).expect("serializable typed call");
        if let Some(entry) = entries.iter().find(|entry| entry.reference == reference) {
            if entry.receipt.lock().unwrap().call != call {
                return Err(error(
                    ErrorCode::InvalidArgs,
                    "ref reused for a different request",
                    Some("ref"),
                ));
            }
            return Ok((entry.clone(), false));
        }
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        let ticket = Ticket {
            reference,
            clock: self.clock,
            receipt: Arc::new(Mutex::new(Receipt {
                call,
                stored: now,
                answer: None,
                waiting: Vec::new(),
                watchers: HashSet::new(),
                outcome: None,
            })),
        };
        entries.push_back(ticket.clone());
        Ok((ticket, true))
    }
}

pub(crate) fn stale(
    call: &Call,
    requested: Option<&super::Instance>,
    current: &str,
) -> ApiResult<()> {
    let operation = super::ops::TABLE
        .iter()
        .find(|op| op.name == call.op())
        .expect("registered call");
    if operation.mutating
        && !operation.idempotent
        && requested.is_some_and(|instance| instance.as_str() != current)
    {
        return Err(error(
            ErrorCode::InvalidState,
            "daemon restarted since this request; check the list",
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    thread_local! { static TIME: std::cell::Cell<Instant> = std::cell::Cell::new(Instant::now()); }
    fn clock() -> Instant {
        TIME.with(|value| value.get())
    }
    fn advance(seconds: u64) {
        TIME.with(|value| value.set(value.get() + Duration::from_secs(seconds)));
        println!("ring clock: advanced {seconds}s");
    }
    fn call() -> Call {
        Call::TorrentsStop {
            args: HashesArgs {
                hashes: Hashes::new(vec![Hash::new("a".repeat(40)).unwrap()]).unwrap(),
            },
        }
    }
    fn reference(value: &str) -> MutationRef {
        MutationRef::new(value).unwrap()
    }
    fn waiter(connection: u64) -> Waiter {
        Waiter {
            connection,
            id: 1,
            generation: 1,
        }
    }
    #[test]
    fn ring_expiry() {
        let mut ring = Ring::with_clock(clock);
        let (ticket, fresh) = ring.claim("client", reference("r"), &call()).unwrap();
        assert!(fresh);
        ticket.answer(Err(error(ErrorCode::NotFound, "not found", None)), |_| {});
        advance(599);
        assert!(!ring.claim("client", reference("r"), &call()).unwrap().1);
        advance(1);
        assert!(ring.claim("client", reference("r"), &call()).unwrap().1);
    }
    #[test]
    fn ring_evicts_oldest() {
        let mut ring = Ring::default();
        for index in 0..257 {
            assert!(
                ring.claim("a", reference(&index.to_string()), &call())
                    .unwrap()
                    .1
            );
        }
        assert!(!ring.claim("a", reference("1"), &call()).unwrap().1);
        assert!(ring.claim("a", reference("0"), &call()).unwrap().1);
        // Replay never refreshes age or moves an entry to the back.
        assert!(ring.claim("a", reference("1"), &call()).unwrap().1);
    }
    #[test]
    fn replay_returns_stored_error() {
        let mut ring = Ring::default();
        let (ticket, _) = ring.claim("a", reference("r"), &call()).unwrap();
        let failure = error(ErrorCode::InvalidState, "test durable failure", None);
        ticket.answer(Err(failure.clone()), |_| {});
        let (replay, fresh) = ring.claim("a", reference("r"), &call()).unwrap();
        assert!(!fresh);
        let mut count = 0;
        replay.attach(waiter(2), |delivery| {
            if let Delivery::Response(_, answer) = delivery {
                assert_eq!(answer, Err(failure.clone()));
                count += 1;
            }
        });
        assert_eq!(count, 1);
    }
    #[test]
    fn named_clients_apply_independently_and_refs_compare_operation_and_args() {
        let mut ring = Ring::default();
        let mut applied = 0;
        for client in ["cli-1", "cli-2"] {
            let (ticket, fresh) = ring.claim(client, reference("same"), &call()).unwrap();
            if fresh {
                applied += 1;
                ticket.answer(
                    Ok(Reply::TorrentsStop(StopResult {
                        state: DoneState::Done,
                        stopped: vec![],
                    })),
                    |_| {},
                );
            }
        }
        assert_eq!(applied, 2);
        assert!(!ring.claim("cli-1", reference("same"), &call()).unwrap().1);
        let different = Call::TorrentsSetSeed {
            args: SetSeedArgs {
                hashes: Hashes::new(vec![Hash::new("a".repeat(40)).unwrap()]).unwrap(),
                on: true,
            },
        };
        let failure = ring
            .claim("cli-1", reference("same"), &different)
            .err()
            .unwrap();
        assert_eq!(failure.message, "ref reused for a different request");
        assert_eq!(failure.details.unwrap()["key"], "ref");
    }
    #[test]
    fn concurrent_replay_waits_and_pending_replay_receives_completion() {
        let mut ring = Ring::default();
        let (ticket, _) = ring.claim("a", reference("r"), &call()).unwrap();
        ticket.attach(waiter(1), |_| panic!("not answered"));
        let (replay, fresh) = ring.claim("a", reference("r"), &call()).unwrap();
        assert!(!fresh);
        replay.attach(waiter(2), |_| panic!("not answered"));
        let mut responses = 0;
        ticket.answer(
            Ok(Reply::TorrentsSetUploadLimit(MutationResult::Pending {
                reference: reference("r"),
            })),
            |delivery| {
                if matches!(delivery, Delivery::Response(_, _)) {
                    responses += 1;
                }
            },
        );
        assert_eq!(responses, 2);
        replay.attach(waiter(3), |delivery| {
            assert!(matches!(delivery, Delivery::Response(_, Ok(_))))
        });
        let event = Event::MutationResult {
            seq: 1,
            ts: 1,
            reference: reference("r"),
            op: "torrents.stop".into(),
            outcome: MutationOutcome::Done,
            error: None,
            result: None,
        };
        let mut connections = vec![];
        ticket.finish(event.clone(), |delivery| {
            if let Delivery::Event(connection, _) = delivery {
                connections.push(connection);
            }
        });
        connections.sort();
        assert_eq!(connections, vec![1, 2, 3]);
        let mut order = vec![];
        replay.attach(waiter(4), |delivery| {
            order.push(match delivery {
                Delivery::Response(_, _) => "pending",
                Delivery::Event(_, _) => "event",
            })
        });
        assert_eq!(order, vec!["pending", "event"]);
    }
    #[test]
    fn stale_uses_the_operation_table() {
        let old = Instance::new("0".repeat(32)).unwrap();
        assert!(stale(&call(), Some(&old), &"1".repeat(32)).is_ok());
        let remove = Call::TorrentsRemove {
            args: RemoveArgs {
                hashes: Hashes::new(vec![Hash::new("a".repeat(40)).unwrap()]).unwrap(),
                delete_files: false,
            },
        };
        assert_eq!(
            stale(&remove, Some(&old), &"1".repeat(32))
                .unwrap_err()
                .message,
            "daemon restarted since this request; check the list"
        );
    }
}
