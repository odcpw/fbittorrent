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

## P1-08 / P1-12 — reconciliation with main's server and CLI modules

Kept main's canonical connection/dispatch/framing/mutation-flow modules, ported
subscription routing, bounded EventSink extensions, bus lifecycle, presence and
idle override, and removed the superseded duplicate source modules. Consolidated
hello capabilities and the daemon case registry. P1-09 queued workers and pending
confirmations now retain the idle guard, and confirmed mutations use the bus's
single event sequence. A new real case verifies direct delivery, subscribed
history, idempotent replay and since-sequence replay agree exactly. The live
transcript adapter retains each subscription's real revision history when calling
the unchanged frozen checker; a regression accepts the canonical consecutive
sequence and rejects a gap. No contract fixture or validator changed.

Commands use `target/test-logs/track-daemon-reconcile/run.py`; evidence is relative
to that directory. `check` (`check-fixed/`) passes `cargo check --locked --tests`;
`api` (`adapter-units/`) passes 77 module tests with one intentional bridge ignore;
`build` (`debug-build/`) passes. All nine bus cases and ten P1-12 cases pass in
foreground (`foreground/`, with the corrected transcript case in
`foreground-adapter/`) and systemd (`systemd/`), including verified private XDG
state and owned-process/unit cleanup. `full` (`full-final/`) passes the single
full `cargo test --locked -- --nocapture`: 326 passed, zero failed, two intentional
ignores. Every changed Rust source is below 900 lines. The earlier parser-phase
and transcript-context failures remain archived. No all.sh between track beads,
per owner policy. Unmet for this port: none. The existing P1-08 literal canonical
live replay limitation above remains recorded; no additional claim is made here.

## P1-13 — daemon notifications and the icon (`fbt-p1-13-7tx`)

Added a bounded 512-waiting/four-worker notifier, pending-key collapse, markup
escaping, action argv, cancellation/group cleanup, config/flag suppression and
VPN transition limiting at the owner. The engine emits completion exactly when
it first persists completion_on (including immediately completed records), with
the resolved metadata name and content target. Handler-origin adds receive the
daemon's Undo/Open-window toast; Undo is a normal API mutation with a unique ref
and keeps files. Manual CLI adds notify only on a TTY or --notify; JSON, quiet and
agent callers suppress that toast. Setup installs the monochrome download SVG
inside the active XDG data home and uses it in both desktop entries and toasts.
Mechanical neighbouring edits thread origin through legacy AddOptions until
P1-11, initialize the engine/test event queue, and report failed handler adds from
the mutation receipt path. No frozen contract changes or live desktop/state edits.

Evidence is under `target/test-logs/fbt-p1-13-7tx/`; commands use its private-XDG
`run.py`. `cases` names the notify_* functions plus `setup_desktop_handlers`, in
`foreground-verified/` and `FBT_BUS_SYSTEMD=1` in `systemd/`.

| Acceptance criterion | Command / JSONL in both modes | Observed result |
|---|---|---|
| Correct notification argv/actions | `cases notify_completed notify_failed notify_handler_cli_once`; `daemon/<case>.jsonl` plus `.notify-send.argv.json` | PASS: one real loopback magnet completion with exact bytes; default/open/folder actions; failed add log action; one actioned handler toast. |
| Open action receives the right path | `cases notify_action_open notify_action_default notify_action_folder`; `.xdg-open.argv.json` | PASS: exact completed file / containing directory. |
| Undo removes, retaining data | `cases notify_added_undo` | PASS: API removes only the record and preserves every payload byte. |
| notify=false suppresses all | `cases notify_off notify_cli_policy`; `notify_off_{config,flag}.jsonl` | PASS: config false and --no-notify suppress completion/failure; non-TTY/JSON/quiet/agent adds have no plain toast; explicit --notify has one. |
| Three VPN transitions in 2s → ≤2 notices | `cases notify_vpn_once_per_transition` | PASS: systemd 0.192 s; two notices, configured interface named. |

Additional cases: forty actual completions deliver forty notices, measured peak
four concurrent children and zero remaining; killed action handler never opens;
restoring a completed ledger never repeats its completion; icon and desktop
handler installation are verified inside private XDG. `policy` unit filter
(`notifier-units-final/`): two passed (escaping, collapse/cap). `api`
(`module-tests/`): 77 passed/one bridge ignored. `engine_watch` queue filter
(`engine-queue-final/`): two passed. Single full `cargo test --locked -- --nocapture`
(`full-final/`): **328 passed, zero failed, two intentional ignores**, 7.95 s.
Earlier failures are retained. No all.sh between track beads per owner policy.
Unmet acceptance criteria: none. Follow-ups: Open-log is wired to the CLI log
verb supplied by P1-14; actual desktop notification rendering/action selection
remains outside these recording-shim checks. P1-14 keeps the existing app.info counts cumulative (connections since start),
as its canonical rule requires.

## Integration batch 2 — merge `6ec496e`

Reconciled main's canonical CLI/server/TUI/settings modules with the bus, idle
policy and notifications. Private-XDG checks in `target/test-logs/track-a-batch2/`:
`run.py check` and Python harness compilation passed; all 38 bus/idle/notification/
settings/TUI cases passed in each launch mode; `run.py full` passed 359 tests,
zero failures, two intentional ignores. Main's structural layout is retained.

## P1-14 — log ring and connection usage (`fbt-p1-14-bqf`)

Added the 2,000-entry stderr-fed ring while preserving original diagnostic bytes,
filtered/cursor API history, bounded append-only log subscriptions, read-only
`log`/NDJSON follow and separate active/cumulative hello counters. Production no
longer writes last-error.txt. Added the authorized app.log/app.stats contract
extension, independent log transcript and capability/version bookkeeping; all ten
existing transcript SHA-256 values remain unchanged. Minimal neighbouring edits
let the TUI's all-fixture mirror consume log snapshots/deltas within the same
2,000-entry bound, and its real one-connection test now reads active app.stats
counts while retaining every original equality assertion.

Evidence root: `target/test-logs/fbt-p1-14-bqf/`; private-XDG commands use its
`run.py`. `policy` (`ring-units/`) passed all three named ring tests; `api`
(`module-tests-fixed/`) passed 86 tests with one bridge ignore; debug `build`
(`final-build/`) passed. `cases log_injected_error log_follow hello_counters
log_down_never_starts tui_single_connection` passed five cases in each mode
(`foreground-final/daemon/`, `FBT_BUS_SYSTEMD=1` in `systemd/daemon/`): one real
injected engine failure with its hash/source through CLI JSON and original stderr,
no last-error file, exact per-kind cumulative increments and live detach counts,
follow entry within one second and clean exit on owner stop, closed read exits 4
without starting anything, real TUI uses one API connection. `full`
(`full-fixed/development_full/`) passes the full locked suite (363 passed, zero failed, two intentional
ignores), including all fixtures and the added bounded mirror regression; the earlier full failure that
identified the unsupported TUI log topic is retained in `full-final/`. Python
compilation, diff whitespace and changed-source 900-line checks pass. No all.sh
between track beads per owner instruction. Unmet criteria: none. Follow-up:
notification Open-log still needs a visible terminal/log pane; these checks prove
CLI history/stream delivery, not desktop presentation.

## P1-17 — upgrade detection and restart (`fbt-p1-17-2ds`)

Added build-time git/dirty identity with the fixture override, owner executable
identity checks every 30 seconds, busy-table deferral and live upgrade_pending,
exit-75 replacement, and a restart CLI that requests daemon scope and waits up to
30 seconds for a different hello instance. A foreground owner exits 75; without
a supervisor the CLI times out with exit 8 and never launches an owner itself.
Existing P1-11 skew warnings remain verified. Neighbouring edits publish pending
state in app.info/summary and route the split CLI to the bounded restart helper.
No existing contract fixture changed; contract patch 1.2.4 records new restart
instance/server output.

Evidence: `target/test-logs/fbt-p1-17-2ds/`; commands use private-XDG `run.py`.
`policy` (`upgrade-units/`) passes two upgrade unit tests. `cases
upgrade_idle_exit_75 upgrade_pending_while_busy restart_now
restart_closed_never_starts` passes all four cases in `foreground/daemon/` and
`FBT_BUS_SYSTEMD=1` in `systemd-fixed/daemon/`: two actual locked builds differ by
compiled p1-17-old/new ids; private copy/rename changes the inode; idle replacement
records systemd SubState=auto-restart and ExecMainStatus=75 then a new build/instance;
a real 8 MiB capped loopback transfer retains the old instance with pending=true
until stopped; explicit restart overrides an interactive busy owner, while the
foreground wait expires cleanly after 30 seconds. Replacement unit environment
and working directory are verified before launch and again in the actual child.
The original systemd fixture checked lock freedom before stopping its replacement
owner; that failure is retained in `systemd/` and corrected by checking after
verified unit/cgroup teardown. `skew.py` (`skew/agent_contract/`) passes the
existing human/JSON version-skew test. Single `full` (`full-final/development_full/`)
passes 365 locked Rust tests, zero failures, two intentional ignores. Python compilation, diff whitespace and changed
source-size checks pass. No all.sh between track beads per owner instruction.
Unmet criteria: none. Follow-up: actual desktop banners/installation remain the
integrated milestone's qualification, outside these isolated unit/process checks.

## P1-18 — events and raw API (`fbt-p1-18-s18`)

Added bounded event replay/follow with instance/seq, filters, explicit resync,
EOF/SIGINT detachment; raw response/error envelopes, typed arguments, stable raw
client identity for cross-process refs, per-op startup/retry policy; a compiled
args/result catalogue shared by api --list, robot-docs api and capabilities. A
unit checks every IPC operation row/argument/result/mutating column. No existing
fixture bytes changed. Neighbouring client plumbing retains transport envelopes
and names across reconnect; the old unit initializer only adds name=None. The
live shape bridge applies actual cli::shapes::rows, rather than duplicating it in
Python. Documentation corrects stale restart/exit-75 wording.

Private-XDG evidence: `target/test-logs/fbt-p1-18-s18/`; `run.py check`
(`initial-check/`) passed; `api` (`module-tests-fixed/`) passed 88/one bridge
ignore; `policy` (`raw-units-fixed/`) passed the argument unit. `agent
 events_follow_completion events_since_seq_replay events_resync_when_older
 events_down_exit_4 api_list_matches api_list_matches_ipc_md api_unknown_op_hint
 api_mutating_starts_daemon_read_does_not` passed eight real cases in each mode
(`foreground/agent_contract/`, `FBT_BUS_SYSTEMD=1` in `systemd/agent_contract/`).
Completion arrived in 0.855/0.804 s with exact real loopback bytes; killing the
reader then resuming prints exactly the two later additions; 1,002 real parked
ledger additions force actual 1,000-entry ring eviction and explicit oldest_seq;
raw A.9 objects equal the actual Rust shape conversion; raw typed failures retain
field details; explicit refs replay identical mutation results across processes;
raw read never starts, write starts the verified private static unit. The final
catalogue also works with an invalid private config (`catalog-final/` and
`catalog-systemd-final/`). `build` (`final-build/`) and single full `cargo test
--locked -- --nocapture` (`full-final/`) pass: 368 tests, zero failures, three
intentional process-bridge ignores (the new shape bridge ran in both live cases).
Python, whitespace, unchanged-fixture and <=900 source-line checks pass.

Literal bead deviations: planned contract 1.2.1 was already superseded by
integrated features; the required monotonic patch is 1.2.5, with separate events
and api changes entries. No catalogue operation is within Levenshtein distance 2
of literal `nope`; it exits 2/unknown_op with nearest=[] and api --list hint,
while the actual distance-two typo torrents.lsit suggests torrents.list. Neither
an old version nor a false neighbour is fabricated. Other criteria met. No
all.sh between track beads per owner policy. Follow-up: app.log queries with many
maximum-length messages need a byte budget beneath the 1 MiB response cap.

## P1-07b — real fixture serving (`fbt-y8i`)

Added idle/seeding/downloading/paused JSON recipes (empty aliases idle), --throttle,
FBTFIXTURE socket/summary/resource metadata and a managed seed/leech context. An
initially closed stdin retains the fixture; closing an initially open pipe ends
it for UI callers. A parent-death guardian drains daemons and removes private
sandboxes after launcher SIGKILL or daemon death. Child evidence roots are unique
per invocation, including --log-root forwarding. Replaced the interim TUI engine
host/probe with real serve/API v1; cap observers serialize one typed connection,
retain their timing/rate/metainfo assertions and observe normalized states.
Neighbouring visual fixture sandboxes now use the retaining subclass. The hello
counter test now establishes separate exact active/cumulative baselines after
API startup probes retire, and checks live per-kind increments too.

Private-XDG `target/test-logs/fbt-y8i/run.py cases fixture_serving_states
fixture_launcher_killed fixture_daemon_killed fixture_stdin_lifetime
fixture_peer_pair daemon_api_probe cap_change_no_check seed_toggle_no_check
launcher_exit_cleanup hello_counters` passes ten cases in foreground-final/
(37.037 s) and FBT_BUS_SYSTEMD=1 systemd-final/ (41.073 s); both retain eight
independent daemon_fixture worker JSONLs and verify no unit/PID/sandbox survives.
Explicit 65536-B/s fixture flag, real capped loopback bytes, SIGKILL launcher,
SIGKILL daemon, EOF, closed stdin, metadata identity and seed/noseed transitions
all pass. The corrected counter test also passes counter-final/. Initial API
argument/status assumptions and refusal-shim manager-query failures are retained.
Python compilation and diff checks pass; single full cargo test --locked
(full-final/) passes 368, zero failures, three intentional process bridges ignored.

Unmet: the *entire expanded* daemon.py suite does not fit three minutes. One
--no-systemd invocation reached 48 completed cases/250.872 logged seconds before
interrupt at 252.346 s (all-cases-driver/ and archived all-cases/processes/).
Interruption drained its owned process and removed its sandbox, verified after
exit; this is not a green whole-suite claim. It found the counter-baseline issue
fixed above. Systemd whole-suite timing remains unqualified; the focused batch is
green in both modes. Global transfer.set_limits is absent until P5-04, so only
the marked DaemonContract.throttle adapter temporarily uses the existing typed
single-seed upload cap; that Do-step remains deferred. No all.sh between track
beads. Follow-ups: wire that adapter when P5-04 lands; restore the expanded CI
runtime budget without deleting or shortening coverage.

## P1-16b — dormant daemon integration (`fbt-awy`)

The four frozen acceptance criteria pass in both launch modes. A bounded test-only
runtime gate installs the torrents subscription before READY=1; five previously
verified completed records list done/unchecked in 30.5–95.7 ms, with no checking
state or matching initialization diagnostic. Explicit CLI seed has exactly one
real validation-admission diagnostic and seeds only its record. Missing files
stay saved done until acted on, then give invalid_state/missing_files and an
unchecked error row. Open_target takes 1.45/1.41 ms and stays dormant; CLI open,
nonempty file inclusion and typed CLI delete-files are also exercised. The delete
check exposed the live-handle precondition: dormant deletion now uses saved
metainfo, persists record removal before side effects, and unlinks exact relative
files through directory descriptors; parent symlinks cannot redirect deletion.
Three new units cover no admission, failed-save/no-deletion and symlink boundaries.

Evidence root `target/test-logs/fbt-awy/`: private `run.py check` passes;
`policy` dormant filter (`dormant-units-final/`) passes 11; `engine_ops` passes
existing typed operations; `restore_unit` passes the named restore test. Grouped
`cli_cases lazy_restore_no_checks dormant_seed_checks dormant_missing_files
dormant_open_no_check` passes four real cases in foreground-final/daemon/ and
FBT_BUS_SYSTEMD=1 systemd-final/daemon/. The initial delete refusal remains in
foreground/ and systemd/. CI dispatcher regressions (`ci-dispatch/`, python-ci/)
pass four cases; these run stub commands, not a live full gate. all.sh schedules
the four real cases in one process and summarizes their daemon suite. Python,
shell syntax, whitespace and <=900 source-line checks pass. Single full locked
Rust (`full-final/`): 371 passed, zero failed, three process bridges ignored.

Do-step remainder: torrents.recheck belongs to Track B's canonical asynchronous
P5-03 contract (hashes, pending, mutation.result), available on integration/Track B
but absent from this branch; it was not replaced with a conflicting synchronous route. Merge that
track before claiming all of Do 2. No fixture byte changes, all.sh run between
track beads, or owner installation. Follow-up: reconcile/replay the P5-03 hook
when the orchestrator merges that work.


## Track dependency boundary

Current branch is clean at the P1-16b checkpoint. Per the track skip rule:
P5-04 and P5-07 await P2-03/P3-09; P6-09 awaits P3-15; P6-13 awaits P2-03.
These dependencies have no implementing bead commits in this branch's ancestry.
P2-03/P3-09 and canonical P5-03 are available on other track/integration refs;
main itself remains at 3a4eba8. No agent merge/rebase/branch switch was performed.
On integration, new operation-table rows also need P1-18's args/result catalogue
metadata. Keep the canonical server layout and the dormant delete-files path.
M1a installation has not been performed; owner paths/binary remain untouched.

## P5-04 — daemon/API/CLI checkpoint (fbt-p5-04-ipg)

Added persisted `transfer.set_limits`/`transfer.set_alt`, tri-state updates,
Summary.alt/limits, cap `limits`, offline/live CLI with K/M suffixes and contract
1.2.6/new limits.ndjson. Existing live session limiters/settings transactions are
reused; existing fixture bytes stay unchanged. UI criteria: **deferred to track C/D**.

Evidence: `target/test-logs/fbt-p5-04-ipg/`. Its `run.py` proves private
HOME/TMPDIR/XDG/downloads. `check`, `api`, `policy` pass (`module-fixed/`);
one full `cargo test --locked -- --nocapture` passes **373/0/3 ignored**
(`full-rust-final/`). `cases alt_limit_holds global_unlimited_keeps_cap alt_survives_restart`
and `agent limits_verb` pass in both modes (`final-foreground/`, `final-systemd/`,
`final-agent-{foreground,systemd}/`). Over 20 s after 5 s warm-up, alternative
**upload** yields 65,525.8/65,527.4 bytes/s; alt pushes take 96.3/101.7 ms.
Global upload zero retains the torrent's 32768 cap; turtle survives restart.

The global API now throttles fixtures. Four isolated case workers preserve all
JSONL/stderr and failures; real CLI/multiple-case/artifact/forced-failure regression
passes (`worker-tests-artifacts/`, `worker-regression/`). Source-fingerprinted,
SHA256-checked real upgrade binaries are prepared once (`prepare`,
`upgrade-preparation-fixed/`: cold build 205.8 s, outside the timed matrix).
Full foreground/systemd runs schedule 114/132 entries in **125.65/147.75 s**,
but are **not green** (`all-{foreground,systemd}/processes/daemon/`). Subsequent
focused repairs pass: live mode for optional-field traces, explicit soak skip
(`replay-fix-focused/`); loopback IMDb env forwarded before systemd launch,
incomplete Undo fixture preserving all owned bytes, canonical P1-11 exits
(`systemd-repairs/`); recording shim limited to the owned unit's show/stop
(`concurrent-down-final/`). Before the IMDb fix, failed systemd cases contacted
public IMDb endpoints; every HOME/XDG/cache path remained private.

Unmet: frozen hello/error replay requires exactly four caps; live advertises nine.
P1-06 normalization says “no extra masks”; ruling requested before changing it.
No full-suite green/all.sh claim. Follow-up: an initial download-direction live
reduction probe exceeded the window while admitted requests drained
(`live-foreground/daemon/alt_limit_holds.jsonl`); final bandwidth proof measures
upload explicitly. P5-07 is not started: its 64 MiB recovery fixture cannot satisfy
`free > remaining + 512 MiB`, and that frozen-rule ruling is also pending.

## Orchestrator rulings, 2026-10-19

Hello/error capability arrays follow the live list for merged additive features;
update the fixture and changes[] bookkeeping, keeping replay comparison exact.
P5-07's frozen 64 MiB/100 MiB recovery acceptance wins over Do 6's impossible
margin: warning stays free < needed + 512 MiB; sample every 5 s, retry after free
rises by max(16 MiB, one piece) from that torrent's ENOSPC baseline, once per
rise; repeated ENOSPC resets its baseline. Log disk_full_recovered; explicit start
retries immediately. UI criteria stay deferred to track C/D.

P5-04 cap ruling applied: hello/error fixture capability arrays and the documented
hello example now equal the live list, exactly. Contract 1.2.7 records each added
cap via changes[].cap; agent unit test pins fixture equality and this coverage.
`run.py api`, `run.py caps` pass (`caps-ruling/`); `cases contract_hello contract_errors`
passes foreground and systemd (`caps-replay-{foreground,systemd}/`). No replay
mask or deferral was added. Earlier full-suite failure is resolved for these rows;
no additional full-suite run is claimed for this targeted bookkeeping delta.

At approximately 10:33 CEST the worktree's entire ignored target tree disappeared
while P5-07 was being edited; cause not established. Committed source is intact,
but earlier raw target/test-logs artifacts are unavailable. SBH was observed
running in enforce mode under disk pressure; this does not by itself establish
causation. The recreated target tree and /tmp/fbt-track-a-evidence are protected;
fresh logs are mirrored there. Earlier numerical results remain recorded above
and in the session output, not promoted into currently available raw evidence.

## P5-07 — daemon/API/CLI (fbt-p5-07-gei)

Added five-second per-path statvfs sampling, filesystem-aware Summary.disk,
ENOSPC classification, per-torrent recovery baselines under the ruling above,
one-per-life disk notification, immediate explicit start retry, and add capacity
warnings with binary units. Contract 1.2.8 adds DiskSpace/add.disk and disk.ndjson;
neighbouring bus, API add/result/dispatch, CLI and notifier changes carry these
fields without a new operation or cap. `python3 /tmp/fbt-track-a-run.py api`,
`units engine::disk`, `units supervisor::`, `build` pass (89 API tests, four policy
tests). `cases add_warns_disk disk_low_banner disk_full_reason disk_full_recovers`
passes all four real 64 MiB tmpfs / 100 MiB local-peer cases, including actual
ENOSPC, one window-action notice, warning before error, and increased allocated
blocks/downloaded bytes after truncating a 32 MiB filler with no start request.
Explicit start is acknowledged on the failed record. Evidence:
`target/test-logs/fbt-p5-07-gei/{module-tests,unit-disk,unit-supervisor,build,live-disk}/`;
the inner `live-disk/disk-workers/` JSONLs retain free/needed, row and retry-log
observations. The tests run foreground in an owned user/mount namespace because
a user unit outside it cannot see the tmpfs; no systemd-mode disk proof is claimed.
`full` passes 378/0/3 ignored (`full-final/`, cargo test --locked -- --nocapture).
Fresh exact hello/error replay passes both modes (`fresh-caps-{foreground,systemd}/`).
Every fresh run is mirrored under `/tmp/fbt-track-a-evidence/fbt-p5-07-gei/`.
UI banner/urgent icon/add-sheet refusal/goldens: deferred to track C/D. No all.sh
between track beads by owner instruction. Follow-up: no disk-capacity proof yet
for deferred URL/magnet metadata; these adds omit disk until size is known.

## P6-09 — daemon/API/CLI (fbt-p6-09-omx)

Added one condition-variable-driven inhibitor owner, observing run-intent
downloading/stalled/metadata/checking states with no allocations on unchanged
samples. It cleans its process group independently of launcher exit, releases
on last stop and drain, and retries failed launchers at most every five seconds
while eligible work remains. The child has an unblocked signal mask and keeps
the session bus environment without changing its XDG runtime directory. Existing
peer_port/settings --port paths are reused. Neighbour edits: bus observation,
daemon lifecycle wiring, fixture registry; peer-port.ndjson documents the existing
settings acknowledgement and engine.changed pair (no new wire field/cap/op).
`FBT_BEAD=fbt-p6-09-omx python3 /tmp/fbt-track-a-run.py check`,
`units inhibit::`, `units api::fixtures`, `build` pass (one injected-spawner
lifecycle test, 30 fixture tests). `full` passes **379/0/3 ignored** (`full-final/`).
The first live runs exposed inherited blocked SIGTERM; the final spawn-mask fix
was verified with the targeted unit, build and live cases after that full run.
`cases port_restart_keeps_clients inhibitor_lifecycle` passes foreground and
systemd (`final-{foreground,systemd}/daemon/`): same instance, surviving API
connection, two engine.changed frames, actual new loopback TCP listener, one
inhibitor during actual download, recorded SIGTERM and no live descendants after
stop/drain. All JSONL/stderr/timestamp traces live under
`target/test-logs/fbt-p6-09-omx/` and are mirrored under
`/tmp/fbt-track-a-evidence/fbt-p6-09-omx/`. Actual host sleep inhibition is not
exercised: the child is a recording shim in both private environments. Settings
sheet/F6 UI fields are deferred to tracks C/D. No all.sh between track beads.

P6-13 framing conflict found before implementation: installed
`/usr/share/man/man1/wl-clipboard.1.gz` says --watch automatically enables
--no-newline and runs the command with selection bytes on stdin. Therefore the
literal planned `wl-paste --watch cat` loses selection boundaries for ordinary
newline-free magnets. Requested an explicit ruling for a NUL-delimited
`wl-paste --watch sh -c 'cat; printf "\\0"'`, retaining one watcher and no
fbittorrent CLI per change. No P6-13 source edits before that ruling.

P6-13 ruling approved by the orchestrator: use
`wl-paste --watch sh -c 'cat; printf "\0"'` with one bounded NUL record per
selection, since magnets never contain NUL. Proceeding with the daemon half.

## P6-13 — daemon/API/CLI (fbt-p6-13-dxe)

Implemented one opt-in reader using the approved NUL wrapper, a 64 KiB record
bound (oversize discarded through delimiter; no partial offers), hash/ledger
deduplication, bounded offer names, notification Add/Dismiss over api.sock,
clipboard origin and idempotent app.clipboard.dismiss. Only the last offered
hash survives parsing; notification Add reconstructs a hash-only magnet, so
clipboard tracker URLs are not retained. Off/drain kills the complete owned
group; launch is serialized with disabling, signals are unblocked before exec,
and idle reads block in poll rather than spinning. Clipboard child/notifier
stderr is suppressed; no clipboard text enters daemon logs. Contract 1.2.9,
clipboard cap/changes entry, new clipboard.ndjson, exact hello/error cap updates,
and CLI on/off (also true/false) bookkeeping land together. Minimal neighbours:
typed call/reply/catalogue, dispatch/server event-bus accessor, add-origin
validation, CLI parser and notifier/lifecycle wiring.

`FBT_BEAD=fbt-p6-13-dxe python3 /tmp/fbt-track-a-run.py check`, `units clipboard::`,
`api`, `build` pass (two record/parser units, 89 API tests). `full` passes
**381/0/3 ignored** (`full-final/`, one full cargo test --locked -- --nocapture).
`cases clipboard_watch_offer clipboard_watch_add clipboard_watch_dedupe
clipboard_watch_off_kills_child clipboard_never_logged contract_hello contract_errors`
passes foreground (`final-foreground/`). Systemd offer/add/dedupe/privacy pass
(`final-systemd/`); corrected production-valid 30 s idle/drain plus exact hello/error
replay pass `idle-final-systemd/`, with the same idle case also passing
`idle-final-foreground/`. Tests use fake wl-paste but execute the real sh/cat NUL
wrapper, record selections/timestamps and notify argv, add a real ledger record,
exercise idempotent dismiss and compare both stderr/app.log against fixed fixture
text. Both reader and notifier deliberately emit that fixture text to stderr;
final scans are empty. Earlier failures retain the in-flight old build's stderr
leak and the short idle setup race; no verdicts were suppressed. All evidence
is under `target/test-logs/fbt-p6-13-dxe/`, mirrored in
`/tmp/fbt-track-a-evidence/fbt-p6-13-dxe/`. UI strips/banner/golden and a real
Wayland clipboard/day-long owner evaluation remain deferred to tracks C/D/owner.
No all.sh between track beads. Follow-up: hash-only notification adds rely on
ordinary peer discovery rather than retaining clipboard tracker parameters.

## P3-02 — RSS daemon/API (fbt-p3-02-n95)

Added eight typed RSS operations, feed/item objects, keyed on-change topics,
an owned coalescing stale-on-interactive scheduler, cached views invalidated by
inode/size/mtime, 4 MiB feed reads, persistent grabbed history and rss-origin adds.
Checks never add torrents. A grab marks read/grabbed only after an accepted real
engine add. Feed-id removal is exact, preserving nested display names. The
in-process CLI still uses the shared locked/atomic store. Contract 1.2.10 adds
rss cap/changes and rss.ndjson; hello/error arrays compare exactly with live caps.
Neighbours: API types/catalogue/dispatch/framing, bus/diff, presence/drain hooks,
add-origin validation, and the TUI API mirror's typed RSS collections (rendering
is deferred; no UI completion claimed). Snapshot row/byte bounds are retained,
and only the selected feed's item table remains in that mirror.

`FBT_BEAD=fbt-p3-02-n95 python3 /tmp/fbt-track-a-run.py api`, `units rss::`,
`build` pass: 89 API tests and 13 RSS/store tests. All four
`cases rss_refresh_on_interactive rss_cli_edit_visible rss_oversized_refused
rss_grab_marks_read_only_on_success` pass both modes (`final-{foreground,systemd}/`):
counted local feed hits follow stale interactive edges only; CLI add pushes in
**103 ms** both modes; >4 MiB refresh returns pending then upstream/rss failure
while both cached items survive; failed grab leaves read/grabbed false, successful
local metainfo add persists rss origin and read/grabbed; nested-name removal and
CLI read-history interoperability pass. One full `cargo test --locked -- --nocapture`
ran (`full-final/`): **381 passed, 1 failed, 3 ignored**. The sole failure was the
new RSS snapshot encountering the TUI mirror's unsupported item type. It was
implemented without changing that test; final `units tui::store::` passes all
**12** tests including every transcript (`tui-mirror-final/`). No second full run
is claimed, following the one-full-run track cadence.

Exact hello replay passes both modes (`caps-{foreground,systemd}/`). Error replay
exposed a pre-existing pressure-fixture race: auxiliary URL replies crossed into
the next line-cap scenario. The harness now drains every admitted reply and
pending outcome after releasing its actual HTTP fixture; canonical expectations
are unchanged. Final `cases contract_errors` passes both modes
(`replay-fixed-{foreground,systemd}/`), with empty outstanding sets recorded.
Logs are under `target/test-logs/fbt-p3-02-n95/`, mirrored under
`/tmp/fbt-track-a-evidence/fbt-p3-02-n95/`. No all.sh between track beads.
Follow-up: the existing legacy error replay still records its older capability/
operation deferrals; this bead adds no new comparison mask or deferral.

## P3-03 — Shows daemon/API (fbt-p3-03-oyg)

Moved src/shows.rs mechanically into shows/mod.rs plus tests.rs (Git detects
69% rename; production prefix/test-body identity in `split-proof.json`; all
15 `shows::` tests pass). Applied the isomorphic skill's byte-preservation/API
path discipline within the owner's scoped move/test cadence; no separate
workspace, tracker ceremony, benchmarks or compile-memory claim. Added typed
Shows operations/views, keyed show/release topics (compound imdb/key inbox
identities), one stale-on-interactive worker, inode/size/mtime reload, cancellation
between bounded EZTV pages, and FBT_EZTV_BASE_URL shared by Shows and browse.
Read intent stays independent of ledger transfer state; paused intent takes
precedence for unresolved magnets. Single read marks establish one undo target;
bulk marking preserves it; undo consumes it; explicit grab alone adds and marks
read/grabbed with shows origin. Fresh interactive opens do not rewrite/check
history. Contract 1.2.11 adds shows cap/changes and shows.ndjson, with exact
hello/error caps. Minimal neighbours: types/catalogue/dispatch/presence/drain,
bus/diff, origin validation, browse endpoint, typed TUI API mirror (no renderer
completion claimed) and the CLI test helper's optional pre-log exit timestamp.

`FBT_BEAD=fbt-p3-03-oyg python3 /tmp/fbt-track-a-run.py check`, `api`,
`units tui::store::`, `build` pass (89 API tests; 12 mirror tests).
One full `cargo test --locked -- --nocapture` passes **382/0/3 ignored**
(`full-final/`), also confirming P3-02's RSS mirror integration is green.
`cases shows_inbox_semantics shows_check_never_downloads shows_cli_edit_visible
shows_check_only_when_asked` passes foreground (final inbox/check/CLI in
`final-foreground/`, idle in `live-foreground/`) and systemd (inbox/check/idle in
`final-systemd/`). Each idle case records **60 one-second zero-hit observations**,
then one stale interactive check, no fresh reconnect fetch and a deliberate
explicit check. Four unread releases are newest first; m/M/u semantics,
one-level undo, Shows-origin grab and independent paused transfer pass. CLI
follow through the in-process path appears in the topic; an independent socket
reader timestamps receipt against the CLI exit before JSONL writes, retaining
the exact <=2 s assertion: **85.7/94.2 ms** foreground/systemd
(`timed-cli-{foreground,systemd}/`). The previous poll-based systemd run timed
out after one observation under load; it remains retained, not counted green.
Final `cases contract_hello contract_errors` passes both modes (`caps-*/`).
Evidence: `target/test-logs/fbt-p3-03-oyg/`, mirrored in
`/tmp/fbt-track-a-evidence/fbt-p3-03-oyg/`. UI rendering stays with C/D; no all.sh
between track beads. Follow-up: legacy EZTV episodes do not distinguish a missing
source seed/size from zero; API omits nonpositive legacy values conservatively.

## P5-01 checkpoint — seed/peer engine and API (fbt-p5-01-c1a)

Connected seed flags ignore padding bits, expose actual encryption/direction,
and count seeds without allocating detail tables. Added typed `torrents.peers`,
addr-keyed detail snapshots/deltas with one-second counter baselines, bounded
caches and release after unsubscribe; CLI show and contract 1.2.12 bookkeeping.
`FBT_BEAD=fbt-p5-01-c1a python3 /tmp/fbt-track-a-run.py fork peer_stats_seed_flag`
passes with a real local seed and incomplete leecher, including six padding bits.
`build` passes. `api` was 90 pass/1 failure (stale documented caps); the example
was corrected and `checkpoint/source/documented_fixture_lines.jsonl` verifies
all examples against exact fixture lines. Logs under
`target/test-logs/fbt-p5-01-c1a/`. Intermediate compiling checkpoint: daemon
swarm cases, final fork suite and full app suite remain pending while the
owner-priority M1a dashboard-presence fix is taken next. No P5-01 closure claimed.

## M1a priority — visible dashboard presence (fbt-m1a-kug)

The shared bar/dashboard socket now declares negotiated
`app.client.dashboard_visible`; a bar counts as interactive only when that
marker and interactive are true. Hiding or disconnecting clears presence.
The original `idle_exit_bar_does_not` assertion (bar sends interactive true)
is unchanged. ApiClient declares visible state after connect/reconnect and
clears both flags when hidden, omitting the additive marker for older strict
servers. Contract 1.2.13 adds dashboard_presence, changes[], the new transcript,
and exact hello/error caps. Neighbours: request/server state, capability/docs
bookkeeping, plugin transport and its isolated Quickshell test.

`FBT_BEAD=fbt-m1a-kug python3 /tmp/fbt-track-a-run.py check`, `build`, and the
single `full` run pass: **384 passed, 0 failed, 3 ignored**. `cli_cases
 dashboard_bar_presence idle_exit_no_torrents idle_exit_download_keeps_alive
 idle_exit_interactive_keeps_alive idle_exit_bar_does_not
 idle_exit_parked_keeps_alive idle_exit_disabled_never_exits
 idle_exit_disabled_shutdown_drains idle_exit_glance_pauses
 idle_exit_glance_drop_resumes idle_exit_mutation_keeps_alive` passes **11/11**
in foreground (`foreground/`) and with `FBT_SYSTEMD=1` (`systemd/`). The visible
case holds for 4 s past a 2 s idle policy, reports interactive, clears it even
with a stale interactive=true after hiding, then receives bye:idle, exit 0 and
both sockets removed (~2.95 s in foreground). Bar-only, transfer, parked,
disabled, mutation and popup-glance rules all remain green. `qs` mode runs
`plugin/tests/run-qs.py client plugin/tests/client` with a private shell and
logs **13/13**, including visible-connect, hide and old-server declarations
(`qs-final/`; initial wrapper import setup failure is retained in `qs/`).
Evidence: `target/test-logs/fbt-m1a-kug/`, mirrored under
`/tmp/fbt-track-a-evidence/fbt-m1a-kug/`. No live shell or owner state was used;
no all.sh between track commits. All requested criteria met; no follow-up.

P5-01 acceptance after the priority repair: `FBT_BEAD=fbt-p5-01-c1a python3
/tmp/fbt-track-a-run.py cli_cases peers_local_swarm peer_rates_move contract_hello
contract_errors` passes **4/4 foreground and 4/4 systemd** (`foreground/`,
`systemd/`). The observer reports exactly two connected peers, seed flags
false/true and seeds=1; CLI show agrees; a paused torrent omits seeds. Real peer
counters and positive rates move in RPC and one-second addr-keyed deltas (one
observed sample 6,998 B/s foreground, 31,634 B/s systemd). `peers.ndjson` now
contains both seed and leecher; a minimal neighbour change keys the Quickshell
store by addr and handles the peer read result, with no Peers UI renderer added.
`qs store` passes **26/26** shared-corpus/store cases (`qs-store/`); `full` once
passes **384/0/3 ignored** (`full-final/`), including typed fixture round trips.
The required full fork command (`fork` without a filter) exits **0**, with
**139 passed/0 failed/3 ignored** across the two crates (`fork-full/`). Its
unchanged generic wrapper scan is retained **failed** because eight deliberate
existing panic-injection diagnostics from spawn_utils/{owner_tests,
owned_frame_tests}.rs match the blanket worker-panic expression; the focused
peer_stats_seed_flag test passes without a panic. No scan was weakened or
excluded. Follow-up for the orchestrator: classify deliberate fork fault
injection diagnostics without suppressing unexpected worker panics. All three
P5-01 acceptance criteria are met; a globally green wrapper scan is not claimed.
Evidence is under `target/test-logs/fbt-p5-01-c1a/`, mirrored in
`/tmp/fbt-track-a-evidence/fbt-p5-01-c1a/`. UI rendering remains P5-02/C/D.

## P7-06 — per-torrent download cap (fbt-p7-06-6mh)

Durable optional download_limit is installed before restore admission and
changed through the same live shared limiter without rebuilding. Added typed
idempotent API operation, configured/effective torrent fields, `limit <sel>
--down N[K|M]|none`, 256-hash client batches, explicit null clear and identity
validation even for an omitted/no-op bps. Contract 1.2.14 bookkeeping and exact
hello/error caps are complete. Today's records without the optional field keep
loading. Mechanical Record literal initializers include the new None field;
no new legacy implementation. Dashboard input is deferred to C/D.

`FBT_BEAD=fbt-p7-06-6mh python3 /tmp/fbt-track-a-run.py check`, `fork
 effective_rate_is_min`, final `build`, and single `full` pass (**386/0/3
ignored**). `cli_cases download_cap_holds contract_hello contract_errors`
passes 3/3 foreground/systemd (`foreground-final/`, `systemd-final/`); `agent
 limit_verb` passes in both modes (`agent-{foreground,systemd}-final/`). Every
one-second dl_bps sample satisfies the unchanged ±25% bound, after exactly 5 s
warmup over the 20 s window; independent byte rates are **65,527.09/65,531.31
B/s**. Live cap change remains downloading, no pushed checking state, same
engine epoch. CLI 0/2 exits, exact JSON, all/none, restart persistence and
omission/identity checks pass. Initial failed runs are retained: the new
operation needed null permission in the server argument filter, and the first
fixture measured payload requested before its cap existed. Final setup pauses,
installs the baseline cap, resumes, then keeps the same warmup/window/bounds;
the later change is still during active transfer. Minimal neighbours are wire
catalogue/validation/framing, CLI dispatch and typed shapes, and Record literal
initializers. Evidence: `target/test-logs/fbt-p7-06-6mh/`, mirrored in
`/tmp/fbt-track-a-evidence/fbt-p7-06-6mh/`. Both acceptance criteria met. One
Do-step ruling remains pending: the literal min of both configured primary and
alt conflicts with accepted P5-04's test requiring unlimited alt to override a
finite primary. Current code preserves that selection and reports min of the
per-torrent cap and the selected global/turtle bucket; no frozen P5-04 test was
changed. Owner clarification requested; independent suite-budget work continues.


### Owner rate-limit ruling (2026-10-19)

The active global rate is the alternative/turtle value while turtle is on and
otherwise the primary value. Effective per-torrent download/upload rate is the
minimum of the configured per-torrent rate and that active global rate; an
unlimited/unset value drops out. The default per-torrent upload cap remains
32 KiB/s. P7-06 already implements this rule; no rate code change was needed.
This supersedes the earlier pending primary-versus-alt question. The isolated
5 s warmup/20 s byte-window and turtle-minimum tests in the P7-06 evidence above
remain the observed proof. Suite wall-budget work is deferred until integration
brings main's complete harness into this branch; the interrupted pre-integration
baseline is retained as interrupted evidence, not a suite result.


### P7-04 — tracker add/remove, daemon/API/CLI half (fbt-p7-04-aq7)

Landed an ordered, watch-driven mutable tracker set in the fork, bounded live
monitor admission/removal (including an empty set), handle-preserving edits,
and durable `Record.trackers` additions plus removal tombstones merged with
metainfo on restore. The ledger is saved before publishing an edit; a forced
atomic-save failure proves the old live set and saved record remain intact.
Private torrents keep the first-metainfo-tracker policy and exclude public
session trackers. Tracker counts reflect edits for live and ledger-only reads.
Added strict/idempotent `torrents.add_tracker` and `torrents.remove_tracker`
(`http(s)`/`udp`, host required, UDP port required, 4096-byte URL bound), raw CLI
coverage, and query-redacted diagnostics. Contract 1.2.15 adds `tracker_edit`,
its changes[] entry, TABLE rows, documentation and tracker-edit.ndjson. Per the
owner's exact-cap ruling, existing hello/error capability arrays and the hello
doc example were updated to the live list; no comparison mask was introduced.
Neighbour edits are the one-line server error-key recovery for `url` plus its
regression, live/saved detail counts, mechanical Record initializers (including
the existing importer), and fork persistence readers of the changed set type.
No compatibility mode or adapter was added.

Evidence root: `target/test-logs/fbt-p7-04-aq7/` (mirrored under
`/tmp/fbt-track-a-evidence/fbt-p7-04-aq7/`). Commands below were executed through
`/tmp/fbt-track-a-run.py` with `FBT_BEAD=fbt-p7-04-aq7`, distinct FBT_RUN_NAME,
CARGO_BUILD_JOBS=4, the rch shim, and validated private HOME/TMPDIR/XDG/download
paths before any product process. All end-to-end steps retained E.1 JSONL.

- `cargo check --locked --tests`: exit 0; `check/development_check/check.jsonl`.
- `cargo test --locked -p librqbit-tracker-comms tracker_set_mutation -- --nocapture`:
  1 passed; `fork-set/development_trackerset/trackerset.jsonl` and stdout log.
- `cargo test --locked -p librqbit-core -p librqbit --features librqbit/rust-tls --lib private_ -- --nocapture`:
  4 fork + 2 core regressions passed; `fork-private/development_fork/fork.jsonl`.
- `cargo build --locked`: exit 0; `build-final/development_build/build.jsonl`.
- `daemon.py target/debug/fbittorrent --no-systemd --case tracker_add_remove --case contract_hello --case contract_errors`
  and the identical `--systemd` selection: 3/3 cases each, exit 0;
  `{foreground-final,systemd-final}/processes/daemon/*.jsonl`. Both modes:
  A stayed at 2 announces while newly added B advanced 2 -> 4 over two actual
  further cycles; the same removal survived a real daemon restart. Empty-set
  re-add, paused edits, idempotence, unknown hash, invalid URLs, raw CLI and
  failed-save rollback also passed. Actual received HTTP request inventories:
  `processes/workers/tracker_add_remove/daemon/tracker_add_remove.tracker-requests.json`.
- `cargo test --locked -- --nocapture` (one full invocation): **389 passed,
  0 failed, 3 existing opt-in tests ignored**;
  `full-final/development_full/full.jsonl`, stdout and daemon stderr logs.
  Includes the new typed URL, exact error-key and durable-record/count tests.
- `python3 -m py_compile .../daemon.py .../daemon_trackers.py`, `git diff --check`:
  exit 0. Every changed/new src file remains <=900 lines. No unexpected panic,
  obligation leak or drain failure in the final runs. No all.sh between track
  beads, per the owner's track policy.

The initial foreground/systemd evidence is retained: only the invalid-URL
`details.key` assertions failed (server returned `args`); the decoder was fixed
and assertions kept unchanged before the final green runs. UI +/x and confirmation
criteria are deferred to tracks C/D. P6-01 is absent on this branch, so the
`torrent:<h>.trackers` not_contacted/removed delta hook remains an explicit
integration follow-up, not claimed proven here. Both named announce acceptance
criteria pass. Main f086ed9 arrived after these edits were underway; this tested
commit must be included by the integrator before the branch can fast-forward
and the complete-harness wall-budget work can run.
