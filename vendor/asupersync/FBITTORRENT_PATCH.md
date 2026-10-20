# fbittorrent's Asupersync fork

This is the crates.io Asupersync 0.5.0 package (`asupersync-0.5.0.crate`, sha256
`f34b1a19ffd6b74570339a156912436335bb09c594c675c62ea164b19a1f2511`; upstream
revision `78b64636e99fea4ea2d868096576021dd3b8e519`) with the fbittorrent changes
below, retained under its original licence: `LICENSE` is byte-identical to the
package's, "MIT License (with OpenAI/Anthropic Rider)", and every distribution
must carry it unmodified. The root `Cargo.toml` selects this tree through
`[patch.crates-io] asupersync = { path = "vendor/asupersync" }`. This is now a
private native-runtime fork, not an API-compatible distribution of upstream.

## Native build scope (2026-10-20)

The app and rqbit enable `native-runtime`, `tls`, `tls-native-roots` and
`tls-webpki-roots`; `epoch-diagnostics` remains an optional measurement feature.
ATP, native QUIC and HTTP/3 were being compiled even with that feature set.
The first cut gated them behind `extended-network`; the subsequent pruning
removed those unused protocol trees, their Lab helpers and the ATP journal-tag
wrapper. Private ATP encoding/decoding and QUIC UDP helpers remain available to
unit tests. Retained implementations are unchanged.
Shared decode variants and outcome fields retain their layout and computation;
their existing conditional dead-code allowances also cover profiles without
ATP consumers. Native runtime,
cancellation, TCP/UDP, TLS and the HTTP client retain their implementations.

Lab is behind `lab-runtime`, enabled by a dev dependency for rqbit-runtime's existing
`lab_pause_drains_a_channel_waiter_and_releases_its_resources` test. That test
exercises region cancellation and resource release through Lab. The release profile omits
Lab, its reactor, chaos channels and RaptorQ regression harness. Their source
and the cancellation test remain available; the test bodies are unchanged.
Lab-only exact-dispatch and state-command wrappers follow that boundary too.
The native checkpoint acknowledgement keeps its fields and computation; its
two Lab-only observations have conditional dead-code allowances when Lab is off.

The unused gRPC, messaging and process APIs, adapter certification, agent-swarm
and audit tooling, plan/conformance/session modules, and Lab's swarm-replay and
deadlock-radar tools were first gated behind `extended-services`, then removed.
The CLI/doctor binary, database drivers, OTLP adapters and their private regex
implementation are also removed. None was used by the app or its consumer
test suites.

The manifest exposes only the native/TLS features above, `epoch-diagnostics`
and `lab-runtime`, with empty defaults. The retired optional dependencies and
upstream binary/example targets are removed. Remaining inactive upstream cfg
fragments in shared files are explicitly recognized by `check-cfg`, but are not
selectable Cargo features. This avoids changing shared runtime logic merely to
remove an inactive branch. The obsolete upstream manifest is removed.

Generic server primitives, HTTP/1 and HTTP/2, and the actor frameworks remain:
native TLS RPC and retained Lab tests share some of their internal machinery.
Removing those safely needs a separate dependency split. Native platform
backends and existing tests for retained behavior are preserved.

## Original runtime patches

These are the original local runtime changes, before the later additions
documented below (the line counts describe those original patches):

| File | Change |
|---|---|
| `src/time/wheel.rs` | +204: `next_deadline` retires cancelled entries from ready storage and occupied wheel buckets before the deadline calculation (`prune_inactive_entries`); live ID/generation checks, ordering, coalescing, wheel wrap and lazy overflow-heap cleanup are preserved; four new tests (`next_deadline_prunes_*`, `next_deadline_pruning_preserves_*`) and the module hook for the restored metamorphic tests. |
| `src/time/wheel_metamorphic_tests.rs` | new, 626 lines: the upstream metamorphic tests the crates.io package left out, restored unchanged. |
| `Cargo.toml` | +2: the measurement-only `epoch-diagnostics` feature, absent from defaults. |
| `src/runtime/epoch_diagnostics.rs` | new, 196 lines: fixed epoch-kind counters and bounded observations (`epoch-diagnostics` only). |
| `src/runtime/epoch_tracker.rs` | +118/-10: `EpochConsistencyTracker::with_independent_module_sequences` (and its time-getter variant), a `VecDeque` violations store, `diagnostic_snapshot`. |
| `src/runtime/mod.rs` | +3: declares `epoch_diagnostics` behind the feature. |
| `src/runtime/builder.rs` | +28: `epoch_diagnostic_snapshot` on the runtime and its handle (`epoch-diagnostics` only). |
| `src/runtime/state.rs` | +12/-1: the runtime state uses `with_independent_module_sequences` unconditionally (the table cursors count independent mutation streams); `epoch_diagnostic_snapshot` behind the feature. |
| `src/cx/cx.rs` | +67: `RegionOwnerKey` and `Cx::reusable_region_owner_key()`, which the rqbit fork calls (`vendor/rqbit/librqbit/src/spawn_utils.rs`); without them the build fails. |
| `src/cx/region_owner_tests.rs` | new, 52 lines: two tests for the owner key (compiled only without `messaging-fabric`). |

## Why

Profiling found repeated scans of cancelled network timeouts dominating the
app's active-download CPU (the wheel change); the epoch diagnostics measured
the epoch tracker's cost; the region-owner key lets the engine reuse an
operation owner without extending its budget or authority. The summaries of
that work are in `docs/validation/2026-10-11/cpu-optimization/README.md`,
`docs/validation/2026-10-12/cpu-optimization/README.md` and
`docs/validation/2026-10-12/cpu-root-cause/README.md` (the receipt data itself
was deleted on 2026-10-18, owner decision Q4).

## Validation

Run from an isolated fbittorrent worktree with private `XDG_*` and temporary
directories, using the rch cargo shim:

```bash
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS=4
export CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0
cargo check --locked
cargo test --locked --workspace --no-default-features --lib --bins -- --test-threads=4
```

The workspace test command disables member defaults because the SHA wrapper's
standalone default selects a different backend from fbittorrent. The app's
dependency features still select its shipping backend. Library and binary
suites exclude archived upstream Tokio examples.

The published Asupersync package is not test-complete: it omits test-only
modules such as `src/actor_conformance_tests.rs`, and it is not a workspace
member. The commands above test the app and rqbit consumers, not a complete
upstream Asupersync suite.

Historical patch validation on 2026-10-18 used the complete upstream Git source
at the revision above: **63 wheel tests** (including the pruning regressions
and metamorphic tests) and **2 region-owner tests** passed. Its conformance dev
dependency also enabled `fuzz`, `metrics`, `test-internals` and
`tracing-integration`. That result was scoped patch evidence, not an
all-features or full-suite result. See `docs/validation/2026-10-18/P0-02.md`
for its provenance.

When upgrading Asupersync, check whether upstream includes these changes and
rerun the timer, transfer and shutdown checks before removing the patch.

## Bounded deadline snapshots (fbt-50y, 2026-10-19)

`src/observability/task_inspector.rs` adds `SnapshotLimits`, `BoundedSnapshot`
and `TaskInspector::bounded_snapshot`. Existing inspector and diagnostics methods
are unchanged. The additive query uses only try-locks for the unified state,
external task and obligation tables (B -> A -> C), and task checkpoint contexts.
Contended or poisoned state is returned as partial JSON with omission reasons.
Requested/owning regions and their ancestors use immutable IDs/parents and atomic
state; the query never takes RegionInner locks or scans obligations.

Caller limits bound task and region counts, encoded bytes and collection time;
additional ceilings are 128 tasks, 64 regions, 64 KiB, 5 ms and 4096 arena slots.
The arena ceiling also bounds iterator work across vacant slots. Checkpoint text
is copied at a UTF-8 boundary, up to 256 bytes, without cloning checkpoint history
or unbounded cancellation strings. A byte budget smaller than fixed metadata
returns an empty payload with `partial=true`. Omitted regions and every exceeded
ceiling are explicit. This is a best-effort snapshot, never proof of quiescence.

Why: native-runtime shutdown previously called complete, blocking inspection
queries after its deadline. A contended runtime lock or large snapshot could
therefore extend shutdown indefinitely. The application now calls this API on
an observer worker with a separate hard caller fuse, and emits through a bounded
nonblocking stderr worker. Error propagation does not depend on either worker.

Tests: `cargo test --locked -p asupersync --lib bounded_snapshot -- --nocapture`
where the complete upstream test tree is available; the new unit control holds
the actual unified-state, task-table and obligation-table locks in turn and
requires partial snapshots within the time cap. The published package's omitted
test sources still prevent its standalone lib tests (see Tests above). The
application fallback is `cargo test --locked -p fbittorrent
native_runtime::shutdown_tests:: -- --nocapture`: real native teardown controls,
a saturated blocking stderr pipe, socket-backed stderr, and 256 actual task records with a held
CxInner lock verify bounded partial collection, count/byte ceilings and phase
error preservation. Evidence and exact commands are in
`docs/validation/2026-10-18/P1-03c.md` and `target/test-logs/fbt-50y/`.

The bounded query also accepts an optional caller-sampled timestamp in the
runtime's clock domain; otherwise it uses the stored logical time. It never
calls an arbitrary timer/TimeSource implementation while holding state locks.
The held-task application control arms a trap clock to verify this property;
the native observer samples the concrete process-monotonic WallClock outside
the collection. Socket stderr uses MSG_DONTWAIT rather than reopening procfs.

## Structured-region capacity (2026-10-20)

The swarm governor's default 1,000-region cap rejected the 500th real active
transfer (each transfer has multiple owned regions), even with modest memory
usage. The default count cap is now zero (unlimited); an explicitly configured
nonzero cap and the existing pressure/envelope checks still apply. No task is
detached and cancellation/region close semantics are unchanged. Application
regression: `native_runtime::tests::more_than_one_thousand_owned_regions_admit_and_close`
opens and closes 1,100 regions on the actual native runtime. The loopback engine
benchmark additionally requires verified progress from all 500 active torrents.
