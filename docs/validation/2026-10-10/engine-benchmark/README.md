# Native engine comparison — measured 11 October 2026

Embedding the engine gives us one app that stops its engine when closed, uses
about half the client process RAM, and starts this local transfer sooner.
**Active download CPU is a regression: about six times the previous stack.**
The next performance work should target that cost.

This compares native Rust/Asupersync with qBittorrent-nox 5.2.4 plus the **same
fbittorrent TUI**. The frontend and upstream rqbit were already Rust; this
describes the engine/runtime/architecture change rather than a language-only
speedup. No app source or runtime policy was changed for these measurements.

## Results

Medians below. There are 20 alternating pairs, 60 status CLI queries per engine,
and three 10-second windows per engine for each idle state. RAM is aggregate
process PSS, which apportions shared pages rather than counting them fully in
every process. It excludes kernel/file-cache memory and the common seed.

| Metric | Native app | qBittorrent plus TUI | Practical result |
|---|---:|---:|---|
| Idle process RAM, empty library | 28.85 MiB | 62.85 MiB | 54% less process RAM |
| Peak process RAM during download | 34.41 MiB | 72.05 MiB | 52% less process RAM |
| Idle CPU, empty library | 0.40% of one core | 0.50% of one core | Both low; three windows cannot establish a meaningful small CPU win |
| Idle CPU, completed fixture | 0.50% of one core | 0.40% of one core | Both low |
| Peak threads during download | 9 | 17 | Fewer threads |
| CPU work for the 128 MiB download | **3.64 CPU-seconds** | **0.61 CPU-seconds** | **Native uses 5.97 times the client CPU** |
| Add command to completion | 4.115 s | 6.551 s | 37% earlier completion in this fixture |
| Add to first payload | 52 ms | 1,885 ms | Earlier start explains much of the wall-time gain |
| Payload transfer time | 4.000 s | 4.000 s | Median rates reach the same 32 MiB/s cap |
| Empty app opening | 16.3 ms | 139.4 ms | About 123 ms saved |
| Reopening completed 128 MiB fixture | 91.9 ms | 132.2 ms | Small cached library only |
| Widget/status CLI response | 2.21 ms | 2.92 ms | Both very quick |
| Quit TUI | 11.4 ms | 3.3 ms | Legacy TUI exit leaves its daemon running |
| Stop every required process | 11.5 ms | 11.6 ms | Essentially tied when the legacy daemon is explicitly stopped |
| Engine RAM after quitting TUI | 0 MiB | 47.08 MiB | Native engine exits with the app |

CPU-seconds measure processor work summed across client processes and threads;
lower is better. Native median user/system work was 2.755/0.91 seconds, versus
0.38/0.23 for legacy. This is not a measurement of battery energy.

All **40 downloaded files matched the expected SHA256**. Each main run has one
positive seed receipt containing exactly 128 MiB, and every first/last payload
timestamp lies inside its add-to-completion interval. A read-only subagent
independently verified the raw receipts, counts and summary calculations.

Native reached the cap in all 20 transfers. Legacy reached it in 18; two legacy
transfers took about 6.05/6.09 seconds on the wire, averaging about 21 MiB/s.
Those samples are retained with no assigned cause. The typical payload rate
is tied at the cap; this **does not establish maximum throughput**.

## Method and host

- Installed native binary SHA256:
  `d11c8402c381eeb137794235d94466d9556db874f94f4887174807a1ac2c4491`.
- qBittorrent-nox 5.2.4 SHA256:
  `39552701e4f06d690f0e46746fc98847fe621f6d1e0ea623e48e63d56e96e2d5`.
- Intel Core Ultra 7 258V, eight cores, Omarchy kernel 7.2.8-5-omarchy-bore,
  optimized release-perf native artifact installed stripped. The host fingerprint
  and unchanged power profile are in `quiet-fingerprint.json`.
- The user chose to wait for an unrelated index build. The final series began
  on 11 October at 09:00 Zurich time after that process exited. Ordinary desktop
  activity remained; no heavy build/index job was observed. CPU/memory pressure
  was low. Quiet checks and before/after pressure are retained.
- Same actual IMDb ratings cache, privately copied once with a fresh timestamp.
  A cached missing-title result prevents remote lookups of `benchmark.bin`.
- Same generated 128 MiB bytes, 256 KiB pieces, private torrent, common TCP seed
  capped at 32 MiB/s, loopback binding, encryption off, sequential/first-last
  policy, and fresh isolated config and destination in every run.
- Same 120×30 real TUI running in a drained PTY. Legacy totals include qBittorrent
  and the TUI. The common seed and observer CPU are recorded separately.
- **The destination is /tmp on tmpfs, a RAM-backed filesystem.** Writes and piece
  verification run with warm caches. This does not qualify physical-disk,
  cold-cache, internet-swarm, VPN-throughput or encrypted-peer performance.

## Separate polling diagnostic

With a completed 8 MiB fixture, ordinary idle was compared with roughly ten
state/procfs observations per second. Each window lasted ten seconds.

| Client | Ordinary completed idle CPU work | Frequent observations | Extra CPU per wall-second |
|---|---:|---:|---:|
| Native | 0.04 CPU-s | 0.09 CPU-s | 0.005 CPU-s |
| Legacy, both processes | 0.06 CPU-s | 0.14 CPU-s | 0.008 CPU-s |

This completed-state diagnostic found small polling overhead. It does not
attribute active-transfer cost to a runtime subsystem or justify subtracting
an exact amount from the main results. Profiling active engine work is next;
scheduling, per-block disk dispatch and bound-socket validation are source-backed
hypotheses, not proven causes.

## Limits and cleanup

- Process PSS is not whole-system RAM. Payload pages and kernel/file cache are
  outside these totals or may be mapped differently by each engine.
- Transfer CPU includes backend work from 100 ms polling and excludes the
  short-lived add CLI. Wall time includes that CLI's fixed 400 ms sleep,
  initialization, discovery and completion reporting.
- Opening means substantial terminal output plus a successful API/status
  response, rather than every background task settling. Legacy opening includes
  two benchmark preference API calls.
- Reopening used only one cached 128 MiB file. Native rehashes existing data on
  reopening, so this result cannot be generalized to a large season library.
- Native keeps its discovery defaults; legacy PeX/LSD are disabled. The private
  torrent suppresses discovery for the payload, but empty-session discovery
  is not identical.
- Sample p95 values are descriptive; 20 runs do not establish extreme tails.
- The real app was closed before and after. Its qBittorrent unit stayed inactive
  and disabled. Nine guarded config/shell/saved-data files were unchanged.
  The original before snapshot used an incorrect `native` ledger path; an
  additional early-run guard accurately records the actual `engine` ledger
  and all saved JSON/torrent files.
- All owned main/diagnostic processes, sockets and temporary directories were
  removed. User downloads, settings and installed binaries were unchanged.

## Evidence and reproduction

- `results.json`: completed quiet series, all raw runs and labeled seed receipts.
- `summary.json`: counts, median/p95/min/max and matched peer timings.
- `observer-calibration.json`: separate completed-state polling diagnostic.
- `quiet-fingerprint.json`, `quiet-host-checks.jsonl`: host/build and quiet checks.
- `quiet-user-state-before.json`, `quiet-user-data-guard.json`,
  `quiet-user-state-after-main.json`, `quiet-cleanup.json`: state/cleanup receipts.
- `preliminary-background.json`, `pilot.json`, `thread-samples.json`: earlier
  interrupted busy-host observations; they are not final proof.

The directory retains its 10 October path because work began then; the completed
measurement timestamps are 11 October.

```sh
python3 agent_ergonomics_audit/audit/regression_tests/engine_benchmark.py \
  /home/oliver/.local/bin/fbittorrent \
  --runs 20 --mib 128 --rate-mib 32 --idle-seconds 10 \
  --ratings /home/oliver/.cache/fbittorrent/title.ratings.tsv \
  --output docs/validation/2026-10-10/engine-benchmark/results.json
```

## CPU optimization follow-up — 11 October 2026

The downloading-CPU issue was subsequently traced to cancelled Asupersync
timers. The fix was installed after twenty fresh matched comparisons against
the preserved original binary, plus a separate longer-transfer check. The
[CPU optimization report](../../2026-10-11/cpu-optimization/README.md) records
the new results, remaining CPU gap, RAM and shutdown checks. The measurements
above are preserved as the original historical benchmark.

## Receive and storage optimization follow-up — 14 October 2026

The adaptive receive/storage pipeline is now installed after ten scoped Rust
gauntlet rounds and two distinct twenty-comparison confirmations. Matched
active-download CPU fell **23.30% / 24.40%** versus the preserved native baseline,
with about **53% less process PSS than qBittorrent**, unchanged native capped
wire throughput and tested engine exit when the app closes. Native CPU remains
about **3.2 times qBittorrent** in the new 512 MiB workload. These numbers are
separate from the historical 128 MiB comparison above. The
[completed receive/storage report](../../2026-10-13/per-chunk-cpu/README.md)
records raw evidence, profiles, negative experiments, qualification limits,
cleanup and exact installation identity.
