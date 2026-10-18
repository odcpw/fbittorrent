# Download CPU optimization — 11 October 2026

**The timer fix is implemented, benchmarked and installed.** The installed binary
is exactly the timer-only candidate measured here. The measured median reduction
in downloading CPU work is **28.3% for 128 MiB**, and **39.1% for 512 MiB**.
The native engine retains its RAM savings and stops when the app closes.
It still uses more downloading CPU than qBittorrent in this fixture.

## What was fixed

Network operations create timeouts and cancel them when data arrives. Asupersync
kept cancelled timer entries behind live timers, repeatedly scanning them while
looking for the next deadline. A repeat profile attributed **52.84% of sampled
user CPU** to `TimerWheel::next_deadline`.

The timer wheel now removes inactive entries from ready storage and occupied
buckets before the existing deadline calculation. Live ID/generation checks,
live order, coalescing and lazy overflow cleanup remain in place. This is the
only implementation change to the retained Asupersync 0.5.0 source. Its license,
upstream revision and original-file hashes are preserved; the Cargo registry is
unchanged. See [the patch](timer-wheel.patch) and
[the local provenance note](../../../../vendor/asupersync/FBITTORRENT_PATCH.md).

A follow-up profile put the same function at **2.38% of sampled user CPU**.
That profile overlapped external CAD work, so it establishes the changed hot
path, not the final speed or CPU saving. The matched runs below provide that
measurement.

A separate experiment yielded the peer loops every 16 messages. Four matched
blocks showed only **1.28%** median CPU improvement, with individual changes from
−0.64% to 4.33% and slightly higher peak PSS. It was rejected and fully reverted.
[Its measurements](yield-accounted.json) and [diff](peer-yield.patch) are retained
as rejected-candidate evidence. The installed binary contains only the timer fix.

## Standard comparison: 128 MiB

Twenty matched blocks, each containing original native, fixed native, and
qBittorrent with the same TUI. One warmup per variant is excluded. The six
orders rotate; eighteen blocks cover them equally and two repeat the first two.
Native-versus-qBittorrent relative order is balanced ten each way.

All values below are medians. **CPU seconds measure processor work**, not elapsed
time: one CPU second is one core busy for one second. PSS counts shared memory
proportionally across the measured app processes.

| Metric | Original native | Fixed native | qBittorrent + TUI |
|---|---:|---:|---:|
| Download CPU work | 4.385 CPU-s | **3.145 CPU-s** | 0.770 CPU-s |
| User / system CPU | 3.255 / 1.125 CPU-s | 2.090 / 1.060 CPU-s | 0.470 / 0.310 CPU-s |
| Download CPU, one core equivalent | 105.8% | **76.0%** | 12.3% |
| Peak app PSS | 34.55 MiB | **33.19 MiB** | 72.95 MiB |
| Peak app RSS | 37.35 MiB | 35.98 MiB | 89.76 MiB |
| Peak threads | 9 | 9 | 17 |
| Add to completion | 4.137 s | 4.133 s | 6.415 s |
| Actual payload transfer | 4.000 s | 4.000 s | 4.000 s |
| First payload after add | 54.6 ms | 53.6 ms | 1,772.5 ms |
| Open app | 33.7 ms | 33.6 ms | 296.6 ms |
| Status CLI | 5.22 ms | 4.61 ms | 4.82 ms |
| Reopen and verify cached file | 170.1 ms | 169.1 ms | 310.9 ms |
| Close TUI | 15.63 ms | 7.46 ms | 7.29 ms |
| Stop app and engine | 15.98 ms | 7.96 ms | 26.13 ms |
| App PSS after TUI closes | **0 MiB** | **0 MiB** | 46.88 MiB |

The fixed native engine uses **54.5% less peak app RAM than qBittorrent + TUI**.
The original/native comparison gives 28.3% CPU reduction from the two medians;
median per-block reduction is **28.38%**, with a descriptive bootstrap 95%
interval **27.53–29.27%**. All twenty blocks improved: **25.64–30.89%**.
The fixed build still uses **4.08 times** qBittorrent's median CPU work.

CPU work was stable: sample coefficient of variation was 1.47% original,
1.55% fixed, and 9.12% qBittorrent. Download elapsed variation was 0.76%, 0.17%,
and 9.43%. Fixed native download p95 was 4.146 s, CPU-work p95 3.20 CPU-s,
and peak-PSS p95 33.38 MiB. Opening/status/closing are short OS-scheduled
operations with greater relative variation. qBittorrent's cached reopen is
bimodal, with p95 1,721 ms; those slower runs remain in the dataset.

Empty and completed idle measurements have **three samples per variant**,
at blocks 1, 11 and 20, rather than twenty. Empty app PSS medians were
29.05 / 29.47 / 63.05 MiB; completed-idle PSS was 34.73 / 33.21 / 71.08 MiB.
The fixed build's empty PSS rose 0.42 MiB while active peak PSS fell 1.36 MiB.
Fixed idle CPU was approximately 0.4% of one core in both states.
Status CLI has sixty calls per variant, three per measured run.

The fixture explicitly stops its isolated qBittorrent process after observing
what remains when the TUI closes. That cleanup is included in “stop app and
engine”; ordinary legacy TUI closure leaves that daemon running. Every native
close and reclose left zero measured RAM, threads and CPU counters, and removed
its IPC socket.

[Raw runs](results.json), [computed distributions](summary.json),
[host fingerprint](final-host.json), [run receipt](final-run.json).

## Longer-transfer check: 512 MiB

Three separate matched blocks plus three excluded warmups. A roughly 16-second
payload crosses the 10-second read/write timeout horizon and exercises continued
cancellation of longer outgoing-channel timeouts. This checks a concrete
accumulation risk beyond the four-second standard transfer.

| Metric, median | Original native | Fixed native | qBittorrent + TUI |
|---|---:|---:|---:|
| Download CPU work | 20.79 CPU-s | **12.67 CPU-s** | 2.04 CPU-s |
| CPU work per MiB | 0.04061 CPU-s | **0.02475 CPU-s** | 0.00398 CPU-s |
| Peak app PSS | 42.78 MiB | **41.00 MiB** | 79.84 MiB |
| Add to completion | 16.359 s | 16.111 s | 21.608 s |
| Actual payload transfer | 16.227 s | 16.000 s | 19.519 s |
| Reopen and verify cached file | 548.5 ms | 545.0 ms | 343.7 ms |

Median CPU work fell **39.1%**; per-block reductions were 38.50%, 38.62%,
and 39.29%. Fixed native CPU work per MiB is almost unchanged from the 128 MiB
case, while the original costs more per MiB in this longer scenario.
This supports cleanup of accumulated cancelled timers. It does not prove
indefinite growth or isolate every source of the remaining CPU cost.

Three blocks are a descriptive duration check. Their first three permutation
orders do not balance every variant's position; they do not support a reliable
confidence interval or general throughput claim. The qBittorrent payload tail
was slower in all three measured runs. The fixture does not establish its cause.

[Raw longer runs](scaling-512.json), [distributions](scaling-summary.json),
[host fingerprint](scaling-host.json), [run receipt](scaling-run.json).

## Host conditions and reproducibility

The user asked to wait for unrelated indexing rather than measure alongside it.
No indexing, compilation or active CAD worker was detected during either final
series. The desktop itself remained active, so this is a **comparison on the
same host with ordinary desktop activity**, not an idle-host result.

An early attempt aborted at residual background estimates of 20.47% and 20.17%
across eight logical CPUs. Independent review established that 20% lay near the
ordinary desktop floor. Before restarting, [the fresh protocol](final-protocol.json)
required a fixed 60-second preflight mean ≤22%, no external heavy/CAD activity,
and an abort after two consecutive 30-second residual windows above 25%.
Identified CAD worker CPU above 5% of one core also stops the fixture.
The interrupted attempt is excluded rather than retroactively accepted.

The standard preflight mean was 20.54%; the completed series' residual mean was
20.22%, range 19.24–21.14%. The longer preflight mean was 20.04%; its series mean
was 18.79%, range 17.32–20.06%. Maximum CPU stall time was 2.14% / 2.54%; maximum
full-memory stall was 0.072% / 0.075%. Neither series aborted or detected external
heavy work. Every interval and timing outlier is retained.

The monitor records self plus reaped-child CPU over the unique live owned
process family. Its wrapper stays alive and reaps the benchmark before the final
snapshot, preserving short-lived fixture work. A real CPU-consuming child smoke
and independent source review checked this correction. The relevant fields and
transfer on reap are documented by the
[Linux proc implementation](https://raw.githubusercontent.com/torvalds/linux/master/fs/proc/array.c)
and [reap implementation](https://raw.githubusercontent.com/torvalds/linux/master/kernel/exit.c).
Procfs snapshots are not atomic; the residual includes IRQ/softirq work and
should not be treated as exact external-process attribution. IRQ levels were
similar in preflight and the early aborted attempt; their origin is unproven.

Interrupted `yield-pilot.json`, `yield-clean.json` and
`final-threshold-interrupted-results.json` are excluded. The first encountered
real CAD activity; the second also exposed the earlier monitor's failure to
retain CPU from reaped fixture processes. Their raw records remain available.

Both final series used isolated XDG state and fresh tmpfs destinations, the same
actual IMDb cache, a private loopback TCP torrent, 256 KiB pieces, encryption off,
sequential/first-last behavior, and a common 32 MiB/s seed. Full payload SHA256
and matched joined seed receipts passed for **all 75 transfers**, including
warmups. No OS settings or other apps were stopped, paused or tuned.

Builds used explicit Rust 1.98, the same release-perf optimization profile and
no RUSTFLAGS. Earlier frame-pointer builds are listed as unrelated cached
fingerprints in [the build receipt](build-receipt.json). Artifact identities:

- Original installed baseline: `d11c8402c381eeb137794235d94466d9556db874f94f4887174807a1ac2c4491`.
- Measured and installed timer fix: `d858a827fb7a682902a36255fe17a5e5ed2b7ff32dbc7454718ac7fb02bd0b91`.
- Asupersync upstream revision: `78b64636e99fea4ea2d868096576021dd3b8e519`.

The [summary helper](../../../../agent_ergonomics_audit/audit/regression_tests/summarize_engine_benchmark.py)
checks every hash, seed-receipt match, time window and native post-close counter
before calculating distributions. High percentiles are nearest rank; at these
sample counts p99 and more extreme estimates equal the maximum and are
conservative, not extreme-tail qualification. The paired bootstrap uses 10,000
resamples and assumes independent matched blocks.

## Functional verification and installation

- **63 actual timer-wheel tests passed**, including four new cancellation churn,
  generation-reuse, wheel-wrap and coalescing checks, plus original metamorphic
  cases. One test-only file omitted from the crates.io package was restored from
  the exact upstream revision; its receipt is retained.
- **88 app tests passed** on the timer-only build. Engine/runtime suites passed
  47 engine and 14 runtime tests; ignored cases are recorded in the raw logs.
- Real libtorrent metadata and payload interoperability passed, including the
  test's eight encryption-policy cases.
- Isolated network-interface lifecycle checks passed for down/up,
  disappearance, and same-name/rapid replacement without changing host routes.
- [Three simultaneous downloads](three-download-final.json), 32/48/64 MiB at
  4 MiB/s each, all advanced while partial. The real TUI kept drawing, and the
  existing maintenance timer persisted completion of the first torrent while
  another still downloaded.
- Quitting those active transfers stopped the process, IPC and listener;
  peer receipts closed while the seeds remained alive. Partial files remained
  unchanged for 0.5 seconds. Reopening completed all three exact SHA256 files.
  This is functional shutdown/resume evidence, not a repeated latency benchmark.
- The initial three-download observer incorrectly treated API readiness as
  finished restoration and saw `checkingDL`. It was corrected to wait for the
  restored partial states; the engine was unchanged. The failed record remains.

The exact measured binary was installed atomically at
`/home/oliver/.local/bin/fbittorrent`, mode 0755. The original executable is
preserved under `target/cpu-optimization/baseline`. The actual app remained
closed with its backend stopped; installation did not launch it.
All **eight guarded user files** remained byte-identical, including settings,
widget configuration, torrent state and RSS/Shows data. qBittorrent's real
systemd unit remained inactive and disabled. Owned fixture processes and
temporary directories are gone.

[Installation receipt](install-receipt.json), [user state check](user-state-after.json),
[cleanup receipt](cleanup.json), [host summary](host-summary.json).

The [independent Sol 6.1 xhigh review](independent-review.md) recomputed the
raw statistics, reconciled host counters, and accepted the source, artifact,
receipt and lifecycle evidence with the stated limits.

## What these results do not establish

This measures the whole app and runtime. Both the frontend and upstream rqbit
were already Rust, so it does not isolate a language-port benefit. `/tmp` is
tmpfs and executable/payload caches are warm. Process RAM excludes the common
seed and kernel/file cache. There is no allocator high-water measurement.

The seed cap does not measure maximum throughput. The fixture does not qualify
Internet swarms, physical disks, encrypted-transfer CPU, ProtonVPN throughput
or battery energy. Progress polling every 100 ms is included in app CPU;
short-lived add CLI CPU and the common seed are excluded from download CPU.
Download elapsed time includes the CLI's fixed 400 ms sleep.

Opening means substantial TUI output plus a successful API/status response,
not every background task settled. qBittorrent opening includes preference
setup. Private torrents suppress payload discovery, but idle discovery settings
are not fully identical. Cached reopen covers one file, not a large season
library. Small shutdown timings include Python/PTY wait scheduling.

The original morning benchmark remains unchanged at
[the earlier report](../../2026-10-10/engine-benchmark/README.md). Its CPU numbers
are historical; the before/after improvement here uses freshly matched original
and fixed builds on this host.

## Follow-up: 12 October 2026

The subsequent [scheduler and upload optimization](../../2026-10-12/cpu-optimization/README.md)
was independently reviewed, measured and installed on 12 October. It retains
the timer fix and reports fresh matched comparisons against this installed
version. The measurements above remain historical evidence for the timer fix.
