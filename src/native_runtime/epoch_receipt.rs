//! Measurement-only serialization, performed outside runtime/tracker locks.
use super::*;
use asupersync::runtime::{epoch_diagnostics::*, epoch_tracker::EpochConsistencyViolation};

pub(super) fn capture(shared: &Arc<RwLock<Option<Runtime>>>, boundary: &str) -> Option<serde_json::Value> {
    let snapshot = {
        let locked = shared.read().unwrap();
        locked.as_ref()?.epoch_diagnostic_snapshot()
    };
    let mut retained = [0u64; 4];
    let mut sample: [Option<String>; 4] = std::array::from_fn(|_| None);
    for value in &snapshot.retained {
        let kind = match value {
            EpochConsistencyViolation::ModuleDesync { .. } => 0,
            EpochConsistencyViolation::SlowTransition { .. } => 1,
            EpochConsistencyViolation::MissingTransition { .. } => 2,
            EpochConsistencyViolation::AdvancementOrderViolation { .. } => 3,
        };
        retained[kind] += 1;
        if sample[kind].is_none() { sample[kind] = Some(format!("{value:?}")); }
    }
    let stage = |value: EpochViolationStageSnapshot| serde_json::json!({
        "by_kind": value.by_kind, "order_pairs": value.order_pairs,
    });
    let mut transitions = std::collections::BTreeMap::new();
    for (module, stats) in &snapshot.transitions.per_module_stats {
        transitions.insert(format!("{module:?}"), serde_json::json!({
            "current_local_sequence": stats.current_epoch.as_u64(),
            "transition_count": stats.transition_count,
            "last_transition_time_ns": stats.last_transition_time.as_nanos(),
        }));
    }
    Some(serde_json::json!({
        "boundary": boundary,
        "kind_axes": VIOLATION_KIND_NAMES, "module_axes": MODULE_NAMES,
        "epoch_domain": snapshot.epoch_domain,
        "enabled": snapshot.enabled,
        "cross_module_correlation": snapshot.cross_module_correlation,
        "configured_strict_ordering": snapshot.configured_strict_ordering,
        "configured_max_epoch_skew": snapshot.configured_max_epoch_skew,
        "slow_transition_threshold_ns": snapshot.slow_transition_threshold_ns,
        "generated": stage(snapshot.counters.generated),
        "adjacent_deduplicated": stage(snapshot.counters.adjacent_deduplicated),
        "recorded": stage(snapshot.counters.recorded),
        "trimmed": stage(snapshot.counters.trimmed),
        "retained_by_kind": retained, "retained_count": snapshot.retained.len(),
        "first_retained_sample_by_kind": sample,
        "transitions": transitions,
        "total_transitions": snapshot.transitions.total_transitions,
        "telemetry": {
            "pending": snapshot.telemetry.pending,
            "capacity": snapshot.telemetry.capacity,
            "overflow": snapshot.telemetry.overflow_count,
            "dispatch_panics": snapshot.telemetry.dispatch_panic_count,
            "emitted_all_receipts": snapshot.telemetry.emitted_count,
        },
        "limit": "measurement-only counters and bounded history; before_close includes initialization/transfer/completed-idle, after_close adds app-region cleanup; runtime teardown follows; telemetry emissions may race callback delivery and are not generated-violation counts",
    }))
}
