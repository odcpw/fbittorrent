//! Measurement-only epoch violation counters.
//!
//! Enabled only by `epoch-diagnostics`; this feature is absent from defaults.
//! Producers increment fixed relaxed atomics under the tracker's existing
//! publication mutex. No formatting, callbacks, extra tasks or per-event I/O.

use super::epoch_tracker::{
    EpochConsistencyViolation, EpochTelemetryStatistics, EpochTransitionStatistics, ModuleId,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Stable diagnostic array order; these labels are not inferred from stacks.
pub const VIOLATION_KIND_NAMES: [&str; 4] = [
    "module_desync",
    "slow_transition",
    "missing_transition",
    "advancement_order_violation",
];

/// Stable dependent/dependency axes for the order-pair matrices.
pub const MODULE_NAMES: [&str; 7] = [
    "scheduler",
    "region_table",
    "task_table",
    "obligation_table",
    "timer_wheel",
    "io_reactor",
    "cancel_protocol",
];

/// Counts at one recording stage. Order matrices use [dependent][dependency].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpochViolationStageSnapshot {
    /// Actual enum kind counts, in `VIOLATION_KIND_NAMES` order.
    pub by_kind: [u64; 4],
    /// AdvancementOrderViolation counts using `MODULE_NAMES` axes.
    pub order_pairs: [[u64; 7]; 7],
}

/// Cumulative record-path counts since tracker creation or reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpochViolationCountersSnapshot {
    /// Every candidate passed to record_violation, before adjacent dedup.
    /// Public current-health queries are not record candidates.
    pub generated: EpochViolationStageSnapshot,
    /// Candidates suppressed by the existing adjacent-record rule.
    pub adjacent_deduplicated: EpochViolationStageSnapshot,
    /// Actual successful additions to bounded history.
    pub recorded: EpochViolationStageSnapshot,
    /// Actual entries removed from the history front due to its bound.
    pub trimmed: EpochViolationStageSnapshot,
}

/// One callback-free coherent tracker observation for a diagnostic receipt.
#[derive(Debug, Clone)]
pub struct EpochDiagnosticSnapshot {
    /// Counter stages, coherent with the retained history below.
    pub counters: EpochViolationCountersSnapshot,
    /// Owned newest history, in oldest-to-newest order, capped at 1,000.
    /// This snapshot is intended for an explicit post-transfer boundary.
    pub retained: Vec<EpochConsistencyViolation>,
    /// Accepted transition statistics, not violation-generation counts.
    pub transitions: EpochTransitionStatistics,
    /// Bounded outbox statistics. Emitted counts include all receipt types.
    pub telemetry: EpochTelemetryStatistics,
    /// The coordinate system selected for this tracker.
    pub epoch_domain: &'static str,
    /// Whether local sequence and slow-transition checks are enabled.
    pub enabled: bool,
    /// Configured shared-domain ordering policy, not a holder identity check.
    pub configured_strict_ordering: bool,
    /// Whether cross-module numeric comparisons apply in this domain.
    pub cross_module_correlation: bool,
    /// Configured shared-domain numeric skew threshold.
    pub configured_max_epoch_skew: u64,
    /// Existing local slow-transition threshold.
    pub slow_transition_threshold_ns: u64,
}

struct StageCounters {
    by_kind: [AtomicU64; 4],
    order_pairs: [[AtomicU64; 7]; 7],
}

impl StageCounters {
    fn new() -> Self {
        Self {
            by_kind: std::array::from_fn(|_| AtomicU64::new(0)),
            order_pairs: std::array::from_fn(|_| {
                std::array::from_fn(|_| AtomicU64::new(0))
            }),
        }
    }

    fn increment(&self, violation: &EpochConsistencyViolation) {
        let kind = match violation {
            EpochConsistencyViolation::ModuleDesync { .. } => 0,
            EpochConsistencyViolation::SlowTransition { .. } => 1,
            EpochConsistencyViolation::MissingTransition { .. } => 2,
            EpochConsistencyViolation::AdvancementOrderViolation {
                module,
                dependency_module,
                ..
            } => {
                self.order_pairs[module_index(*module)][module_index(*dependency_module)]
                    .fetch_add(1, Ordering::Relaxed);
                3
            }
        };
        self.by_kind[kind].fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> EpochViolationStageSnapshot {
        EpochViolationStageSnapshot {
            by_kind: std::array::from_fn(|kind| self.by_kind[kind].load(Ordering::Relaxed)),
            order_pairs: std::array::from_fn(|dependent| {
                std::array::from_fn(|dependency| {
                    self.order_pairs[dependent][dependency].load(Ordering::Relaxed)
                })
            }),
        }
    }

    fn reset(&self) {
        for counter in &self.by_kind {
            counter.store(0, Ordering::Relaxed);
        }
        for row in &self.order_pairs {
            for counter in row {
                counter.store(0, Ordering::Relaxed);
            }
        }
    }
}

fn module_index(module: ModuleId) -> usize {
    match module {
        ModuleId::Scheduler => 0,
        ModuleId::RegionTable => 1,
        ModuleId::TaskTable => 2,
        ModuleId::ObligationTable => 3,
        ModuleId::TimerWheel => 4,
        ModuleId::IoReactor => 5,
        ModuleId::CancelProtocol => 6,
    }
}

pub(crate) struct EpochViolationCounters {
    generated: StageCounters,
    adjacent_deduplicated: StageCounters,
    recorded: StageCounters,
    trimmed: StageCounters,
}

impl EpochViolationCounters {
    pub(crate) fn new() -> Self {
        Self {
            generated: StageCounters::new(),
            adjacent_deduplicated: StageCounters::new(),
            recorded: StageCounters::new(),
            trimmed: StageCounters::new(),
        }
    }

    pub(crate) fn generated(&self, violation: &EpochConsistencyViolation) {
        self.generated.increment(violation);
    }

    pub(crate) fn adjacent_deduplicated(&self, violation: &EpochConsistencyViolation) {
        self.adjacent_deduplicated.increment(violation);
    }

    pub(crate) fn recorded(&self, violation: &EpochConsistencyViolation) {
        self.recorded.increment(violation);
    }

    pub(crate) fn trimmed(&self, violation: &EpochConsistencyViolation) {
        self.trimmed.increment(violation);
    }

    pub(crate) fn snapshot(&self) -> EpochViolationCountersSnapshot {
        EpochViolationCountersSnapshot {
            generated: self.generated.snapshot(),
            adjacent_deduplicated: self.adjacent_deduplicated.snapshot(),
            recorded: self.recorded.snapshot(),
            trimmed: self.trimmed.snapshot(),
        }
    }

    pub(crate) fn reset(&self) {
        self.generated.reset();
        self.adjacent_deduplicated.reset();
        self.recorded.reset();
        self.trimmed.reset();
    }
}
