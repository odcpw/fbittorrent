# Daemon bus budget — 2026-10-19

P1-08 (`fbt-p1-08-xyp`), host `t14`, release-perf binary SHA256
`d5455ca819933c76069b57e2264ba645b17a48686b8f714a91f89addbc1c9eff`. Every window is 60 seconds; each row has three
independent private daemons. Fixtures contain 10 or 100 distinct 256 KiB local
payloads, paused and settled before timing. Summary subscribers use 5000 ms.

| Torrents × subscribers | Daemon CPU seconds | Daemon peak RSS KiB |
|---|---|---|
| 100x0 | 0.21, 0.23, 0.24 | 19828, 20024, 20076 |
| 100x1 | 0.25, 0.24, 0.26 | 20384, 20236, 20300 |
| 100x8 | 0.26, 0.25, 0.27 | 20876, 20612, 20492 |
| 10x0 | 0.18, 0.19, 0.2 | 17628, 17216, 17268 |
| 10x1 | 0.18, 0.19, 0.18 | 17168, 17336, 17624 |
| 10x8 | 0.21, 0.21, 0.2 | 17772, 17872, 17828 |

The reference sends six genuine `status --json` calls ten seconds apart against
the same zero-subscriber daemon. CPU is the `/proc/self/stat` cutime+cstime delta
for only those reaped CLI children, so kernel tick rounding occurs once across
the six calls. Individual `/proc/<pid>/stat` samples often round to zero because
a CLI takes only a few milliseconds; their counters and precise `wait4` CPU are
retained separately. RSS is exclusively the observed `/proc/<pid>/status`
VmHWM; `wait4` RSS is supplementary and is excluded from the reported RSS.

| Fixture torrents | Reference CLI CPU seconds | Reference peak RSS KiB |
|---|---|---|
| 10 | 0.01, 0.02, 0.03 | 8852, 8324, 8348 |
| 100 | 0.01, 0.02, 0.03 | 8792, 8808, 8796 |

The median one-subscriber increment is **0.00 s** for 10 torrents and **0.02 s**
for 100; both reference medians are **0.02 s**. Both bar criteria pass, including
the exact two-kernel-tick boundary. Decimal arithmetic preserves equality;
there is no extra tolerance. The resource comparison gate uses baseline median
plus max(10 percent, three spreads) for every configuration and CPU/RSS metric.
Existing P0 TUI baseline entries and reference metadata remain intact.

Commands (the retained runner proves private HOME/TMPDIR/all five XDG paths
before invoking the memory-capped scope and every daemon/CLI):

```sh
FBT_BUS_RUNNAME=budget-command-final python3 target/test-logs/fbt-p1-08-xyp/run.py budget
FBT_BUS_RUNNAME=budget-proc-reference-command python3 target/test-logs/fbt-p1-08-xyp/run.py budget_reference
```

Both use `CARGO_BUILD_JOBS=4` and
`flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope -p MemoryHigh=10G -p MemoryMax=14G --quiet -- …`.
The first executes `resources.py <binary> --daemon --matrix --runs 3 --seconds 60`.
After correcting reference RSS to use `/proc` alone, the second remeasures all
six affected zero-subscriber/reference pairs using the same production
`resources_daemon.measure` function and unchanged binary. It retains the valid
one/eight-subscriber windows. The combined artifact explicitly records those
sources; no sample is chosen by its value.

Evidence below is relative to `target/test-logs/fbt-p1-08-xyp/`:

- `daemon-budget-qualified.json`: final arrays, raw CLI calls, bar verdicts,
  binary hash and measurement sources; `budget-proc-reference/daemon_budget/`
  holds all replacement pairs and their JSONL verdicts.
- `budget-final/daemon_budget/t{10,100}_s{1,8}.jsonl`: retained subscriber windows.
- `budget-command-final/development_budget/budget.jsonl` and
  `budget-proc-reference-command/development_budget_reference/budget_reference.jsonl`:
  both driver exits 0, no background panic or drain failure.
- `daemon-budget.json` and `budget/`: earlier failed floating-point boundary
  comparison, retained rather than overwritten. `daemon-budget-final.json`
  retains the pre-correction reference RSS method and is superseded by the
  qualified artifact.

Measurement scope: the binary includes the event-driven observer and its late
handle/resolver notification fixes. Two later cold-path fixes quote client names
in overflow logs and defer startup snapshot deadlines; they are covered by debug
unit/live tests and are not exercised by these settled paused workloads. These
numbers qualify the P1-08 bus, before later daemon policy and CLI track changes.
