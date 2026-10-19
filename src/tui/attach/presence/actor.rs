use super::*;
use crate::api::{self, validation::Validate};
use serde_json::json;
use std::{collections::BTreeMap, time::Instant};

struct Pending {
    op: &'static str,
    reference: Option<api::MutationRef>,
    reply: Option<mpsc::SyncSender<Result<Value, String>>>,
    deadline: Instant,
}
pub(super) fn budget(op: &str) -> Duration {
    Duration::from_secs(match op {
        "torrents.add" => 100,
        "torrents.get" | "torrents.files" | "torrents.set_files" => 65,
        _ => 20,
    })
}
fn emit(sender: &mpsc::SyncSender<Update>, update: Update) -> Result<()> {
    sender
        .try_send(update)
        .map_err(|_| anyhow::anyhow!("API update queue overflowed; reconnect for a fresh snapshot"))
}
fn targets(args: &Value) -> Vec<api::Hash> {
    args.get("hashes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(args.get("hash").and_then(Value::as_str))
        .filter_map(|hash| api::Hash::new(hash).ok())
        .collect()
}
pub(super) fn run(
    mut wire: Wire,
    requests: mpsc::Receiver<Request>,
    updates: mpsc::SyncSender<Update>,
    stopping: Arc<AtomicBool>,
    completed: Arc<super::confirmations::Confirmations>,
) -> Result<()> {
    let caps = api::validation::NegotiatedCaps::from_hello(wire.hello());
    let mut pending = BTreeMap::<u64, Pending>::new();
    let mut heartbeat = Instant::now();
    let mut received = Instant::now();
    let result = (|| -> Result<()> {
        while !stopping.load(Ordering::Acquire) {
            while pending.len() < 7 {
                let request = match requests.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                };
                if Instant::now() >= request.deadline {
                    let _ = request.reply.send(Err(
                        "API queue deadline expired before dispatch; request was not sent".into(),
                    ));
                    continue;
                }
                let hashes = targets(&request.args);
                match wire.start(request.op, request.args, request.reference.as_ref()) {
                    Ok(id) => {
                        if let Some(reference) = &request.reference {
                            emit(
                                &updates,
                                Update::Sent {
                                    reference: reference.clone(),
                                    op: request.op,
                                    hashes,
                                },
                            )?;
                        }
                        pending.insert(
                            id,
                            Pending {
                                op: request.op,
                                reference: request.reference,
                                reply: Some(request.reply),
                                deadline: request.deadline,
                            },
                        );
                    }
                    Err(error) => {
                        let fatal = error.downcast_ref::<std::io::Error>().is_some();
                        let _ = request.reply.send(Err(error.to_string()));
                        if fatal {
                            return Err(error);
                        }
                    }
                }
            }
            let expired: Vec<_> = pending
                .iter()
                .filter(|(_, request)| Instant::now() >= request.deadline)
                .map(|(id, _)| *id)
                .collect();
            for id in expired {
                let request = pending.remove(&id).unwrap();
                let error = format!(
                    "API {} timed out; result unknown; check the list",
                    request.op
                );
                if let Some(reference) = request.reference {
                    emit(
                        &updates,
                        Update::Failed {
                            reference,
                            error: error.clone(),
                            unknown: true,
                        },
                    )?;
                }
                if let Some(reply) = request.reply {
                    let _ = reply.send(Err(error));
                } else {
                    bail!("presence heartbeat timed out");
                }
            }
            if heartbeat.elapsed() >= Duration::from_secs(5) && pending.len() < 8 {
                let id = wire.start("app.client", json!({"interactive":true}), None)?;
                pending.insert(
                    id,
                    Pending {
                        op: "app.client",
                        reference: None,
                        reply: None,
                        deadline: Instant::now() + Duration::from_secs(6),
                    },
                );
                heartbeat = Instant::now();
            }
            if received.elapsed() > Duration::from_secs(45) {
                bail!("API silent for 45 seconds");
            }
            let Some(frame) = wire.receive()? else {
                continue;
            };
            received = Instant::now();
            if frame.get("ev").is_some() {
                let event: api::Event = serde_json::from_value(frame)?;
                event.validate()?;
                caps.event(&event)?;
                completed.observe(&event);
                let bye = matches!(event, api::Event::Bye { .. });
                emit(&updates, Update::Event(event))?;
                if bye {
                    return Ok(());
                }
                continue;
            }
            let id = frame
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("API reply has no positive id"))?;
            let Some(request) = pending.remove(&id) else {
                continue;
            }; // A timed-out request can still finish later.
            let result = Wire::result(request.op, &frame);
            if let Ok(value) = &result {
                caps.reply(&api::Reply::decode(request.op, value.clone())?)?;
            }
            if let (Some(reference), Ok(value)) = (&request.reference, &result) {
                if value["state"] == "pending" {
                    anyhow::ensure!(
                        value["ref"].as_str() == Some(reference.as_str()),
                        "API pending reply changed the mutation reference"
                    );
                }
            }
            if let Some(reference) = request.reference {
                match &result {
                    Ok(value) => emit(
                        &updates,
                        Update::Acknowledged {
                            reference,
                            pending: value["state"] == "pending",
                        },
                    )?,
                    Err(error) => {
                        let unknown = error
                            .downcast_ref::<super::super::wire::RemoteError>()
                            .is_none_or(|error| error.0.code == api::ErrorCode::Timeout);
                        emit(
                            &updates,
                            Update::Failed {
                                reference,
                                error: error.to_string(),
                                unknown,
                            },
                        )?;
                    }
                }
            }
            let malformed = result.as_ref().is_err_and(|error| {
                error
                    .downcast_ref::<super::super::wire::RemoteError>()
                    .is_none()
            });
            if let Some(reply) = request.reply {
                let _ = reply.send(result.map_err(|error| error.to_string()));
            } else {
                result?;
            }
            if malformed {
                bail!("invalid API reply; result unknown; check the list");
            }
        }
        Ok(())
    })();
    // Wire's drop shuts down all descriptor clones, including the cancellation handle.
    drop(wire);
    for (_, request) in pending {
        if let Some(reply) = request.reply {
            let _ = reply.send(Err(
                "API connection lost; result unknown; check the list".into()
            ));
        }
    }
    result
}
