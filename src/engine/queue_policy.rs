//! Pure queue admission and ledger order changes; no runtime or filesystem I/O.
use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Stats {
    pub paused: bool,
    pub finished: bool,
    pub error: bool,
    pub retry: bool,
    pub rechecking: bool,
    /// A reorder protects existing transfers until their slot is released.
    pub pinned: bool,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Top,
    Up,
    Down,
    Bottom,
}

pub(super) fn positioned(r: &Record) -> bool {
    !r.paused && r.completion_on <= 0 && !r.force && !r.staged
}
#[cfg(test)]
pub(super) fn position(records: &[Record], hash: &str) -> Option<u64> {
    records
        .iter()
        .filter(|r| positioned(r))
        .position(|r| r.hash == hash)
        .map(|i| i as u64 + 1)
}

pub(super) fn admit(records: &[Record], stats: &[Option<Stats>], limit: usize) -> Vec<bool> {
    assert_eq!(records.len(), stats.len());
    let mut run = vec![false; records.len()];
    let mut slots = 0;
    // Moving a queued row cannot preempt transfers already occupying slots.
    // Normal resume/FIFO admission remains unchanged outside a reordering.
    for (i, (record, stats)) in records.iter().zip(stats).enumerate() {
        if let Some(s) = stats {
            if !record.paused
                && !record.force
                && !record.staged
                && s.pinned
                && !s.paused
                && !s.finished
                && !s.error
                && !s.rechecking
                && slots < limit
            {
                run[i] = true;
                slots += 1;
            }
        }
    }
    for (i, (record, stats)) in records.iter().zip(stats).enumerate() {
        let Some(s) = stats else {
            continue;
        };
        if record.paused || record.staged || s.rechecking {
            continue;
        }
        if s.error && !s.retry {
            continue;
        } else if s.finished {
            run[i] = !record.stop_on_complete();
        } else if record.force {
            run[i] = true;
        } else if !run[i] && slots < limit {
            run[i] = true;
            slots += 1;
        }
    }
    run
}

pub(super) fn move_record(ledger: &mut Ledger, hash: &str, to: Direction) -> Result<u64> {
    if !ledger.records.iter().any(|r| r.hash == hash) {
        return Err(super::recheck::MissingTorrent(hash.into()).into());
    }
    let indices: Vec<_> = ledger
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| positioned(r))
        .map(|(i, _)| i)
        .collect();
    let current = indices
        .iter()
        .position(|&i| ledger.records[i].hash == hash)
        .ok_or_else(|| InvalidTorrentState {
            hash: hash.into(),
            reason: "no_queue_position",
        })?;
    let target = match to {
        Direction::Top => 0,
        Direction::Bottom => indices.len() - 1,
        Direction::Up => current.saturating_sub(1),
        Direction::Down => (current + 1).min(indices.len() - 1),
    };
    if target < current {
        for i in (target + 1..=current).rev() {
            ledger.records.swap(indices[i], indices[i - 1]);
        }
    } else {
        for i in current..target {
            ledger.records.swap(indices[i], indices[i + 1]);
        }
    }
    Ok(target as u64 + 1)
}

pub(super) fn validate_force(ledger: &Ledger, hashes: &[String], on: bool) -> Result<()> {
    for hash in hashes {
        let record = ledger
            .records
            .iter()
            .find(|r| &r.hash == hash)
            .context("torrent record not found")?;
        let reason = if record.staged {
            Some("staged")
        } else if on && record.completion_on > 0 && record.stop_on_complete() {
            Some("complete_no_seed")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(InvalidTorrentState {
                hash: hash.clone(),
                reason,
            }
            .into());
        }
    }
    Ok(())
}
