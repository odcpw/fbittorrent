//! Deterministic lab runtime for testing.
//!
//! The lab runtime provides:
//!
//! - Virtual time (no wall-clock dependencies)
//! - Deterministic scheduling (same seed → same execution)
//! - Trace capture and replay
//! - Schedule exploration (DPOR-style)
//! - Test oracles for invariant verification
//! - Await point tracking for cancellation injection
//! - Integrated cancellation injection with oracle verification
//! - Chaos testing with configurable failure injection
//!
//! # Quick Start
//!
//! [`LabConfig`] makes the seed explicit, while [`LabRuntime`] reports
//! quiescence and oracle results without depending on wall time.
//!
//! <!-- core-api-doctest: lab-runtime-config -->
//! ```
//! use asupersync::{Budget, Cx, LabConfig, LabRuntime, Outcome, main};
//! use asupersync::record::ObligationKind;
//! use asupersync::record::region::RegionState;
//!
//! #[main]
//! async fn main(cx: &Cx) {
//!     cx.checkpoint().expect("example starts active");
//!
//!     let mut empty = LabRuntime::new(LabConfig::new(7));
//!     let empty_report = empty.run_until_quiescent_with_report();
//!     assert!(empty_report.quiescent);
//!
//!     let mut lab = LabRuntime::new(LabConfig::new(7).panic_on_leak(false));
//!     let region = lab.state.create_root_region(Budget::INFINITE);
//!     let (task, _handle) = lab
//!         .state
//!         .create_task(region, Budget::INFINITE, async {})
//!         .expect("create lab task");
//!     lab.state
//!         .create_obligation(
//!             ObligationKind::SendPermit,
//!             task,
//!             region,
//!             Some("deliberate doctest leak".to_string()),
//!         )
//!         .expect("create obligation");
//!     lab.state
//!         .update_task(task, |record| record.complete(Outcome::Ok(())))
//!         .expect("complete task");
//!     lab.state
//!         .region(region)
//!         .expect("region exists")
//!         .set_state(RegionState::Closed);
//!
//!     let report = lab.report();
//!     let leak = report
//!         .oracle_report
//!         .entry("obligation_leak")
//!         .expect("leak oracle is registered");
//!     assert!(!leak.passed, "the lab must report the deliberate leak");
//! }
//! ```
//!
//! # Chaos Testing
//!
//! Enable chaos injection to stress-test error handling:
//!
//! ```ignore
//! // Light chaos for CI (1% cancel, 5% delay)
//! let config = LabConfig::new(42).with_light_chaos();
//! let mut runtime = LabRuntime::new(config);
//!
//! // ... run tests ...
//!
//! // Check injection statistics
//! let stats = runtime.chaos_stats();
//! println!("Injections: {} delays, {} cancellations", stats.delays, stats.cancellations);
//! ```
//!
//! See the [`chaos`] module for detailed documentation on chaos testing.

#[cfg(feature = "benchmark-adapters")]
pub mod benchmark_cartel;
pub mod chaos;
pub mod config;
pub mod conformal;
pub mod crashpack;
pub mod dual_run;
pub mod exploration_budget;
pub mod explorer;
#[cfg(feature = "benchmark-adapters")]
pub mod forensics;
pub mod fuzz;
pub mod http;
pub mod injection;
pub mod instrumented_future;
pub mod ldfi;
pub mod ldfi_trace;
pub mod meta;
pub mod network;
pub mod numa;
pub mod opportunity;
pub mod oracle;
pub mod replay;
#[cfg(feature = "benchmark-adapters")]
pub mod replay_minimization;
pub mod runtime;
pub mod scenario;
pub mod scenario_runner;
pub mod snapshot_restore;
pub mod spork_harness;
pub mod util;
pub mod virtual_time_wheel;

#[cfg(all(test, feature = "legacy-internal-test-harnesses"))]
mod deterministic_validation_tests;

pub use crate::util::{
    StrictEntropyGuard, disable_strict_entropy, enable_strict_entropy, strict_entropy_enabled,
};
pub use config::LabConfig;
pub use conformal::{
    CalibrationReport, ConformalCalibrator, ConformalConfig, ConformityScore, CoverageTracker,
    PredictionSet,
};
pub use crashpack::{
    ATP_CRASHPACK_SCHEMA_VERSION, AtpCrashpack, AtpEvidenceLedger, AtpOracleResult,
    AtpTransferOracle, AtpTransferState, CrashpackBuilder, CrashpackError, ReplayError,
    TraceMinimizer, TraceMinimizerConfig, TransferOracle, TransferOracleResult, TransferState,
    TransferViolation, ViolationSeverity,
};
pub use dual_run::{
    CancelTerminalPhase, CancellationRecord, CaptureAnnotation, CaptureManifest, ComparisonVerdict,
    CounterTolerance, DUAL_RUN_SCHEMA_VERSION, DrainStatus, DualRunHarness, DualRunResult,
    DualRunScenarioIdentity, ExecutionInstanceId, FieldObservability, LiveExecutionProfile,
    LiveRunMetadata, LiveRunResult, LiveRunnerConfig, LiveWitnessCollector, LoserDrainRecord,
    NORMALIZED_OBSERVABLE_SCHEMA_VERSION, NormalizedObservable, NormalizedSemantics,
    ObligationBalanceRecord, OutcomeClass, Phase, PromotedExplorationScenario,
    PromotedFuzzScenario, RegionCloseRecord, RegionState as DualRunRegionState, ReplayMetadata,
    ReplayPolicy, ResourceSurfaceRecord, RuntimeKind, ScenarioFamilyId, SeedLineageRecord,
    SeedMode, SeedPlan, SemanticMismatch, TerminalOutcome, assert_dual_run_passes,
    assert_semantics, capture_cancellation, capture_loser_drain, capture_obligation_balance,
    capture_region_close, capture_terminal_from_result, capture_terminal_outcome,
    check_core_invariants, compare_observables, normalize_lab_observable, normalize_lab_report,
    normalize_live_observable, promote_exploration_report, promote_fuzz_finding,
    promote_regression_case, promote_regression_corpus, run_live_adapter,
};
pub use exploration_budget::{
    ExplorationBudget, ExplorationBudgetAssumptions, ExplorationBudgetConfig,
    ExplorationBudgetEstimate,
};
pub use explorer::{
    CoverageMetrics, DporCoverageMetrics, DporExplorer, ExplorationReport, ExplorerConfig,
    RunResult, ScheduleExplorer, TopologyExplorer, ViolationReport,
};
pub use fuzz::{
    FuzzConfig, FuzzFinding, FuzzHarness, FuzzRegressionCase, FuzzRegressionCorpus, FuzzReport,
    fuzz_quick,
};
pub use http::{
    RequestBuilder, RequestTrace, TestHarness, TraceEntry, VirtualClient, VirtualServer,
};
pub use injection::{
    LabBuilder, LabInjectionConfig, LabInjectionReport, LabInjectionResult, LabInjectionRunner, lab,
};
pub use instrumented_future::{
    AwaitPoint, CancellationInjector, InjectionMode, InjectionOutcome, InjectionReport,
    InjectionResult, InjectionRunner, InjectionStrategy, InstrumentedFuture,
    InstrumentedPollResult,
};
pub use meta::{
    ALL_ORACLE_INVARIANTS, BuiltinMutation, MetaCoverageEntry, MetaCoverageReport, MetaReport,
    MetaResult, MetaRunner, builtin_mutations, invariant_from_violation,
};
pub use network::{
    DeterministicNetwork, Fault as NetworkFault, JitterModel, LatencyModel, NetworkConditions,
    NetworkConfig, NetworkMetrics, NetworkTraceEvent, NetworkTraceKind, Packet,
};
pub use numa::{
    NumaCachePressureInput, NumaCachePressureProjection, NumaPressureClass,
    project_numa_cache_pressure,
};
pub use oracle::{
    ActorLeakOracle, ActorLeakViolation, BayesFactor, DetectionModel, DeterminismOracle,
    DeterminismSourceHint, DeterminismViolation, DownOrderOracle, DownOrderViolation, EProcess,
    EProcessConfig, EProcessMonitor, EValue, EvidenceEntry, EvidenceLedger, EvidenceLine,
    EvidenceStrength, EvidenceSummary, FinalizerId, FinalizerOracle, FinalizerViolation,
    LogLikelihoodContributions, LoserDrainOracle, LoserDrainViolation, MailboxOracle,
    MailboxViolation, MailboxViolationKind, MonitorResult, ORACLE_ALL, ORACLE_DESCRIPTORS,
    ObligationLeakOracle, ObligationLeakViolation, Oracle, OracleDescriptor, OracleEntryReport,
    OracleRegistry, OracleRegistryError, OracleReport, OracleStats, OracleSuite, OracleViolation,
    QuiescenceOracle, QuiescenceViolation, RegistryLeaseOracle, RegistryLeaseViolation,
    ReplyLinearityOracle, ReplyLinearityViolation, SupervisionOracle, SupervisionViolation,
    SupervisionViolationKind, SupervisorQuiescenceOracle, SupervisorQuiescenceViolation,
    TaskLeakOracle, TaskLeakViolation, TraceEventSummary, assert_deterministic,
    assert_deterministic_for_seeds, assert_deterministic_multi,
};
#[cfg(feature = "messaging-fabric")]
pub use oracle::{
    FabricPublishOracle, FabricPublishViolation, FabricQuiescenceOracle, FabricQuiescenceViolation,
    FabricRedeliveryOracle, FabricRedeliveryViolation, FabricReplyOracle, FabricReplyViolation,
};
pub use replay::{
    ExplorationFingerprintClass as ReplayExplorationFingerprintClass,
    ExplorationReport as ReplayExplorationReport,
    ExplorationRunSummary as ReplayExplorationRunSummary, NormalizationResult, ReplayValidation,
    SporkExplorationReport, SporkExplorationRunSummary, TraceDivergence, TraceSummary,
    classify_fingerprint_classes, compare_normalized, explore_scenario_runner_seed_space,
    explore_seed_space, explore_spork_seed_space, normalize_for_replay,
    normalize_for_replay_with_config, summarize_spork_reports, traces_equivalent,
};
pub use runtime::{
    AutoAdvanceTermination, HarnessAttachmentKind, HarnessAttachmentRef, LabAutoCrashpack,
    LabAutoCrashpackError, LabConfigSummary, LabRunReport, LabRuntime, LabTraceCertificateSummary,
    SporkHarnessReport, VirtualTimeReport, lab_test_config_from_env, lab_test_seed_override,
    run_async_lab_test_with_config, run_async_under_lab, run_async_under_lab_with_config,
};
pub use scenario::{
    CancellationSection, CancellationStrategy, ChaosSection, FaultAction, FaultEvent, IncludeRef,
    LabSection, LatencySpec, LinkConditions, NetworkPreset, NetworkSection, Participant,
    SCENARIO_SCHEMA_VERSION, Scenario, ValidationError as ScenarioValidationError,
};
pub use scenario_runner::{
    ExplorationRunSummary, FilteredOracleReport, ScenarioExplorationResult, ScenarioRunResult,
    ScenarioRunner, ScenarioRunnerError as FrankenLabRunnerError, TraceCertificateSnapshot,
};
pub use snapshot_restore::{
    IncrementalSnapshot, RestorableSnapshot, RestoreError, SNAPSHOT_ARTIFACT_MAGIC,
    SNAPSHOT_ARTIFACT_VERSION, SnapshotArtifact, SnapshotArtifactKind, SnapshotCodecError,
    SnapshotLimits, SnapshotRestore, SnapshotStats, ValidationResult,
};
pub use spork_harness::{
    HarnessError, ScenarioRunnerError, SporkAppHarness, SporkScenarioConfig, SporkScenarioResult,
    SporkScenarioRunner, SporkScenarioSpec,
};
pub use util::{
    StackTraceConfig, capture_stack_trace, capture_stack_trace_default, capture_stack_trace_depth,
    capture_stack_trace_minimal,
};
pub use virtual_time_wheel::{ExpiredTimer, VirtualTimerHandle, VirtualTimerWheel};
