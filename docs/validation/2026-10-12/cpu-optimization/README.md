# Remaining download CPU optimization — 12 October 2026

**Implemented, independently reviewed, benchmarked and installed.** The exact
measured executable uses **38.94% less download CPU work** than the installed
11 October timer fix in twenty matched blocks. Peak app memory remains about
**56% below qBittorrent + TUI**, and the native engine stops with the app.
Asupersync remains the runtime.

**A substantial CPU gap remains:** selected native uses 2.93 times qBittorrent's
CPU work in the main fixture. The three-block larger-file check improves native
CPU work by about 40%, but still measures a 4.71-times gap. These results
establish a useful reduction; they do not establish parity.

## What changed

The app now uses one Asupersync scheduler worker instead of two. Profiling
identified considerable scheduling, locking, wakeup and timer overhead in this
small client. One worker reduces that coordination. The existing blocking pool
still supports one to two workers, and storage admission remains two.

Uploads needed a companion correction: file reads run on the existing blocking
pool so they cannot stall the sole scheduler worker. The writer transfers its
existing buffer into a region-owned operation and receives the same buffer
back. That operation acquires its own admission permit and holds it until the
physical read finishes, including when the peer drops its waiter. There is no
additional payload copy or new thread pool. Cancellation, shutdown, encryption
and interface readiness checks remain in place.

The first upload attempt acquired the permit in the caller. Moving it in Rust
did not transfer the runtime's obligation holder; a cancellation test exposed a
leak and worker panic. The corrected operation owns both the acquisition and
lifetime. No deadline or assertion was relaxed. See the
[ownership failure and correction](upload-ownership-failure.md),
[retained upload patch](upload-guard.patch), and
[negative-control receipt](upload-negative-receipt.json).

## Main comparison: 128 MiB at 32 MiB/s

Twenty matched blocks each contain the preserved installed native binary,
selected native, and the selected TUI with isolated foreground qBittorrent
5.2.4. Six execution orders rotate; native/qBittorrent relative order is balanced
ten each way. One warmup per variant is excluded.

CPU seconds measure processor work: one CPU second is one core busy for one
second. Memory is PSS, which counts shared memory proportionally across the
measured app processes. All table entries are medians.

| Metric | Installed timer fix | Selected native | qBittorrent + TUI |
|---|---:|---:|---:|
| Download CPU work | 2.72 CPU-s | **1.67 CPU-s** | 0.57 CPU-s |
| User / system CPU | 1.85 / 0.88 CPU-s | 1.09 / 0.58 CPU-s | 0.35 / 0.22 CPU-s |
| Peak app PSS | 34.23 MiB | **34.02 MiB** | 77.14 MiB |
| Actual payload transfer | 4.000 s | 4.000 s | 4.000 s |
| Add to completion | 4.117 s | 4.118 s | 6.474 s |
| Open app | 15.92 ms | 16.34 ms | 133.51 ms |
| Status CLI | 2.09 ms | 2.12 ms | 2.75 ms |
| Stop app and engine | 15.55 ms | 15.50 ms | 12.02 ms |
| Reopen and verify completed file | 96.97 ms | 90.58 ms | 129.20 ms |
| App PSS after TUI closes | 0 MiB | **0 MiB** | 49.79 MiB |

Median matched CPU reduction is **38.94%**, with improvement in every block
(34.47–41.70%). A descriptive 10,000-resample bootstrap 95% interval is
37.50–39.48%, assuming independent matched blocks. CPU sample variation is
1.64% / 2.38% / 6.45%; CPU-work p95 is 2.77 / 1.73 / 0.60 CPU-s. Twenty samples
cannot qualify extreme tails. Status latency has sixty calls per variant;
empty/completed idle measurements have three samples per variant.

Empty app PSS is 30.53 / 30.44 / 66.04 MiB; completed-idle PSS is
34.20 / 34.00 / 79.94 MiB. Selected peak PSS is 55.90% below qBittorrent + TUI.
CPU includes the measured TUI and qBittorrent processes, including the common
100 ms status polling, and excludes the common Python seed and add CLI. PSS
excludes kernel memory, filesystem cache and the seed.

The fixture measures what remains after TUI closure, then explicitly stops its
own qBittorrent process. That cleanup is included in qBittorrent's total-stop
time; ordinary legacy TUI closure leaves the daemon running. Every native close
and reclose removes IPC and leaves zero measured CPU counters, RAM and threads.
The harness measures CLI readiness and cached rechecking; these are not
measurements of visual perception or physical disk throughput.

[Raw main results](final2-results.json), [distributions](final2-summary.json),
[run receipt](final2-run.json), [host fingerprint](final2-host.json),
[host qualification](final2-host-qualification.json).

## Larger and faster checks

Each check uses three matched blocks and three excluded warmups. Their first
three order permutations do not balance every variant's position. These are
descriptive checks of duration and speed risks, with no reliable confidence
interval or general maximum-throughput claim.

| Check | Installed CPU | Selected CPU | qBittorrent CPU | Matched CPU saving | Selected / qBittorrent |
|---|---:|---:|---:|---:|---:|
| 512 MiB, 32 MiB/s seed | 10.56 CPU-s | **6.41 CPU-s** | 1.36 CPU-s | 39.70% | **4.71×** |
| 512 MiB, 128 MiB/s seed | 5.17 CPU-s | **3.55 CPU-s** | 1.09 CPU-s | 32.50% | 3.26× |

At 32 MiB/s, both native builds transfer the payload in 16.000 seconds; selected
peak PSS is 39.62 MiB versus qBittorrent's 80.71 MiB, a 50.92% saving.
At 128 MiB/s, both native builds sustain the seed cap for approximately four
seconds, and selected PSS is 39.93 MiB versus 87.80 MiB. This establishes no
native throughput loss in that capped fixture. qBittorrent's payload takes
about 19.016 seconds in both larger checks. The fixture does not isolate why;
these duration differences do not establish Internet speed differences.

[Longer raw results](scaling-results.json), [summary](scaling-summary.json),
[run receipt](scaling-run.json); [faster raw results](rate-results.json),
[summary](rate-summary.json), [run receipt](rate-run.json).

## Functional verification

- Final source: **88 app tests**, **50 engine tests** (two ignored), and
  **14 runtime tests** (one ignored) passed. Logs:
  [app](selected-app-tests.log), [engine/runtime](selected-engine-runtime-tests.log).
- The three upload tests check native timer progress during a real blocking
  read, admission lifetime after waiter cancellation, and read-error handling.
  The old inline read fails the timer-progress negative control as expected.
- **Eight real libtorrent interoperability cases** passed with one scheduler
  worker: plain/encrypted, outgoing torrent/magnet policies, and incoming
  two-torrent SKEY routing. Exact payload and shutdown assertions are unchanged.
  [Interop log](selected-interop-tests.log).
- The exact selected executable advanced **three concurrent downloads**,
  maintained responsive TUI/status and persisted one completion while another
  transfer remained active. Closing stopped the app, engine, IPC and listeners;
  partial files stayed unchanged for 0.5 seconds. Reopening completed all three
  exact-hash files, and closing again left zero resources.
  [Functional receipt](selected-functional.json). Its timings are functional
  evidence under load, separate from the qualified comparison.

All **87 final timed transfers**, including nine excluded warmups, have exact
full-payload hashes and joined seed receipts. Main native closure/reclosure has
84 zero-resource receipts; each additional check has sixteen.

## Other candidates and remaining CPU work

Request batching passed seven wire/order/backpressure/cancellation tests, but
four matched pilot blocks gave a median CPU change of **−1.25%**. It was
rejected and reverted. [Decision and evidence](batch-decision.json).

A persistent storage-worker prototype would add hundreds of lines without
evidence of a sufficiently large removable cost. Direct download task-spawn
self samples were only 0.91%, and the prototype retains the pool-to-peer wake
chain. Its opportunity score was 0.4, below the optimization skill's gate.
It remains outside production and has not been compiled or tested.

A smaller timer-context borrowing experiment was reverted. Its test build was
blocked by test modules missing from the published vendor source, and its
estimated gain was marginal. It has no passing candidate test or benchmark
claim. [Decision](ambient-decision.json),
[test isolation limits](ambient-test-isolation-limit.json).

The fresh selected perf diagnostic still shows scheduler/wakeup overhead,
clock/timer work, allocation/context bookkeeping and necessary hardware SHA1.
Most callchains lack usable callers, so self samples cannot establish precise
inclusive savings for another redesign. The syscall diagnostic hit an existing
status timeout under tracing and is partial. It is excluded from performance
proof. The fixture binds the loopback interface; per-operation interface
readiness checks remain intact. [Profile ranking](profile-after-ranked.txt),
[profile receipt](profile-after.json), [tracing limits](strace-decision.json).

## Host conditions, artifact identity and limits

The JSM profiling and optimization skills were applied with Sol 6.1 xhigh
subagents for runtime/peer investigation and independent evidence review.
No other app or OS setting was stopped, paused or tuned.

Before fresh candidate timings, independent review accepted two complete actual
30-second preflight windows, each requiring residual CPU ≤22%, summed identified
CAD-worker activity ≤5% of one core, consistent accounting, and no external
compiler/index work. Monitoring rejects two consecutive complete 30-second
residual windows above 25%, any identified CAD activity above its threshold,
heavy jobs, inconsistent accounting or fixed worker identity changes. The final
partial window is checked conservatively before accepting child completion.
[Protocol and fixed inventory](window-protocol-final2.json),
[completed-child guard tests](monitor-test-receipt.json).

Main preflight residuals were 17.02% and 17.45%; its measured series' weighted
residual mean was 17.01% (16.20–17.85%). All three final series passed the
predeclared guards. The longer series includes one 25.91% window, accepted under
the two-consecutive-window rule. This is an ordinary active desktop comparison;
procfs snapshots are non-atomic and short external work may evade classification.

Earlier busy preflights stay rejected. An initial final attempt was interrupted
after 38 measured rows when an external worker disappeared; attribution was
uncertain, and that entire series is excluded. No rows were combined with
`final2`. [Excluded-attempt decision](final-attempt1-decision.json).

Comparisons use isolated XDG state, fresh tmpfs destinations, identical actual
IMDb cache, private loopback TCP torrents, 256 KiB pieces, encryption off and
the same selection behavior. They do not qualify physical disk performance,
Internet swarms, heavy seeding, encrypted throughput, VPN throughput, battery
use or maximum speed. October 11 numbers are separate historical runs.

Builds use explicit Rust 1.98, the same release-perf profile and no RUSTFLAGS.
Artifact identities:

- Preserved installed timer fix:
  `d858a827fb7a682902a36255fe17a5e5ed2b7ff32dbc7454718ac7fb02bd0b91`.
- Selected, measured and installed executable:
  `feb8657526da270c94bcfa8974b9fd09b312673947aae223d45bd37c7bbfc33e`.

A final-source rebuild has identical code/data sections but a different GNU
build-ID note and whole-file hash. The preserved selected executable is used
consistently for functional tests, final timings and installation.
[Rebuild receipt](selected-reproduction.json),
[section difference](reproduction-section-differences.json).

The [independent Sol 6.1 xhigh review](independent-review.md) recomputed every raw
summary metric, bootstrap interval, host formula and receipt join with zero
mismatches. It accepted the bounded source, artifact and measurement evidence,
including the remaining CPU gap and rejected candidates. It read saved test
logs; it did not rerun tests, builds, profiles or transfers. Installation was
performed afterward and is recorded below.

## Installation and actual user state

The exact measured executable was atomically installed at
`/home/oliver/.local/bin/fbittorrent`, mode 0755. The old executable remains
preserved in `target/cpu-optimization-20261012/baseline`. The actual app stayed
closed with its native backend stopped. qBittorrent's systemd unit stayed
inactive/disabled. Fixture processes and temporary directories are gone.

All guarded fbittorrent settings, torrent state, RSS/Shows data and the old
backup receipt match the start of work. The desktop's `omarchy/shell.json`
changed during the elapsed run before installation; this work did not edit it,
and its cause is unverified. All eight guarded files match the fresh preinstall
snapshot after installation. The installed binary hash equals the measured hash.

[Installation receipt](installation.json), [start snapshot](user-state-before.json),
[preinstall snapshot](user-state-preinstall.json),
[postinstall check](user-state-after.json), [fixture cleanup](cleanup.json).
