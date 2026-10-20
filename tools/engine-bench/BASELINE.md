# Phase 1: 2026-10-20

Intel Core Ultra 7 258V, eight cores, Linux, shared host. Decimal MB/GB;
RSS converted from KiB to MiB. No shared cache flushing. Each measured daemon
uses fresh private HOME/XDG/downloads, loopback binding, upload off, idle exit
disabled, and an ordered shutdown/process audit. Three repeats unless stated.
The initial one-minute load ranged approximately 5–27 across the session.

The accepted initial runs use **tmpfs** (`--disk-root /tmp`), measuring an engine
ceiling rather than disk throughput. Source `190176f`, binary SHA-256
`10b61661bdf2b49090de1df9ec32d947b7845ef089b84fad5e7343fb51008247`, release-perf
(same optimizations as release, retained symbols). Main's later hardening,
refactor and sampler changes are not attributed to this work.

| Workload | Median (min–max) | CPU s/GB | Peak RSS MiB |
|---|---:|---:|---:|
| 1 GiB / 1 seed | 284 MB/s (235–338) | 3.33 | 22.6 |
| 1 GiB / 4 seeds | 527 MB/s (491–622) | 2.09 | 23.2 |
| 1 GiB / 16 seeds | 356 MB/s (285–566) | 3.01 | 24.6 |
| Initial hash check | 952 MB/s (799–1,405) | see JSON | see JSON |
| Explicit recheck | 850 MB/s (778–1,258) | see JSON | see JSON |
| Streaming: app playable | 1.030 s (1.010–1.038) | — | 26.9 |
| Magnet: first verified piece, with choke fix | 1.040 s (1.037–1.052) | — | 23.6 |

Scale controls, one 20-second window each: 100 idle torrents used 0.04 CPU s
and 215 KiB RSS/torrent; 100 actively progressing torrents used 0.81 CPU s and
387 KiB/torrent; 500 idle torrents used 0.08 CPU s and 68 KiB/torrent.
500 active torrents failed admission with resource pressure preventing a child
region. That is a failed capacity result, not a successful active measurement.
Low-rate seeds flush each block: buffering a whole megabyte at 32 KiB/s would
incorrectly trigger the real client's read timeout. Every active torrent must
make verified byte progress; instantaneous zero-speed/stalled gauges between
blocks are allowed.

All accepted transfers verified the complete deterministic payload and SHA-256,
first/last priority and sequential single-peer order, zero payload upload,
zero Unchoke messages, and clean shutdown. The original magnet runs exposed two
Unchokes during metadata bootstrap. Commit `341dc4e` keeps that handler choked;
three real 1 GiB magnet runs then passed. Choke-fix binary SHA-256:
`08d2405c0b9ff63918849de078e500e56e8e5c762e31d920de142999e58ae1a3`.

## Diagnostics and first opportunity

Separate instrumented runs are never throughput comparisons. Kernel CPU-clock
sampling captured 995 user PCs, zero loss/errors, 31 threads. Exclusive samples:
hardware AWS-LC SHA-1 29.95%; Cx checkpoint 3.12%; malloc 1.81%; scheduler execute
1.31%; scheduler next-task 1.01%. Library/unknown PCs remain in the denominator.
This diagnostic completed the payload but failed the disk shutdown drain, so
its wall time is excluded. Hardware hashing is already selected.

Allocation diagnostic: 1,402,279 calls / 1 GiB, about 1,306 allocations/decimal MB.
A separate successful tmpfs strace run counted 487,271 calls / 1 GiB, about
454 calls/MB, including owned helper execs. Prominent counts: ioctl 203,350;
pwritev 65,536; writev 65,541; readv 36,107; pread64 16,385. Strace's summed
futex wall time is not CPU time. Interface checks must remain on every actual
socket I/O operation.

Measured opportunity: coalesce already-queued outgoing payload requests into
bounded FIFO writes. Impact 3 × confidence 4 / effort 3 = **4**. Reuse the
existing buffer, never wait to fill it, never cross a control/policy message,
keep cancellation checkpoints between bounded batches, retain all interface
checks, and leave picker/hash/upload/durability logic untouched. Required proof:
wire fields/order, control barriers, whole-payload hash, first/last streaming,
choke/hostile upload probe, shutdown drain, before/after CPU and syscall counts.
Floating-point and RNG behavior are unchanged; this changes only write grouping.

## Retained observations

Relative to the assigned worktree:

- `target/engine-bench/baseline-tmpfs/*/{result,teardown}.json`
- `target/engine-bench/metadata-proof/*/{result,teardown}.json`
- `target/engine-bench/scale-paced/idle100-1/`
- `target/engine-bench/scale-valid/{active100-1,idle500-1,active500-1}/`
- `target/engine-bench/profile-kernel2/download1-1/{profile,summary}.json`
- `target/engine-bench/syscalls-tmpfs/download1-1/syscalls.txt`

Disk runs on the host's Btrfs volume observed roughly 26–39 MB/s under heavy I/O
pressure but failed strict shutdown/snapshot drain checks. They are excluded
from accepted throughput numbers. No deadline or stop-rule assertion was relaxed.
No benchmark process was left running after these completed or failed cases.
