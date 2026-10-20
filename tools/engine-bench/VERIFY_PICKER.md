# Verification and picker optimization

The control is the retained `release-perf` binary built from `0bf7bb6` (SHA-256
`a6f2f0fabd3ac34674553e3ea3c2b7b8a3991693f63102a3351d1f98c3aa4943`).
Its engine, native runtime and rqbit sources are identical to merged main
`1b9c4e0`. Setup changes between those revisions are outside these workloads.
Measurements and immutable binaries live outside Cargo targets in
`/home/oliver/Projects/odcpw/fbt-verify-evidence-20261020-cod1/`.

## Method

The initial profile in `fbt-astra-engine-perf/tools/engine-bench/RESULTS.md`
identifies hardware SHA-1 and picker scanning as hot paths. The new diagnostic
uses the private 4 GiB `recheck` workload and the existing per-thread kernel
CPU-clock sampler. Instrumented timings are excluded from comparisons.
Normal comparisons use three warmups and ten AB/BA pairs, identical metadata,
private filesystem and seed executable. No shared caches are flushed.

| Opportunity | Impact | Confidence | Effort | Score |
|---|---:|---:|---:|---:|
| Independent verification ranges on existing workers | 4 | 4 | 2 | 8 |
| Lazy queued-piece selection and set-bit traversal | 2 | 5 | 1 | 10 |

## Verification isomorphism

Each range contains whole pieces. Its file cursor starts at exactly
`first_piece * piece_length`; the original file/padding traversal, reusable
64 KiB read buffer, hardware SHA-1 implementation and expected-digest comparison
remain unchanged within the range. Read ordering between ranges changes; a
piece's byte order and digest do not. Disjoint verified bitmaps combine by OR.

Only the complete successful result reaches the tracker and resume cache.
Cancellation or pause discards the result. A shared physical-work guard covers
all admitted callbacks even if their async waiter is dropped. The existing
storage admission and native pool bound all checks; no extra threads are
created. One quarter of available logical CPUs, at most two, are used for checks
of at least 32 MiB. Small checks and app-validated resume snapshots stay serial
and zero-read respectively.

Ordering: download priority is untouched by verification. Tie breaking: N/A,
each piece has its own expected digest. Floating point: N/A. RNG: unchanged.
The filesystem regression compares serial and parallel maps at every split of
an uneven multi-file torrent, then corrupts a spanning piece and proves that
only that piece is absent and queued. The app test gates actual physical reads
and proves prompt cancellation and no partial-have publication during both
initial checking and explicit recheck, including saved-paused intent.

## Verification measurement and decision

Ten retained alternating pairs after three warmup pairs, private tmpfs, 4 GiB.
Values are median [min, max]; instrumented runs are excluded.

| Phase | Control wall seconds | Candidate wall seconds | CPU seconds, control → candidate |
|---|---:|---:|---:|
| Initial check | 2.941 [2.834, 3.019] | 1.603 [1.510, 1.849] | 2.980 → 3.050 |
| Explicit recheck | 2.860 [2.686, 3.362] | 1.769 [1.656, 2.135] | 2.845 → 3.415 |

Paired median wall reduction: initial 45.4% (bootstrap 95%: 40.4–46.8%),
recheck 38.4% (30.8–41.1%). Keep: the bounded two-worker wall-time gain is
useful despite the explicit recheck's 20% increase in CPU seconds.
Every payload was independently SHA-256 verified; all 26 runs drained without
SIGKILL or survivors (64–1066 ms). Cold disk throughput is unproven: these
comparisons use RAM-backed payloads and the rechecks report zero physical reads.

Evidence: `paired-summary.json`, `paired.log`, `paired/{before,after}/*/result.json`
and adjacent `teardown.json` in the directory above. Candidate binary SHA-256:
`7ef7d7e92db146703ec2f9b24b62bbe2ba4a2421fada94fc4706b7610682ecc5`.
Its source manifest and patch are `candidate-build.json` and `candidate.patch`.

The separate diagnostic profiles retain hardware SHA-1 as the dominant user-CPU
symbol (control 97.05%, candidate 99.45%, no lost samples). They are not timing
comparisons: the control diagnostic used Btrfs and the candidate used tmpfs.
A 1 GiB loopback wire run also verified first/last and sequential-middle order,
byte-identical output and clean drain (`wire/download1-1/`, 565 ms shutdown).
