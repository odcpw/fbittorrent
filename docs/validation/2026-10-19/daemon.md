# Daemon track

Commands below use the private-XDG runner `target/test-logs/<bead-id>/run.py`;
all application launches prove HOME/TMPDIR/five XDG paths and download isolation.
No all.sh between beads, per the owner track policy.

## P1-08 — event bus (`fbt-p1-08-xyp`)

Commits `d44d4cd` (implementation), `ae508cd` (live replay test), `e47c2a5`
(budget/evidence). Added consistent snapshots/deltas, revision tracking, event
replay, bounded slow-reader shutdown, engine observers and restart/detail cleanup.
Split the inherited server below 900 lines; engine/fork hooks, app subscription
counts, harness/resource gate and README changes are additive. Evidence paths in
this section are relative to `target/test-logs/fbt-p1-08-xyp/`.

| Criterion | Command / evidence | Result |
|---|---|---|
| Subscribe fixtures / recovery | `run.py api`; `run.py cases bus_subscriptions subscribe_gap_recovery`; `api-final/`, `event-driven-{live,systemd-final}/daemon/` | **PARTIAL**: frozen decoder/reducer and whole live typed transcript pass; losing a real frame causes unsubscribe/resubscribe and a fresh snapshot in both modes. Exact canonical live replay remains unmet. |
| Slow reader / healthy peers | `cases slow_client_closed_others_live`; `escaped-client-live/`, `event-driven-systemd-final/` | PASS: real queue reaches 512, bye/EOF, healthy pause delta continues; newline name remains on one log line. |
| Ten-minute revisions | `FBT_TEST_SOAK=1 FBT_BUS_BINARY=target/release-perf/fbittorrent run.py cases rev_soak_10min`; `soak-final/daemon/` | PASS: 600 s soak, final rev 652, no gaps/interleaved snapshots; case 608.266 s. |
| Matrix / bar budget | `run.py budget`, `run.py budget_reference`; `daemon-budget-qualified.json`, `budget-final/`, `budget-proc-reference/` | PASS: three runs each at 10/100 × 0/1/8; bar increments 0.00/0.02 s/min ≤ references 0.02/0.02. Full method/table: `daemon-budget.md`. |

Supporting tests: full Rust 281 passed/two intentional ignores (`final-unit-suite/`)
preceded the late observer fixes; final API 62/one bridge ignored (`api-final/`),
observer 3 (`observer-module-final/`), real fork 1 (`fork-tests/`), eight live cases
per mode (`event-driven-*`, `event-replay-{live,systemd}`), final startup fix
(`transitional-live/`), Python logger/CI/budget math (`verified-gates/`,
`daemon-gate-tests/`, `budget-exact-arithmetic/`) all pass. P1-12's full run below
also rechecks the final engine/API code. Earlier failed runs remain archived.

Unmet: canonical live `subscribe.ndjson`/`subscribe-gap.ndjson` replay under the
unchanged P1-06 normalization table: no identity rules cover hash-bearing topic
strings, changed-map keys or removed values; examples also require future
policy/watch/limits fields and exact counters. The retained
`event-driven-systemd-final/daemon/bus_subscriptions.live.ndjson` validates frozen
types but is not claimed as exact replay. No fixture, validator, mask or IPC doc
was weakened. Follow-ups: settle those replay preconditions/rules; rerun the
integrated resource gate after policy/CLI migration. Inherited P1-06 active-check
and dynamic-history disagreements remain outside this change.

## P1-12 — stop rule and idle net (`fbt-p1-12-pmn`)

Added default 300/minimum 30/zero-off config and CLI override, transfer/restore/
mutation/search/presence busy inputs, bar exclusion, glance pause/resume/drop,
summary countdown and bye-idle drain. Owner shutdown preserves durable run intent.
CLI declarations and RSS/Shows handlers moved out of main (886 lines); original
bodies/tests unchanged. Minimal API/engine hooks and agent/README updates included.
Paths below are relative to `target/test-logs/fbt-p1-12-pmn/`. The `run.py cases`
commands name all ten `idle_exit_*` cases in `daemon_policy.py`, once with
`FBT_BUS_RUNNAME=policy-live`, once with `FBT_BUS_SYSTEMD=1 FBT_BUS_RUNNAME=policy-systemd`.

| Criterion | Case / JSONL under both `policy-{live,systemd}/daemon/` | Result |
|---|---|---|
| Empty exits | `idle_exit_no_torrents.jsonl` | PASS: bye idle, exit 0, both sockets removed. |
| Local download stays | `idle_exit_download_keeps_alive.jsonl` | PASS: real 8 MiB seed/leecher, 32768 B/s cap, verified bytes and positive rate; survives 4 s with 2 s idle delay. |
| Interactive stays | `idle_exit_interactive_keeps_alive.jsonl` | PASS: named reason, no countdown, owner shutdown drains. |
| Bar alone exits | `idle_exit_bar_does_not.jsonl` | PASS: bar interactive=true is ignored; bye idle, exit 0. |
| Parked download stays | `idle_exit_parked_keeps_alive.jsonl` | PASS: missing private payload + run intent, missing interface; named parked_waiting reason, alive after 4 s. |
| Zero off / shutdown drains | `idle_exit_disabled_{never_exits,shutdown_drains}.jsonl` | PASS: no fake busy/countdown; shutdown drains, durable paused=false survives. |
| Busy reasons name work | Above + `idle_exit_mutation_keeps_alive.jsonl` | PASS: downloading, seeding, interactive, parked_waiting, real held loopback URL mutation; search count unit-tested for P3-01. |

Glance cases also pass: fresh reads freeze at 20 s across a 5 s pause, resume at
20 s, exit after 20.728/20.742 s including drain; socket drop resumes the short
countdown. All 20 cases have conforming JSONL and clean owned process/unit cleanup.
`run.py check`: exit 0 (`initial-check/`); `run.py policy`: 6 passed (`policy-units/`);
`run.py full`: **292 passed, 0 failed, 2 intentional bridge/probe ignores**, 7.94 s
(`final-full/development_full/full.{jsonl,stdout.log}`). Unmet for P1-12: none;
UI attachment/quit verbs remain P1-15. Handoffs: P1-09 asynchronous mutation workers
must retain the lifetime guard; P3-01 must update the live/grace-period search count.
