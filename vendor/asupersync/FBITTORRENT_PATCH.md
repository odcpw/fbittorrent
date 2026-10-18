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
