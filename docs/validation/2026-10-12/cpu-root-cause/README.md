# Engine CPU optimization — completed measurements, 12 October 2026

The targeted integration fixes are installed. **Download CPU fell by 11.7% in
matched rounds, but still uses 4.02 times qBittorrent's client CPU.** The remaining
CPU gap is significant. Asupersync and FrankenTUI remain in use, the app keeps
its process RAM saving over qBittorrent, and closing it stops the engine.

The installed normal executable is
`d72971dc337b8b875737897831c2baeccf9054a6b56b93cc6fbf9efaa56cbe2b`.
[Installation receipt](installation.json) binds the exact measured artifact to
the installed file; mode is `0755`. The preserved previous native binary is
`feb8657526da270c94bcfa8974b9fd09b312673947aae223d45bd37c7bbfc33e`.

## Final comparison

Twenty matched rounds compare the previous installed native app, the new
native app, and qBittorrent-nox 5.2.4 with the same TUI. All sixty measured rows
are retained; three separate warmups are excluded. Each transfer uses 512 MiB,
256 KiB pieces, a private loopback torrent, the common 32 MiB/s seed cap, fresh
isolated destinations, and the same actual IMDb cache.

Medians below. CPU-seconds measure total processor work. RAM is aggregate
process PSS; kernel/file cache, the common seed, observer and desktop widget
are outside these totals.

| Metric | Previous native | Installed native | qBittorrent + TUI |
|---|---:|---:|---:|
| CPU work per 512 MiB | 6.565 CPU-s | **5.790 CPU-s** | 1.440 CPU-s |
| Peak process RAM during download | 38.43 MiB | **44.05 MiB** | 76.34 MiB |
| Idle RAM, empty library | 28.66 MiB | 28.63 MiB | 59.16 MiB |
| Peak download threads | 9 | 9 | 18 |
| Open empty app | 24.66 ms | 25.28 ms | 152.30 ms |
| Status CLI, 60 queries per variant | 2.11 ms | 2.12 ms | 2.89 ms |
| Quit TUI | 15.38 ms | 11.38 ms | 3.24 ms |
| Stop all required processes | 15.49 ms | 11.49 ms | 15.77 ms |
| Engine RAM after quitting TUI | 0 MiB | **0 MiB** | 50.65 MiB |
| Reopen completed 512 MiB fixture | 338.97 ms | 335.65 ms | 159.37 ms |
| Add to completion detection | 16.111 s | 16.107 s | 21.485 s |
| Seed's payload wire interval | 16.000 s | 16.000 s | 19.021 s |

All **20/20 matched CPU comparisons improve**. The median matched saving is
11.6573%; a descriptive 10,000-resample bootstrap interval is 9.39–13.03%,
assuming independent matched rounds. The ratio of the new and qBittorrent CPU
medians is 4.0208. The complete distributions and individual savings are in
[summary](final-512-summary.json) and [raw results](final-512-results.json).

The RAM tradeoff is explicit: peak native PSS rises **5.62 MiB** relative to the
previous native build, while remaining **42.30% below qBittorrent**. Empty idle
RAM remains about 51.6% below qBittorrent. Three idle windows per variant do not
establish a meaningful small idle CPU improvement.

Both native versions reach the cap in all twenty transfers. Every qBittorrent
wire rate is below it: 26.22–27.52 MiB/s, median 26.92 MiB/s. No cause is assigned
to that difference. The native optimization establishes **no download speed
gain** at this cap. Independent payload hashing occurs after the captured
completion time. The previous native's last reopening outlier, 1,363 ms, remains
in the results.

## What changed and why

- **Reuse storage owners across writes.** Repeated managed owner creation for
  small chunks is replaced with bounded reusable workers. A lazy lease belongs
  to the peer: idle peers allocate no worker group, departed peers release it,
  and a full cache can fall back and recover later. The cache is bounded to
  sixteen live groups; global physical admission remains two. Workers still
  check caller identity, authority, priority and cancellation before entry.
- **Correct runtime diagnostics.** RuntimeState supplies independent table
  mutation counts. Comparing their numeric distance as publication order
  generated 163,899 `ModuleDesync` observations in one actual old diagnostic
  transfer. Private runtime tracking now treats these as independent sequences.
  Real local errors and lifecycle/holder/generation/close checks remain. Public
  shared-domain defaults retain their previous checks.
- **Avoid repeated history compaction.** The bounded 1,000-entry epoch history
  uses a queue with preserved order, deduplication, snapshots and reset behavior.
- **Keep physical writes owned through completion.** Dropping an async waiter
  cannot release its permit or finish region close while its write is running.
  Captured destructor panics settle with the original panic, graceful peer
  departure drains accepted work, and both worker exit and failed spawn
  reservation rollback release the final live-group slot.

These changes are measured together in the final comparison. Earlier isolated
pilots are useful causal evidence; their percentages cannot be added. The
[historical investigation](investigation-log.md), [Oracle assessment](oracle-report.md),
[Oracle follow-up](oracle-followup-report.md), and [attribution](fp-analysis.md)
retain the individual experiments and their limits.

A one-thread pool trial was **rejected**. It used more CPU in all four matched
pairs, median matched saving −2.37%, with no consistent RAM benefit. Max two is
retained. [Pilot raw results](pool-choice-results.json),
[summary](pool-choice-summary.json), [monitor](pool-choice-run.json).

## What the remaining gap means

Local upstream rqbit 9.0.1 already waits for each chunk's write/checksum before
reading the next peer message. The port preserved that order. No omitted
upstream asynchronous download queue was found.

The structural difference is the owned-buffer copy and storage dispatch/reply
round trip for each 16 KiB chunk. That remains an optimization seam; this
comparison does not measure its remaining share. Worker-only batching usually
has only one job available. A useful batch would need bounded reader/handler
ownership, ordered writes/checksums, observed errors/panics and physical drain.
[Upstream comparison and exact source identities](upstream-piece-path.md).

The older frame-pointer profile supports investigating lifecycle and dispatch
cost. The subsequent normal actor profile has usable self samples but mostly
missing caller frames. It cannot establish reliable inclusive shares of the
remaining hot paths. Diagnostic inline paths were removed and never installed;
running filesystem work on the cooperative scheduler is not an accepted fix.
[The raw profile is also preserved outside the build cache](durable-profile-copy.json).

## Correctness and lifecycle checks

- **19 final storage tests pass**, including real held-file writes, cache
  saturation/recovery, departing/idle peers, authority and native cancellation,
  accepted sibling drain, queued capture panic, and the final reservation race.
  Old EOF and retirement implementations fail the same focused regressions.
  The original owner tests' 71 assertion token sequences were independently
  verified as preserved.
  [Final owner tests](storage-drop-fix/peer-lease-prototype/lease-receipt.json),
  [retirement race](storage-drop-fix/peer-lease-prototype/retirement-rollback/README.md),
  [assertion preservation](storage-drop-fix/peer-lease-prototype/assertion-preservation.json).
- **The original two application ownership tests pass.** Their source and
  2 s/50 ms bounds remain unchanged.
  [Application receipt](storage-drop-fix/peer-lease-prototype/native-receipt.json).
- **The exact selected executable passes three simultaneous partial downloads.**
  All three advance, the terminal responds, close leaves zero resources, partial
  data remains stable while closed, reopening completes all three with exact
  hashes, and reclose leaves zero resources. Resume retains partial data;
  limited retransmission is present. [Selected functional receipt](lease-pool2-v3-functional.json).
- **16 independent libtorrent interoperability cases pass:** eight each at
  actual configured pool maxima one and two, covering plain/RC4, incoming
  uploads, outgoing downloads, magnet metadata and routing between two torrents.
  Each case transfers 196,721 bytes and bounds teardown. [Interop receipt](lease-v3-interop.json),
  [log](lease-v3-interop.log).
- **Eight public epoch semantic tests pass**, including shared defaults, real
  local violations and actual native cancellation/quiescence.
  [Semantic proof](epoch-domain-public-proof.md).
- All **63 final files**, including excluded warmups, match the expected hash
  and one positive joined full-byte seed receipt. Zero-byte reconnect receipts
  are also retained. Every native close and reclose has zero resources.
  The read-only Sol 6.1 xhigh reviewing subagent independently recomputed all
  raw checks, metrics and the bootstrap interval.

Earlier full scopes passed 90 application, 68 engine and 14 runtime tests;
those precede the final retirement regression. They are not presented as a
fresh final full-suite run. Profiling, extreme optimization and Asupersync JSM
skills guided the scoped changes and evidence checks.

## Conditions, limits and unchanged state

The final series starts only after two complete passing 30-second windows,
14.09% and 13.08% residual host activity. Monitoring has 47 complete windows
at 8.30–15.90%; the last 20.84 s partial window is separate, at 8.15%. There are
no heavy external jobs, active inventoried CAD work, identity changes, accounting
errors or load disqualifiers. Ordinary desktop activity remains; short/new
external jobs cannot all be detected. [Qualification](final-512-qualification.json),
[monitor receipt](final-512-run.json), [samples](final-512-monitor.jsonl).

The host is Intel Core Ultra 7 258V, eight cores. Governors, balanced power
profile, `balance_performance` EPP, affinity and turbo settings match before and
after; none were tuned. Some earlier pilots used `balance_power`, so their
absolute CPU seconds are not compared as optimization percentages.
[Before fingerprint](final-512-fingerprint.json),
[after fingerprint](final-512-after-fingerprint.json).

Normal Rust 1.98, optimized `release-perf`, thin LTO and one codegen unit were
used, without extra features, frame-pointer flags or measurement counters.
Before/after maps covering 1,151 files are identical. The installed file is
byte-identical to the normal tested/measured P2 artifact.
[Build receipt](lease-pool2-v3-build.json).

This is private loopback on RAM-backed `/tmp` with warm caches. It does not
qualify cold physical disks, internet swarms, VPN throughput, encrypted-peer
CPU performance, battery energy, maximum throughput or large-library reopening.
Opening includes backend readiness and a successful status response;
qBittorrent opening includes two fixture preference calls. Active CPU excludes
the short-lived add CLI and common seed, and includes backend progress polling
every 100 ms. High percentiles at twenty samples describe observed tails.

The real app stayed closed. Its qBittorrent unit remains inactive/dead/disabled.
Eight guarded settings and saved-data hashes are unchanged, including widget
settings and the actual engine ledger. An abandoned directory from the earlier
interrupted owned pilot was identified and removed after checking it had no
process users. A source backup that matched the broad prefix was preserved.
All known owned fixture directories and clients are gone.
[Cleanup](state-and-cleanup-before-install-final.json),
[stale owned fixture receipt](stale-owned-fixture-cleanup.json),
[final state](state-after-install.json).

Exact commands are retained in the build, qualification and monitored-run
receipts. Reproduction must use fresh output names, the same two-window host
qualification and monitored wrapper, the preserved baseline, actual ratings
cache, `--runs 20 --mib 512 --rate-mib 32 --idle-seconds 10`, and the installed
normal artifact. Do not overwrite the completed evidence.
