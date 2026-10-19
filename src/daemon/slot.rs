//! Replaceable engine state; the daemon owns the listener, lock and lifecycle.
use crate::{engine::{AppEngine, ParkedLedger}, engine::ops::{EngineOp, EngineResult, HashSelection}};
use anyhow::{bail, Result};

pub enum Slot {
    Starting,
    Running(AppEngine),
    Parked(ParkedLedger),
    Stopping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    Running,
    Parked,
    Stopping,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Parked => "parked",
            Self::Stopping => "stopping",
        }
    }
}

impl Slot {
    pub fn state(&self) -> State {
        match self {
            Self::Starting => State::Starting,
            Self::Running(_) => State::Running,
            Self::Parked(_) => State::Parked,
            Self::Stopping => State::Stopping,
        }
    }
}

pub struct EngineSlot {
    pub slot: Slot,
    pub engine_epoch: u64,
}

fn transition_epoch(from: State, to: State, epoch: u64) -> Result<u64> {
    use State::*;
    match (from, to) {
        (Starting, Running | Parked) => Ok(epoch.max(1)),
        (Running, Starting | Parked) | (Parked, Starting | Running) => Ok(epoch + 1),
        (Starting | Running | Parked, Stopping) => Ok(epoch),
        _ => bail!(
            "invalid engine slot transition {} -> {}",
            from.name(),
            to.name()
        ),
    }
}

impl EngineSlot {
    pub fn new() -> Self {
        Self {
            slot: Slot::Starting,
            engine_epoch: 0,
        }
    }

    /// Return the old engine so its bounded drain happens outside the slot lock.
    pub fn replace(&mut self, next: Slot) -> Result<Slot> {
        self.engine_epoch = transition_epoch(self.slot.state(), next.state(), self.engine_epoch)?;
        Ok(std::mem::replace(&mut self.slot, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parked_adapter_retains_typed_generation() {
        let directory = std::env::temp_dir().join(format!("fbt-parked-adapter-{}-{}",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&directory).unwrap();
        let ledger = ParkedLedger::new(directory.clone(), crate::daemon::lifecycle::Lifecycle::default());
        let generation = ledger.ledger_generation();
        for expected in 1..=2 {
            ledger.execute(EngineOp::Stop { hashes: HashSelection::All }).unwrap();
            let EngineResult::Torrents(rows) = ledger.execute(EngineOp::Info).unwrap() else { panic!("expected torrent list") };
            assert!(rows.is_empty());
            assert_eq!(generation.wait_after(expected - 1, std::time::Duration::ZERO), Some(expected));
            println!("parked adapter durable save/read: expected generation={expected} observed={}", generation.current());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn transitions_table() {
        use State::*;
        for from in [Starting, Running, Parked, Stopping] {
            for to in [Starting, Running, Parked, Stopping] {
                let expected = match (from, to) {
                    (Starting, Running | Parked | Stopping) => Some(7),
                    (Running, Starting | Parked) | (Parked, Starting | Running) => Some(8),
                    (Running | Parked, Stopping) => Some(7),
                    _ => None,
                };
                let observed = transition_epoch(from, to, 7);
                println!(
                    "slot {} -> {}: expect={expected:?} observed={observed:?}",
                    from.name(),
                    to.name()
                );
                assert_eq!(observed.ok(), expected);
            }
        }
        for path in [
            vec![
                (Starting, 0),
                (Running, 1),
                (Starting, 2),
                (Running, 2),
                (Parked, 3),
                (Running, 4),
                (Stopping, 4),
            ],
            vec![
                (Starting, 0),
                (Parked, 1),
                (Starting, 2),
                (Running, 2),
                (Stopping, 2),
            ],
            vec![(Starting, 0), (Stopping, 0)],
        ] {
            for pair in path.windows(2) {
                assert_eq!(
                    transition_epoch(pair[0].0, pair[1].0, pair[0].1).unwrap(),
                    pair[1].1
                );
            }
            println!("slot lifetime {path:?}");
        }
    }
}
