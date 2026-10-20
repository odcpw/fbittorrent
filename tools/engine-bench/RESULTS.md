# Engine request batching: 2026-10-20

The candidate change reduces allocation and socket-I/O overhead. It is held
for review, not qualified for release. It does **not**
establish an end-to-end throughput or CPU improvement on this shared host.
Full engine qualification remains red: both control and candidate fail two of
three Btrfs shutdown drains, and both reject the 500-active-torrent workload.
Those failures were retained, not averaged into performance results or waived.

The initial Phase-1 baseline and metadata-choke finding are in `BASELINE.md`.
The actual comparison control includes main `93cea91` and choke fix `341dc4e`,
at source `923bf45`. Upstream hardening, typed views, sampler and metainfo-cache
changes therefore cannot be credited to request batching. The assigned Opus
worktree/branch was preserved and moved outside the main checkout to
`/home/oliver/Projects/odcpw/fbt-astra-engine-perf` for RCH source isolation.

All retained evidence lives outside Cargo targets at
`/home/oliver/Projects/odcpw/fbt-engine-evidence-_d8rk7f7/`:

- Control SHA-256: `6ee92b60df0828429cda34dc148b8ad1ce162df81b884a10cd410e09047759a4`.
- Candidate SHA-256: `d076b78a594a73c4b91dd33fae2a43efbbdd18d5ff857faeb70fc12616742441`.
- Seed SHA-256: `124359d60d4d28208f4d5482c31cf274cacf01399f8ec6210f0afdbc14d93593`.
- `control-build.json`, `candidate-build.json`, `candidate.patch`, `bin/` identify the exact builds.
- `paired-order/`, `paired-summary.json`, `diagnostics-summary.json`, `*-matrix/`
  retain measurements, process/IO counters, load/pressure, wire observations and teardown results.

Both binaries used `release-perf`, with the shipping optimization settings and
retained symbols. Builds used RCH and the shared flock/systemd memory wrapper.
No build/test ran from the main checkout. No hosted CI was used.

## Measurements

Intel Core Ultra 7 258V, eight cores, shared Linux host. Decimal MB/GB;
RSS below is MiB unless specified. Normal transfer trials alternated AB/BA,
three warmups plus ten measured pairs per seed count. Each torrent is 1 GiB.
Load ranged 5.1–18.7 during measured pairs. Downloads use private tmpfs, so
these numbers describe an engine ceiling, not physical-disk throughput.
Timings include admission/discovery and verified completion, excluding fixture
generation and independent post-run verification. API observations poll every
20 ms plus response latency; admission/tick phase is visible in the spread.

Median (minimum–maximum), ten measured runs per cell:

| (a) Seeds / binary | MB/s | Daemon CPU s/GB | Peak RSS MiB |
|---|---:|---:|---:|
| 1 / control | 364 (251–433) | 2.81 (2.18–3.23) | 22.71 (22.63–22.89) |
| 1 / candidate | 342 (197–518) | 2.70 (2.13–3.78) | 22.73 (22.47–22.88) |
| 4 / control | 581 (288–1,131) | 2.24 (1.59–3.28) | 23.04 (22.77–23.23) |
| 4 / candidate | 667 (364–916) | 2.44 (1.83–3.28) | 23.00 (22.75–23.25) |
| 16 / control | 517 (385–819) | 2.62 (2.29–3.24) | 24.80 (24.36–25.12) |
| 16 / candidate | 448 (223–713) | 2.51 (2.09–4.16) | 24.84 (24.60–24.98) |

Normal `/proc` read/write-call reductions are **18.2%** for four seeds and
**15.3%** for sixteen (median paired reductions). The single-seed estimate is
inconclusive. Exploratory paired bootstrap intervals for all throughput/CPU
changes include zero; `analyze_pairs.py` retains the calculation and fixed seed.
Peak transfer RSS is effectively unchanged.

Separate instrumented runs, three repetitions per cell; median (min–max):

| Seeds | Allocator calls/MB, control → candidate | All syscalls/MB, control → candidate |
|---|---:|---:|
| 1 | 976.8 (963.1–985.8) → 897.3 (882.3–907.1) | 452.3 (450.2–454.0) → 402.1 (382.0–402.5) |
| 4 | 915.2 (913.2–917.3) → 811.5 (805.8–820.6) | 398.3 (396.4–400.2) → 271.7 (269.3–274.0) |
| 16 | 1,111.5 (1,109.4–1,118.3) → 1,034.4 (1,027.6–1,045.6) | 404.3 (404.0–404.4) → 307.2 (306.2–307.2) |

Allocator calls fall about **8.1%, 11.3%, 6.9%**. Strace counts include owned
helper execs and alter scheduling/batch occupancy; their times are not used for
speed claims. The normal-run read/write counts above are the primary evidence
for untraced I/O savings. Socket `writev` medians per GiB fall from
65,541/65,553/65,601 to 42,144/20,680/32,109. Every actual socket I/O retains its
interface-binding checks. Payload `pwritev` remains 65,536/GiB.

Ancillary workloads: three runs each, performed in separate control/candidate
blocks. Control load was 11.3–26.0; candidate load was 4.8–9.3. These descriptive
before/after values cannot establish causal timing/CPU gains.

| Workload | Control median (min–max) | Candidate median (min–max) |
|---|---:|---:|
| (b) Initial hash check, MB/s | 747 (734–864) | 907 (896–973) |
| (b) Explicit recheck, MB/s | 951 (747–961) | 971 (871–991) |
| (d) Magnet → first verified piece, s | 1.050 (1.030–1.055) | 1.038 (0.107–1.049) |
| (e) Verified playable prefix, app ready, s | 1.995 (1.032–2.005) | 1.020 (1.012–1.987) |
| (e) Verified contiguous 8 MiB prefix, s | 1.079 (0.164–1.155) | 0.095 (0.090–1.060) |

Initial-check CPU medians are 1.127→1.062 s/GB; explicit recheck 1.043→1.006.
Initial-check peak RSS medians are 16.64→16.82 MiB, explicit recheck 16.82→16.97.
No shared caches were flushed; these rechecks are warm-cache measurements.
Streaming overlays a real deterministic Matroska prefix and decodes the verified
prefix with ffmpeg, as well as observing the app's playable-ready state.

| (c) Workload | CPU ms/torrent in 20 s, control → candidate (range) | Amortized RSS KiB/torrent, control → candidate (range) |
|---|---:|---:|
| 100 idle | 0.30 (0.20–0.30) → 0.20 (0.10–0.20) | 208.4 (207.9–209.2) → 210.5 (209.6–211.5) |
| 500 idle | 0.12 (0.10–0.14) → 0.10 (0.08–0.10) | 66.4 (66.1–68.2) → 70.2 (69.0–71.3) |
| 100 active | 8.90 (7.90–9.20) → 6.30 (6.30–6.60) | 382.0 (379.8–383.3) → 384.6 (384.0–387.4) |
| 400 active | 7.30 (6.63–7.85) → 6.18 (6.00–7.45) | 238.0 (237.6–247.5) → 244.7 (238.4–246.0) |

Active torrents each advance verified bytes within the window; queued/paused
torrents cannot count as active. Both binaries reject 500 active torrents with
`child region mint failed: resource pressure prevents reg`. That row is a
capacity failure, not a successful performance measurement.

## Change and behaviour proof

The profiled opportunity was queued request writes: 65k socket writes/GiB and
their binding-check syscalls. Impact 3 × confidence 4 / effort 3 = 4.
The writer serializes at most sixteen already-queued Request frames into its
existing buffer, without waiting for more. The first non-request is retained
and processed next. It never batches across control/policy/disconnect messages.

Ordering and tie-breaking are preserved: requests remain FIFO; the picker and
file priorities are untouched. Cancellation checkpoints remain between bounded
batches, and `write_all` still performs the normal bound-socket checks. Upload
guards, payload storage, hashes and durability are unchanged. Batching adds no floating-point or RNG operations and changes no application
seeds; it only groups serialized writes.

Three focused native-runtime tests decode actual protocol frames and check wire
fields/order, bounded work, control barriers, closed queues and immediate empty
queue handling. Command: `cargo test --locked -p fbittorrent -p librqbit
--no-default-features --features librqbit/rust-tls --lib request_batch -- --nocapture`.
The compiler required `RUST_MIN_STACK=16777216`; benchmark daemons did not inherit
that override. `request-batch-tests.log` records three passes.

All 78 paired cases (including warmups), 42 ancillary cases and 36 instrumented
cases passed their assertions and ordered drain/ownership audits. Complete
payload hashes match across all 127 successful transfer cases (`golden-integrity.json`).
All 508 retained piece-map observations advertise first/last priority; the
current runner also asserts this on every fetched map. Single-peer wire traces
also prove those requests precede sequential middle requests. An initial control
warmup exposed that cross-peer arrival order reflects OS scheduling, so that
invalid assertion was replaced with these behaviour checks; the failed attempt
is retained under `paired/` and `paired.log`.

The hostile-peer proof sent a Request while choked: zero uploaded payload and
zero Unchoke, complete payload and clean drain. Three real magnet transfers per
binary fetched metadata over the wire and stayed choked. The separate safety
fix `341dc4e` corrected metadata-bootstrap Unchokes found in Phase 1; it is
present in both performance binaries. The private user/network-namespace
`vpn.headless_park_unpark` case passed interface loss/return and saved-intent
checks (`vpn-proof/`), without changing a host interface.

The unmodified `bash scripts/gate-fast.sh` could not complete because of
artifact transport. Continuing its exact workflow registry separately also
remained **FAIL**, not a successful full gate. Its Rust stage passed 689 tests with
one ignored. RCH initially delivered no test executable; that exact Cargo-named
artifact was copied from the worker and SHA-256 verified. Later build wrappers
ended on SIGTERM after successful remote builds/artifact retrieval, leaving
unacknowledged source leases. One local retry was stopped, the owned receipts
were recovered, and a watcher prevented a subsequent retry from compiling
locally. No other worktree's processes were signalled. The gate/orchestration
source was not edited. `gate-fast-first/`, `gate-fast-interrupted/`,
`gate-fast-final-attempt/`, delivery receipts and `rch-recover-*.log` retain the
failed attempts. The exact fast workflow registry is exercised separately with
the hash-verified debug binary and test validator. Its 17 cases finished in
72.3 seconds: **15 PASS, 2 FAIL**, retained in `gate-workflows/summary.json`.

- `zero_upload`: all payload-permission checks passed, but the first teardown
  needed SIGKILL. The isolated retry passed; the safety failure remains FAIL.
  The control release binary passed this case, which does not resolve the
  candidate's first debug/concurrent teardown failure.
- `setup_idempotent`: setup returned a closed-backend error, then the harness
  lacked the expected actions result. The unchanged control reproduces it
  (`control-workflow-failures/`).
- Native detach/quit, CLI down, idle rules and the real plugin power case passed.

All owned RCH receipts are acknowledged after same-ID recovery. No benchmark
or workflow binary remained running at the final process audit. The forced
cleanup above is still a failed safety check despite removing the process.

## Remaining limits and ranked work

**Btrfs qualification fails.** Of three fresh private 1 GiB / four-seed runs per
binary, only one per binary drained cleanly. The others report a clean-resume
snapshot timeout with a busy native blocking worker and fail runtime drain.
Logs point to the resume/storage flush path; they do not establish a specific
kernel syscall as the root cause. Full I/O pressure reached 63% during this
series. Failed transfer timings are excluded. No process survived any completed
or failed case, and no shutdown deadline or warning assertion was relaxed.
See `boundary-status.json`, `disk-repeat-status.json` and their per-case logs.

What did not pay: request batching did not demonstrate a throughput or CPU
gain in normal paired runs. A SHA-1 SIMD rewrite was not attempted because the
profile already samples the hardware SHA-1 implementation. Single-seed normal
read/write-call improvement also remains inconclusive.

Reprofiling captured 2,331 control and 1,500 candidate user-PC samples across
three single-seed runs each, with no lost samples/errors. Hardware SHA-1 accounts
for 27.5%→32.6%, cancellation checkpoints 2.8%→3.9%, and piece-picker scanning
2.4%→3.0%. These are exclusive percentages from instrumented runs, not CPU-time
speedup estimates. Stripped libc symbol attribution was not used to select work.

1. Repair the bounded clean-resume/storage drain under real disk pressure, with
   data integrity and stop-rule proofs. This is a qualification blocker.
2. Measure piece-level disk-write coalescing/read reuse: payload writes still
   cost 65,536 `pwritev` calls/GiB. Preserve verification and durable resume.
3. Reduce per-torrent runtime-region cost before raising capacity: 500 active
   torrents hit admission limits despite modest process RSS. Keep owned-task
   cancellation and drain guarantees.
4. Profile hashing/recheck buffering and picker bitmap scanning on 4 GiB
   workloads; retain hardware SHA-1, first/last priority and sequential order.

Rollback for batching is a normal revert of its single optimization commit;
the independent metadata-choke fix and measurement tooling remain separable.
