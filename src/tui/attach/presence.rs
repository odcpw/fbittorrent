//! One bounded, multiplexed API connection for the TUI.
use super::wire::Wire;
use anyhow::{bail, Result};
use serde_json::Value;
use std::{
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
mod actor;
mod confirmations;

pub(crate) enum Update {
    Event(crate::api::Event),
    Sent {
        reference: crate::api::MutationRef,
        op: &'static str,
        hashes: Vec<crate::api::Hash>,
    },
    Acknowledged {
        reference: crate::api::MutationRef,
        pending: bool,
    },
    Failed {
        reference: crate::api::MutationRef,
        error: String,
        unknown: bool,
    },
    Closed(String),
}
struct Request {
    op: &'static str,
    args: Value,
    reference: Option<crate::api::MutationRef>,
    reply: mpsc::SyncSender<Result<Value, String>>,
    deadline: Instant,
}
/// All transport work runs off the UI thread. Queues and in-flight requests are bounded.
pub(crate) struct Presence {
    pub(in crate::tui) hello: crate::api::HelloResult,
    pub(in crate::tui) id: u64,
    confirmations: Arc<confirmations::Confirmations>,
    commands: mpsc::SyncSender<Request>,
    updates: Mutex<mpsc::Receiver<Update>>,
    closed: Arc<AtomicBool>,
    reason: Arc<Mutex<String>>,
    interrupt: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Presence {
    pub(in crate::tui) fn connect() -> Result<Arc<Self>> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let wire = Wire::connect()?;
        let interrupt = wire.interrupt()?;
        let hello = wire.hello().clone();
        let (commands, requests) = mpsc::sync_channel::<Request>(8);
        let (events, updates) = mpsc::sync_channel::<Update>(128);
        let closed = Arc::new(AtomicBool::new(false));
        let stopping = closed.clone();
        let reason = Arc::new(Mutex::new(String::new()));
        let failure = reason.clone();
        let confirmations = Arc::new(confirmations::Confirmations::default());
        let completed = confirmations.clone();
        let thread = std::thread::Builder::new()
            .name("fbittorrent-tui-api".into())
            .spawn(move || {
                let keep_updates_open = events.clone();
                let error = actor::run(wire, requests, events, stopping.clone(), completed.clone())
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "API connection closed".into());
                *failure.lock().unwrap_or_else(|error| error.into_inner()) = error;
                stopping.store(true, Ordering::Release);
                completed.closed();
                drop(keep_updates_open);
            })?;
        Ok(Arc::new(Self {
            hello,
            id: (1 << 48) + NEXT.fetch_add(1, Ordering::Relaxed),
            confirmations,
            commands,
            updates: Mutex::new(updates),
            closed,
            reason,
            interrupt,
            thread: Some(thread),
        }))
    }
    pub(in crate::tui) fn request(&self, op: &'static str, args: Value) -> Result<Value> {
        if !self.alive() {
            bail!("fbittorrent stopped");
        }
        let reference = if crate::api::ops::TABLE
            .iter()
            .any(|entry| entry.name == op && entry.mutating)
        {
            static REFERENCES: AtomicU64 = AtomicU64::new(1);
            let sequence = REFERENCES
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| anyhow::anyhow!("mutation reference sequence exhausted"))?;
            Some(
                crate::api::MutationRef::new(format!("tui-{}-{sequence}", std::process::id()))
                    .map_err(anyhow::Error::msg)?,
            )
        } else {
            None
        };
        Wire::encode(&args)?; // Bound queued payloads before they enter the worker.
        let (reply, result) = mpsc::sync_channel(1);
        self.commands
            .try_send(Request {
                op,
                args,
                reference,
                reply,
                deadline: Instant::now() + actor::budget(op),
            })
            .map_err(|_| anyhow::anyhow!("API connection busy or closed"))?;
        result
            .recv_timeout(actor::budget(op) + Duration::from_secs(1))
            .map_err(|error| {
                anyhow::anyhow!(
                    "API request {}; result unknown; check the list",
                    match error {
                        mpsc::RecvTimeoutError::Timeout => "timed out",
                        mpsc::RecvTimeoutError::Disconnected => "lost its connection",
                    }
                )
            })?
            .map_err(anyhow::Error::msg)
    }
    pub(in crate::tui) fn request_confirmed(&self, op: &'static str, args: Value) -> Result<Value> {
        let deadline = Instant::now() + actor::budget(op);
        let reply = self.request(op, args)?;
        if reply["state"] != "pending" {
            return Ok(reply);
        }
        let reference = crate::api::MutationRef::new(
            reply["ref"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("pending reply lacks ref"))?,
        )
        .map_err(anyhow::Error::msg)?;
        self.confirmations.wait(&reference, (reply["saving"] != true).then_some(deadline), &self.closed)
    }
    pub(in crate::tui) fn next_updates(&self) -> Result<Option<Vec<Update>>> {
        let result=receive_batch(&self.updates.lock().unwrap_or_else(|error|error.into_inner()));
        match result {
            Ok(updates)=>Ok(Some(updates)),
            Err(mpsc::RecvTimeoutError::Timeout) if self.alive()=>Ok(None),
            Err(_)=>bail!("{}",self.reason.lock().unwrap_or_else(|error|error.into_inner())),
        }
    }
    pub(in crate::tui) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.interrupt.shutdown(Shutdown::Both);
        self.confirmations.closed();
    }
    pub(in crate::tui) fn alive(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
}
impl Drop for Presence {
    fn drop(&mut self) {
        self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// A single pending ftui message owns at most 16 updates. Draining already-ready
// frames avoids a render-loop wakeup/ack round trip for each snapshot fragment.
fn receive_batch(receiver:&mpsc::Receiver<Update>)->std::result::Result<Vec<Update>,mpsc::RecvTimeoutError> {
    let first=receiver.recv_timeout(Duration::from_millis(100))?;
    let mut batch=Vec::with_capacity(16);
    batch.push(first);
    batch.extend(receiver.try_iter().take(15));
    Ok(batch)
}
#[cfg(test)]
mod batch_tests {
    use super::*;
    #[test]
    fn snapshot_backlog_drains_in_bounded_ordered_batches() {
        let (sender,receiver)=mpsc::sync_channel(128);
        for n in 0..36 {sender.send(Update::Closed(n.to_string())).unwrap();}
        drop(sender);
        let mut seen=Vec::new();
        for count in [16,16,4] {
            let batch=receive_batch(&receiver).unwrap();assert_eq!(batch.len(),count);
            for row in batch {let Update::Closed(text)=row else{panic!("unexpected update")};seen.push(text);}
        }
        assert_eq!(seen,(0..36).map(|n|n.to_string()).collect::<Vec<_>>());
        assert!(matches!(receive_batch(&receiver),Err(mpsc::RecvTimeoutError::Disconnected)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_updates_keep_order_and_stay_bounded_before_the_next_ack() {
        let (sender, receiver) = mpsc::sync_channel(128);
        for i in 0..80 {
            sender
                .send(Update::Acknowledged {
                    reference: crate::api::MutationRef::new(format!("burst-{i}")).unwrap(),
                    pending: false,
                })
                .unwrap();
        }
        let mut seen = Vec::new();
        for length in [16, 16, 16, 16, 16] {
            let batch = receive_batch(&receiver).unwrap();
            assert_eq!(batch.len(), length);
            for update in batch {
                let Update::Acknowledged { reference, .. } = update else {
                    panic!("wrong update")
                };
                seen.push(reference.to_string());
            }
        }
        assert_eq!(
            seen,
            (0..80).map(|i| format!("burst-{i}")).collect::<Vec<_>>()
        );
        drop(sender);
        assert!(matches!(
            receive_batch(&receiver),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }
}
