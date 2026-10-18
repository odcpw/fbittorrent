//! Feature-only process cumulative counters. Read after runtime teardown.
//! These counters explain path shape; diagnostic CPU timings are not normal wins.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static RECEIVED_PIECE_FRAMES: AtomicU64 = AtomicU64::new(0);
static COPIED_PIECE_PAYLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
static PIECE_HANDOFF_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static PHYSICAL_PIECE_ENTRIES: AtomicU64 = AtomicU64::new(0);
static BATCH_DEPTH_1: AtomicU64 = AtomicU64::new(0);
static BATCH_DEPTH_2: AtomicU64 = AtomicU64::new(0);
static BATCH_DEPTH_3: AtomicU64 = AtomicU64::new(0);
static BATCH_DEPTH_4: AtomicU64 = AtomicU64::new(0);
static ALL_STORAGE_POOL_SUBMISSIONS: AtomicU64 = AtomicU64::new(0);

static COALESCING_BATCHES: AtomicU64 = AtomicU64::new(0);
static COALESCING_WAITED_BATCHES: AtomicU64 = AtomicU64::new(0);
static COALESCING_PENDING_POLLS: AtomicU64 = AtomicU64::new(0);
static COALESCING_DEADLINE_STOPS: AtomicU64 = AtomicU64::new(0);
static COALESCING_WAIT_NS_TOTAL: AtomicU64 = AtomicU64::new(0);
static COALESCING_WAIT_NS_MAX: AtomicU64 = AtomicU64::new(0);
static COALESCING_INTAKE_NS_MAX: AtomicU64 = AtomicU64::new(0);
static COALESCING_DEADLINE_OVERSHOOT_NS_MAX: AtomicU64 = AtomicU64::new(0);

/// Feature-only wall-clock observation, including a dropped waiting future.
/// Pending witnesses count actual parked attempts; the configured deadline
/// does not bound scheduler delay. No timestamps or guard exist in normal cfg.
pub(crate) struct CoalescingObservation {
    started: Instant,
    waiting_started: Option<Instant>,
    pending_polls: u64,
    deadline_stopped: bool,
    window_ns: u64,
}
impl CoalescingObservation {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            started: Instant::now(),
            waiting_started: None,
            pending_polls: 0,
            deadline_stopped: false,
            window_ns: nanos(window),
        }
    }
    pub(crate) fn pending(&mut self) {
        self.waiting_started.get_or_insert_with(Instant::now);
        self.pending_polls = self.pending_polls.saturating_add(1);
    }
    pub(crate) fn deadline_stop(&mut self) {
        self.deadline_stopped = true;
    }
}
impl Drop for CoalescingObservation {
    fn drop(&mut self) {
        let elapsed_ns = nanos(self.started.elapsed());
        COALESCING_BATCHES.fetch_add(1, Ordering::Relaxed);
        COALESCING_INTAKE_NS_MAX.fetch_max(elapsed_ns, Ordering::Relaxed);
        COALESCING_DEADLINE_OVERSHOOT_NS_MAX
            .fetch_max(elapsed_ns.saturating_sub(self.window_ns), Ordering::Relaxed);
        if let Some(started) = self.waiting_started {
            let wait_ns = nanos(started.elapsed());
            COALESCING_WAITED_BATCHES.fetch_add(1, Ordering::Relaxed);
            COALESCING_PENDING_POLLS.fetch_add(self.pending_polls, Ordering::Relaxed);
            COALESCING_WAIT_NS_TOTAL.fetch_add(wait_ns, Ordering::Relaxed);
            COALESCING_WAIT_NS_MAX.fetch_max(wait_ns, Ordering::Relaxed);
        }
        if self.deadline_stopped {
            COALESCING_DEADLINE_STOPS.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[inline]
pub(crate) fn received_piece_frame() {
    RECEIVED_PIECE_FRAMES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn copied_piece_payload_bytes(bytes: usize) {
    let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    COPIED_PIECE_PAYLOAD_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

#[inline]
pub(crate) fn piece_handoff_attempt(depth: usize) {
    PIECE_HANDOFF_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    let counter = match depth {
        1 => Some(&BATCH_DEPTH_1),
        2 => Some(&BATCH_DEPTH_2),
        3 => Some(&BATCH_DEPTH_3),
        4 => Some(&BATCH_DEPTH_4),
        _ => None,
    };
    if let Some(counter) = counter {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub(crate) fn physical_piece_entry() {
    PHYSICAL_PIECE_ENTRIES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub(crate) fn storage_pool_submission() {
    ALL_STORAGE_POOL_SUBMISSIONS.fetch_add(1, Ordering::Relaxed);
}

/// Process cumulative observation, not an atomic cross-counter transaction.
/// Runtime teardown success must be checked before treating it as terminal.
pub fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "received_piece_frames": RECEIVED_PIECE_FRAMES.load(Ordering::Relaxed),
        "copied_piece_payload_bytes": COPIED_PIECE_PAYLOAD_BYTES.load(Ordering::Relaxed),
        "piece_handoff_attempts": PIECE_HANDOFF_ATTEMPTS.load(Ordering::Relaxed),
        "physical_piece_entries": PHYSICAL_PIECE_ENTRIES.load(Ordering::Relaxed),
        "batch_depth_counts": {
            "1": BATCH_DEPTH_1.load(Ordering::Relaxed),
            "2": BATCH_DEPTH_2.load(Ordering::Relaxed),
            "3": BATCH_DEPTH_3.load(Ordering::Relaxed),
            "4": BATCH_DEPTH_4.load(Ordering::Relaxed)
        },
        "all_storage_pool_submissions": ALL_STORAGE_POOL_SUBMISSIONS.load(Ordering::Relaxed),
        "coalescing_batches": COALESCING_BATCHES.load(Ordering::Relaxed),
        "coalescing_waited_batches": COALESCING_WAITED_BATCHES.load(Ordering::Relaxed),
        "coalescing_pending_polls": COALESCING_PENDING_POLLS.load(Ordering::Relaxed),
        "coalescing_deadline_stops": COALESCING_DEADLINE_STOPS.load(Ordering::Relaxed),
        "coalescing_wait_ns_total": COALESCING_WAIT_NS_TOTAL.load(Ordering::Relaxed),
        "coalescing_wait_ns_max": COALESCING_WAIT_NS_MAX.load(Ordering::Relaxed),
        "coalescing_intake_ns_max": COALESCING_INTAKE_NS_MAX.load(Ordering::Relaxed),
        "coalescing_deadline_overshoot_ns_max": COALESCING_DEADLINE_OVERSHOOT_NS_MAX.load(Ordering::Relaxed)
    })
}
