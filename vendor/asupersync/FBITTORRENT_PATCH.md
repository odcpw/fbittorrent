# Local Asupersync patch

This is the crates.io Asupersync 0.5.0 package (`asupersync-0.5.0.crate`, sha256
`f34b1a19ffd6b74570339a156912436335bb09c594c675c62ea164b19a1f2511`; upstream
revision `78b64636e99fea4ea2d868096576021dd3b8e519`) plus the fbittorrent patch
below, retained under its original licence: `LICENSE` is byte-identical to the
package's, "MIT License (with OpenAI/Anthropic Rider)", and every distribution
must carry it unmodified. The root `Cargo.toml` selects this tree through
`[patch.crates-io] asupersync = { path = "vendor/asupersync" }` (plan §8.2
option B: the full tree is committed; there is no fork).

## What differs from the package

`diff -r --exclude=.cargo-ok` against the registry copy lists exactly these ten
files (line counts from `diff`, added/removed):

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

## Tests

The published package is not test-complete: `cargo test --lib` on this tree
does not compile, because the package omits test-only modules such as
`src/actor_conformance_tests.rs`. Test the patch using the complete upstream
Git source at `78b64636e99fea4ea2d868096576021dd3b8e519`: overlay the eight
differing Rust files listed above (the metamorphic file already matches),
retain upstream's workspace manifest and lockfile, and add only
`epoch-diagnostics = []` to its `[features]`. Every shared `src/` file must
then be byte-identical to this vendored tree; the licence must match too.

From that prepared source checkout, using the rch cargo shim:

```bash
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN=1.98.0 CARGO_BUILD_JOBS=4
export CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0
export RUSTFLAGS='-C debuginfo=0'
export CARGO_TARGET_DIR=/absolute/path/to/fbittorrent/target/asupersync-patch-tests
cargo test --locked -p asupersync --lib --no-default-features \
  --features proc-macros,native-runtime,tls,tls-webpki-roots,epoch-diagnostics \
  time::wheel -- --nocapture --test-threads=4
cargo test --locked -p asupersync --lib --no-default-features \
  --features proc-macros,native-runtime,tls,tls-webpki-roots,epoch-diagnostics \
  region_owner_tests -- --nocapture --test-threads=4
```

On 2026-10-18 these commands passed **63 wheel tests** (including all four
pruning regressions and the metamorphic tests) and **2 region-owner tests**,
with no failures, ignored tests, or worker-panic scan matches. The upstream
lockfile remained unchanged. Its conformance dev dependency additionally
enables `fuzz`, `metrics`, `test-internals`, and `tracing-integration` in this
test harness. This is scoped patch evidence, not an all-features or full-suite
result: `--all-features` enables the nightly-only feature and
`messaging-fabric`, which excludes `region_owner_tests`.

See `docs/validation/2026-10-18/P0-02.md` for provenance and
`target/test-logs/asupersync/{build,test,fork_test}.jsonl` for the retained
command receipts. The retired standalone wheel runner was not restored.
The refreshed fbittorrent gate also passed 91 app tests, 14 runtime tests
(1 existing throughput test ignored), its background-panic scan, the release
build, and isolated native integration.

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

## Atomic child-region wake registration (2026-10-20)

`src/runtime/spawn_mailbox.rs`: `AdmittedRegionSlot::register` retains the
outcome lock until its waker is inserted, using the same inner -> waiters lock
order as publication. An already-published result releases the lock before
invoking the waker. Previously publication could occur between the empty-result
check and waiter insertion, drain an empty list, and permanently strand the
opening despite successful admission.

The app's real 1,100-region capacity regression reproduced this in 4/50 parallel
runs; phase-timed controls stalled during admission after 2-238 ms of progress,
then hit the unchanged ten-second child fuse. The regression retains 1,100
simultaneous, now explicitly distinct region identities, checks every close and
the two-second runtime shutdown, and reports admission/close timings.

Test: `cargo test --locked -p fbittorrent
native_runtime::tests::more_than_one_thousand_owned_regions_admit_and_close
-- --exact --nocapture`; loop the resulting Cargo-identified test executable
50 times with four parallel processes and other focused suites running. Also
run the native runtime and engine tests for cancellation/close behavior.
