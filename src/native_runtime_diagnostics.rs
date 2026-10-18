//! Deadline-only observations. These handles retain state, not RuntimeInner:
//! keeping a Runtime/RuntimeHandle here would itself prevent final teardown.
use asupersync::{
    observability::{
        diagnostics::{Diagnostics, Reason},
        task_inspector::{TaskInspector, TaskInspectorConfig},
    },
    runtime::{blocking_pool::BlockingPoolHandle, Runtime},
    types::{Budget, RegionId},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

pub(super) struct ShutdownDiagnostics {
    tasks: TaskInspector,
    regions: Diagnostics,
    blocking_pool: Option<BlockingPoolHandle>,
    root: RegionId,
    application: RegionId,
}

impl ShutdownDiagnostics {
    pub(super) fn new(runtime: &Runtime, application: RegionId) -> Self {
        Self {
            tasks: runtime.task_inspector(TaskInspectorConfig::default()),
            regions: runtime.diagnostics(),
            blocking_pool: runtime.blocking_handle(),
            root: runtime.request_cx_with_budget(Budget::INFINITE).region_id(),
            application,
        }
    }

    fn snapshot(&self) -> Value {
        let mut tasks = self.tasks.list_active_tasks();
        tasks.sort_by_key(|task| task.id);
        let mut pending = BTreeSet::from([self.root, self.application]);
        pending.extend(tasks.iter().map(|task| task.region_id));
        let mut visited = BTreeSet::new();
        let mut checkpoints = BTreeMap::new();
        let mut regions = Vec::new();
        while let Some(id) = pending.pop_first() {
            if !visited.insert(id) {
                continue;
            }
            let explanation = self.regions.explain_region_open(id);
            let mut reasons = Vec::new();
            for reason in explanation.reasons {
                match &reason {
                    Reason::ChildRegionOpen { child_id, .. } => {
                        pending.insert(*child_id);
                    }
                    Reason::TaskRunning {
                        task_id,
                        last_checkpoint_message,
                        ..
                    } => {
                        checkpoints.insert(*task_id, last_checkpoint_message.clone());
                    }
                    _ => {}
                }
                reasons.push(reason.to_string());
            }
            regions.push(json!({
                "id": format!("{id:?}"),
                "name": if id == self.application { "native_application" }
                    else if id == self.root { "native_runtime_root" } else { "child_region" },
                "state": explanation.region_state.map(|state| format!("{state:?}")),
                "reasons": reasons,
            }));
        }
        // Runtime queries return copied data and release their locks before
        // serialization or logging. Checkpoints are labels only when supplied
        // by the task; never guess a subsystem or spawn site from a numeric ID.
        json!({
            "regions": regions,
            "tasks": tasks.into_iter().map(|task| json!({
                "id": format!("{:?}", task.id),
                "region": format!("{:?}", task.region_id),
                "state": format!("{:?}", task.state),
                "phase": format!("{:?}", task.phase),
                "age_ms": task.age.as_millis(),
                "polls": task.poll_count,
                "checkpoint": checkpoints.remove(&task.id).flatten(),
                "obligations": task.obligations.iter().map(|id| format!("{id:?}")).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "blocking_pool": self.blocking_pool.as_ref().map(|pool| json!({
                "name": "native_blocking_pool", "busy_threads": pool.busy_threads(),
                "pending_jobs": pool.pending_count(), "active_threads": pool.active_threads(),
            })),
            "runtime_ownership": "not exposed by runtime API",
        })
    }

    pub(super) fn failure(&self, message: &str, began: Instant) -> String {
        let snapshot = self.snapshot();
        let detail = format!(
            "{message} elapsed_ms={} outstanding={snapshot}",
            began.elapsed().as_millis()
        );
        // JSON escapes checkpoint/cancellation text, including embedded newlines.
        eprintln!("ERROR engine {detail}");
        detail
    }
}
