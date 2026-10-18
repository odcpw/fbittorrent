# Persistent measurement helpers

Run these only after the root agent releases the matching source/build or
measurement slot. They preserve receipts here or in a caller-selected evidence
directory; no helper installs a binary or controls another user's job.

```sh
python3 docs/validation/2026-10-12/cpu-root-cause/helpers/source_manifest.py --output <source.json>

python3 docs/validation/2026-10-12/cpu-root-cause/helpers/freeze_build.py \
  --label <candidate> --binary <preserved-stripped-binary> \
  --receipt <build.json> --log <build.log>
# Optional: --features epoch-diagnostics; this identifies a diagnostic build.
# Optional: --target-dir <isolated-cargo-target> --symbols <symbol-binary>

python3 docs/validation/2026-10-12/cpu-root-cause/helpers/host_qualify.py inventory \
  --output <workers.json>
python3 docs/validation/2026-10-12/cpu-root-cause/helpers/host_qualify.py qualify \
  --inventory <workers.json> --output <qualification.json> --samples <host.jsonl>
```

The source manifest covers Cargo.toml, Cargo.lock and every Rust/TOML file under
src and vendor. Build receipts compare the complete maps before and after an
offline, locked Rust 1.98.0 build with two jobs and release-perf. Inherited
RUSTFLAGS, CARGO_ENCODED_RUSTFLAGS and Cargo environment rustflag overrides are
removed. This proves the enumerated
files did not differ at those boundaries; source edits must also stay frozen
by coordination. Registry inputs, Cargo configuration and other assets are
outside that map. The build preserves a symbol copy and copies executable mode
before stripping a separate 0755 artifact. Existing evidence paths are refused.

Inventory capture is separate from qualification. It selects the identified CAD
workers by all three argv conditions: argv0 ends in `/build123d-mcp/bin/python`,
argv1 is exactly `-c`, and argv2 starts with
`from multiprocessing.spawn import spawn_main;`. It records PID, start_ticks,
cwd and argv. Qualification refuses an inventory with another selector. Capture
fresh identities after a daemon restart. The summed CPU gate uses these fixed
CAD worker identities throughout both windows.
Qualification requires two complete 30-second windows. **Each** must have
background residual at most 22% across all CPUs, fixed worker CPU sum at most
5% of one core, no external build/index job, consistent accounting, and unchanged
PID/start identities. Threshold options can tighten those limits but cannot
relax them. Failed windows are retained and return exit 75.

Continue to use the original regression runner
`agent_ergonomics_audit/audit/regression_tests/run_engine_benchmark_monitored.py`
with 30-second monitoring windows, a 25% residual limit and two consecutive busy
full windows to abort. The fixed worker limit remains 5% of one core. Monitoring
and qualification use the original engine_benchmark_host accounting. Neither
stage makes the host perfectly quiet or guarantees observation of short jobs.
