//! Exercise the vendored policy through its public downstream API. The
//! published dependency omits test-only source modules, so its own lib-test
//! target cannot serve as the consumer regression.
use super::NativeRuntime;
use asupersync::{
    error::ErrorKind,
    record::{ObligationAbortReason, ObligationKind, ObligationRecord, SourceLocation},
    runtime::{obligation_table::ObligationCreateArgs, ObligationTable},
    types::{ObligationId, Time},
};

fn create(table: &mut ObligationTable, runtime: &NativeRuntime, tick: u64) -> ObligationId {
    table.create(ObligationCreateArgs {
        kind: ObligationKind::IoOp,
        holder: runtime.cx().task_id(),
        region: runtime.cx().region_id(),
        now: Time::from_nanos(tick),
        description: None,
        acquired_at: SourceLocation::unknown(),
        acquire_backtrace: None,
    })
}

fn record(table: &ObligationTable, id: ObligationId) -> Option<&ObligationRecord> {
    table
        .iter()
        .find(|(_, record)| record.id == id)
        .map(|(_, record)| record)
}

#[test]
fn completed_io_history_is_bounded_without_losing_pending_or_leaks() {
    let runtime = NativeRuntime::new().unwrap();
    let mut table = ObligationTable::new();
    table.set_resolved_history_limit(8);
    let pending = create(&mut table, &runtime, 0);
    let leaked = create(&mut table, &runtime, 1);
    table.mark_leaked(leaked, Time::from_nanos(2)).unwrap();
    let mut recent = Vec::new();
    for tick in 3..10_003 {
        let id = create(&mut table, &runtime, tick);
        if tick % 2 == 0 {
            table.commit(id, Time::from_nanos(tick + 1)).unwrap();
        } else {
            table
                .abort(
                    id,
                    Time::from_nanos(tick + 1),
                    ObligationAbortReason::Cancel,
                )
                .unwrap();
        }
        recent.push(id);
        assert!(table.len() <= 10, "successful I/O history grew at {tick}");
        assert_eq!(table.pending_count(), 1);
        assert!(record(&table, pending).unwrap().is_pending());
        assert!(record(&table, leaked).unwrap().is_leaked());
    }
    for id in &recent[recent.len() - 8..] {
        assert!(record(&table, *id).is_some(), "recent history disappeared");
    }
    assert!(record(&table, recent[0]).is_none());
    table.commit(pending, Time::from_nanos(10_004)).unwrap();
    assert_eq!(table.pending_count(), 0);
    assert!(record(&table, leaked).unwrap().is_leaked());
}

#[test]
fn evicted_ids_cannot_settle_a_reused_pending_slot_or_cross_the_region_fence() {
    let runtime = NativeRuntime::new().unwrap();
    let mut table = ObligationTable::new();
    table.set_resolved_history_limit(0);
    let old = create(&mut table, &runtime, 0);
    let old_slot = table
        .iter()
        .find(|(_, record)| record.id == old)
        .unwrap()
        .0
        .index();
    table.commit(old, Time::from_nanos(1)).unwrap();
    let current = create(&mut table, &runtime, 2);
    let current_slot = table
        .iter()
        .find(|(_, record)| record.id == current)
        .unwrap()
        .0
        .index();
    assert_eq!(old_slot, current_slot);
    assert_ne!(old, current);
    assert_eq!(
        table.commit(old, Time::from_nanos(3)).unwrap_err().kind(),
        ErrorKind::ObligationAlreadyResolved
    );
    assert_eq!(
        table
            .abort(old, Time::from_nanos(3), ObligationAbortReason::Cancel)
            .unwrap_err()
            .kind(),
        ErrorKind::ObligationAlreadyResolved
    );
    assert_eq!(table.pending_count(), 1);
    assert!(record(&table, current).unwrap().is_pending());
    let region = runtime.cx().region_id();
    table.mark_region_finalized(region);
    assert_eq!(
        table
            .commit(current, Time::from_nanos(4))
            .unwrap_err()
            .kind(),
        ErrorKind::RegionFinalized
    );
    assert_eq!(
        table
            .abort(current, Time::from_nanos(4), ObligationAbortReason::Cancel)
            .unwrap_err()
            .kind(),
        ErrorKind::RegionFinalized
    );
    assert_eq!(table.pending_count(), 1);
}

#[test]
fn enabling_a_bound_keeps_recent_history_and_preserves_holder_index() {
    let runtime = NativeRuntime::new().unwrap();
    let mut table = ObligationTable::new();
    let pending = create(&mut table, &runtime, 0);
    let mut completed = Vec::new();
    for tick in 1..21 {
        let id = create(&mut table, &runtime, tick);
        table.commit(id, Time::from_nanos(tick + 1)).unwrap();
        completed.push(id);
    }
    assert_eq!(table.len(), 21, "unconfigured tables keep audit history");
    table.set_resolved_history_limit(3);
    assert_eq!(table.len(), 4);
    assert_eq!(
        table.pending_obligation_ids_for_task(runtime.cx().task_id()),
        vec![pending]
    );
    for id in &completed[17..] {
        assert!(record(&table, *id).is_some());
    }
    table.set_resolved_history_limit(1);
    assert_eq!(table.len(), 2);
    assert!(record(&table, completed[19]).is_some());
    table
        .abort(pending, Time::from_nanos(22), ObligationAbortReason::Cancel)
        .unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(table.pending_count(), 0);
}
