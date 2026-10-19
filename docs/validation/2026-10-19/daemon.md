# Daemon track

## P1-08 — event bus (`fbt-p1-08-xyp`)

Implementation: `d44d4cd`, live replay regression `ae508cd`; final evidence commit
follows this section. Added bounded subscriptions, consistent chunked snapshots,
field deltas/null deletion, an instance event ring, structural engine observers,
restart/detail cleanup and resource tooling. Split the inherited API server into
modules; all changed application Rust files are below 900 lines. Small neighbouring
changes expose engine sampling/admission notifications, use live subscription
counts, advertise subscribe and wire the opt-in resource gate.

All paths below are relative to `target/test-logs/fbt-p1-08-xyp/`.

| Criterion | Command / artifact | Observed result |
|---|---|---|
| Subscribe fixtures / gap recovery | `run.py api`; `run.py cases bus_subscriptions subscribe_gap_recovery`; `api-final/`, `event-driven-live/daemon/`, `event-driven-systemd-final/daemon/` | **PARTIAL**: frozen fixture decoder/reducer tests and complete six-topic live typed transcript pass; losing a real frame triggers unsubscribe/resubscribe and a fresh consistent snapshot in both modes. Full canonical live example replay is not claimed (see below). |
| Slow reader closes; others live | `run.py cases slow_client_closed_others_live`; `escaped-client-live/daemon/`, `event-driven-systemd-final/daemon/` | PASS: actual queue reaches 512, bye/EOF, healthy pause delta continues; newline client name stays on one log line. |
| Ten-minute consecutive rev | `FBT_TEST_SOAK=1 FBT_BUS_BINARY=target/release-perf/fbittorrent run.py cases rev_soak_10min`; `soak-final/daemon/rev_soak_10min.{jsonl,frames.ndjson}` | PASS: 608.266 s case, 600 s soak, final rev 652, zero gaps or interleaved snapshots. |
| Resource matrix / bar criterion | `run.py budget`, `run.py budget_reference`; `daemon-budget-qualified.json`, `budget-final/`, `budget-proc-reference/`; `daemon-budget.md` | PASS: complete three-run 10/100 × 0/1/8 matrix; 100×8 CPU 0.26/0.25/0.27 s/min, RSS 20876/20612/20492 KiB. Median bar increments 0.00/0.02 s ≤ references 0.02/0.02 s. |

Supporting tests: `run.py full` passed 281 tests (two intentional subprocess/bridge
ignores) in `final-unit-suite/` before three late observer regressions were added.
The final touched modules passed API 62/one bridge ignored (`api-final/`), engine
observer 3 (`observer-module-final/`), real fork transition test 1 (`fork-tests/`),
and eight live cases per foreground/systemd mode (seven in `event-driven-live/`
and `event-driven-systemd-final/`, plus `event-replay-{live,systemd}/`). The restart
case also passed after the startup-deadline fix (`transitional-live/`). Python
logging, CI dispatch and budget-boundary regressions passed (`verified-gates/`,
`daemon-gate-tests/`, `budget-exact-arithmetic/`). Earlier failed observer and
measurement runs are retained separately. No all.sh rerun under the track policy.

Unmet: full canonical live replay of `subscribe.ndjson` (the actual frozen filename,
rather than the bead's `subscribe-torrents.ndjson`) and `subscribe-gap.ndjson` under
P1-06's unchanged normalization table. It has no identity rules for hash-bearing
`topic` strings, `changed` map keys or `removed` values, and the subscription
examples also require future policy/watch/limits fields and specific transfer
counters. The current live six-topic transcript is retained as
`event-driven-systemd-final/daemon/bus_subscriptions.live.ndjson`; it validates
against the frozen types, but is not represented as an exact canonical replay.
No fixture, validator, normalization mask or IPC document was changed to hide
this distinction. A scope/normalization ruling remains outstanding.

Follow-ups: resolve canonical subscription replay normalization/future-feature
preconditions; run the integrated resource gate again after policy/CLI migration.
The inherited P1-06 live active-check/history disagreements remain outside this
change. Continue with P1-12; P1-11 awaits P1-09 and P1-12/P1-14/P1-17.
