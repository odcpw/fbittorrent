# Isolated shell checks

The tests copy the plugin into a disposable configuration with private HOME,
TMPDIR and all five XDG directories. They never reload the installed shell.
`FBT_PLUGIN_SOURCE` selects a development copy; the default is this worktree's
plugin. Quickshell, Python, Node and the installed read-only Omarchy components
are required.

- `qs-client.sh`, `qs-store.sh`: API session and frozen transcript reducer checks.
- `qs-glyphs.sh`, `qs-format.sh`: actual Qt text/format/contrast checks.
- `contrast.py --self-test`: checked-in theme oracle and known-bad control.
  `--installed` additionally checks current installed themes; `--refresh` explicitly
  copies them and the owner's current colors into `contracts/ui/themes/`.
- `service-tests.py <debug-binary>`: an owned real daemon and two widget instances.
- `FBT_TEST_VISUAL=1 qs-visual.sh popup downloading --binary <debug-binary>`:
  actual native popup over a real local seed/client transfer, inside a private
  headless Sway compositor using pixman. No owner display socket is inherited.
  `--light catppuccin-latte` selects the private theme copy; `--compare
  omarchy.network` adds the installed reference panel and geometry annotations.
  The reference has an unavailable private network backend, so it never scans or
  changes the owner's network. The comparison image is for human review, without
  an automated claim that the popup looks first-party.
- `qs-visual.sh --dry-run`: read-only tool diagnostics. `qs-icons.sh` / `qs-visual.sh
  bar downloading`: all bar presentation profiles in dark/light. Bar profiles
  derive from the frozen wire objects; they do not prove live engine transitions.

Native popup captures require `sway` on PATH or `FBT_TEST_SWAY=/absolute/path/sway`.
On this Arch machine, `python3 plugin/tests/prepare-headless.py --download` can
extract packages into ignored `plugin/tests/target/headless-tools/` without sudo
or a system install. It checks the SHA-256 values from the local pacman sync
metadata before extraction. Normal tests never download tools automatically.
A missing headless compositor fails; there is no fallback to the owner's display.

Images and previous-image pixel differences go to `target/test-logs/visual/<date>/`.
JSONL, daemon, compositor and QML logs go to `target/test-logs/<suite>/`.
`daemon.py <debug-binary> --serve-fixture empty|paused|downloading` exposes the same
real fixture as `FBTFIXTURE {runtime, root, state, row}` on stdout; close stdin to
stop and reap all owned children. It never fabricates states for a screenshot.

## Smoke probe (2026-10-19)

| File | What it tests | Run | Last result |
|---|---|---|---|
| `smoke/probe-ui.qml` | Installed `qs.Ui.PanelHero` and `qs.Commons` imports via private symlinks | `FBT_BIN=<binary> plugin/tests/qs-smoke.sh` | PASS, Quickshell 0.3.2, revision 2b4501a282689792b62e94f3fb30d8f512324fe6 |
| `smoke/shell.qml` | Service/socket/real daemon/store, metadata add/pause/resume/remove and restart | Same command; `FBT_TEST_QS=1 all.sh` enables it in the integrated gate | Uses the current API subscription service |

The smoke runs offscreen with its own daemon and private HOME/XDG paths. It
never probes an owner API or Wayland socket. Mutations and reconnect observations
use the disposable daemon.

## Dashboard and network boundary

`plugin/tests/qs-dashboard.sh --binary <debug-binary>` runs the actual dashboard
inside the private compositor, summons it with the installed `omarchy-shell`
command pointed at the private config, types with `wtype`, and checks 50 native
window create/destroy cycles. The installed registry verifies that only a bar
layout reference enables the panel. The test host implements the panel-loader
facade; this does not claim a run inside the owner's live host shell.

Real-daemon service, smoke, dashboard and visual tests now enter private user,
network and mount namespaces before creating cases. Only loopback exists; probes
verify that IPv4/IPv6 have no external route. A private UDP DNS fixture answers
A/AAAA with loopback and never forwards; its resolv.conf/hosts mounts affect only
the private mount namespace. `private-net-test.py` covers the boundary, forged
marker rejection, both DNS families and oversized-datagram rejection. Namespace
creation failure is a test failure, with no owner-network fallback.

This boundary is required: the P3-08 FD investigation found background DNS
sockets in the older debug binary despite the engine's loopback configuration.
Earlier captures/mutation tests exercised real local payloads, but their DNS was
not fully contained. Requalified runs are retained with the Track C receipt.

## Torrents table

`python3 plugin/tests/dashboard-table.py --binary <debug-binary>` checks the real
native table at 900 and each tier boundary ±1, including live header widths,
sidebar/details docking, a native double-click, no right-click action, Cancel-first
remove/delete, typed sort persistence across a service restart, confirmed
pause/resume, Details upload opt-in/out, recorded argv for open/copy/title links,
real Shows follow and Ctrl+Q shutdown. It captures Details at 900/1280/1500/1600 px.
`python3 plugin/tests/run-qs.py table plugin/tests/table` covers the Qt proxy,
extra typed sorts, pending/failed ref indexes, unread badges and visible ranges.
`lint.sh` rejects collection loops in Service/view frame handlers with positive
and negative controls.

The native pointer sender builds in ignored test storage using `cc`,
`wayland-scanner` and `pkg-config wayland-client`. Its checked-in MIT protocol XML
comes from [wlr-protocols](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-virtual-pointer-unstable-v1.xml).
It requires an explicit private display plus the private-network marker, and
keeps the device alive until the table reports the hover before sending buttons.
There is no owner-display fallback. The native proxy uses the installed Qt 6.12
`model` property and a native ValueFilter over the store’s cached visibility role; a real empty ListModel avoids
Qt's null-source crash during panel construction.

`python3 plugin/tests/dashboard-table.py --profile --binary <debug-binary>` creates
200 genuine daemon records (199 paused and one 8 MiB private seed/leech transfer
at 32 KiB/s). The same fixture is `daemon.py <binary> --serve-fixture table-200`.
After window creation, a guarded private recipe Timer calls the production scroll
method 200 times at 50 ms intervals (elapsed time is checked), and samples the
real transfer every second. This avoids spawning 200 CLI processes during the
measurement; the production guarded IPC scroll entry point is checked separately. It parses QSG_RENDER_TIMING's renderer timings, or
the software backend's whole-frame timings (including polish/sync/render/swap),
and fails on absent samples or any frame above 32 ms. Logs include every measured
frame and actual completed-byte/rate samples. It qualifies the negotiated
poll/subscription transport actually available in the tested binary; fallback
results do not prove a pushed subscription. All compositor, DNS, network, state
and child-process boundaries described above apply.

The table batches actual source-order changes: the installed Qt proxy otherwise
resets delegates for each individual source move. One detach/reattach surrounds
the move group and preserves contentY; ordinary role patches keep the same proxy.
The source ListModel remains patched in place. The unit regression observes the
real Qt rowsMoved signal and requires every move inside one boundary, and none
for an unchanged-order role patch. `FBT_TEST_QS_DIAG=1` enables per-owned-process
ptrace authorization and a bounded GDB stack capture if the profile stalls; it
never changes the system ptrace policy or inspects an owner process.

`plugin/tests/qs-dashboard.sh --rss --binary <debug-binary>` starts from a saved
feed after a daemon restart, enters RSS with `4`, and drives add/cancel, refresh,
read toggles and deliberate staged Download. `--recheck` scrolls to the real
Overview button, checks Cancel-first confirmation and verifies a corrupted private
payload through the pending acknowledgement and final receipt. Both use private
Sway/XDG/network state. `plugin/tests/qs-client.sh` advances the client clock past
the ordinary reply deadline to cover long checks, lost connections and late
receipts without claiming that the live hash scan took that long.

## M2 owner journeys

Run `plugin/tests/qs-dashboard.sh --journeys --binary <current-binary>` at an
integration release point. `--workflow W1`, `W2` or `W8` selects one journey.
Each drives the real dashboard with native keyboard input in its own private
Sway, XDG and loopback network namespace. W1 uses real local providers, IMDb,
browse data, a tracker and two torrent engines; W2 follows/checks Shows and
checks read/transfer persistence; W8 verifies Cancel, filtered removal, bounded
Undo and deletion against private payload digests and unrelated sentinel files.
Player and notification commands are private recording shims. An unavailable
workflow fails explicitly. Logs are in `target/test-logs/qs_journey_w1/`, `qs_journey_w2/` and `qs_journey_w8/`.

## M1 / M6 milestone checks

`FBT_BIN=<current-binary> plugin/tests/qs-smoke.sh` exercises only an owned
private daemon, including mutations and restored Summary/torrent subscriptions
after restart. It never probes the owner's socket.

`plugin/tests/qs-smoke.sh --palette <current-binary>` and
`plugin/tests/qs-dashboard.sh --palette <current-binary>` run the same
`palette_summon_routing` journey: the installed separate summon route, native
query/magnet/path input, staged review cancellation, unchanged bar toggle and
continued transfer. Both require the private network namespace and compositor.
`FBT_TEST_QS=1 scripts/gate-full.sh` includes it once alongside store and smoke.

`FBT_TEST_VISUAL=1 plugin/tests/qs-visual.sh --milestone --binary <current-binary>`
captures empty, downloading, paused, 200-row, ready and may-stall popups in
`owner` and `catppuccin-latte`, with private copies of each theme. Each capture
also pairs the real installed Wi-Fi panel (its backend is an unavailable private
shim). PNGs are under `target/test-logs/visual/<date>/`, with per-state logs under
`target/test-logs/qs_visual_runs/`. No owner-theme switch or shell IPC is used.
The opt-in full gate runs this matrix using the executable it just built.

## M2 milestone checks

`python3 plugin/tests/m2.py --binary <current-binary>` runs the P2 checks, every
dashboard entry point, all width boundaries (including 1098/1398/1598), the three
owner journeys and P3 daemon cases serially. Native shells and foreground daemon
cases use private network namespaces. The real-manager catalogue autostart case
uses its existing disposable named unit, private XDG and loopback HTTP fixture
in the manager's network namespace, so both ends can actually communicate. Focus with
`--group p2|dashboard|daemon` or repeated `--case NAME`. Each invocation retains
commands, exit status, output and generated case evidence under `target/m2-*/`.
See [the one-page owner demo](../../docs/M2-demo.md) for real-desktop checks.

## Resource measurement

The resource gate is local and opt-in: `FBT_TEST_RESOURCES=1`. Run it on the
reference host before each release, after other builds and tests have finished.
It measures the daemon with 10 and 100 paused local torrents and 0, 1 and 8
Summary subscribers: three 60-second windows per configuration, plus matched
CLI polling references (about 24 minutes total). `FBT_TEST_DAEMON_RESOURCES=1`
selects the same matrix; setting both runs it once. Each fixture has private
HOME/XDG paths, local payloads and a loopback-bound daemon.

The checked-in P1-08 daemon baseline is in `baselines/resources.json` beside
P0-06's original TUI measurements. The gate uses the daemon workloads because
the TUI now attaches to the daemon and `q` detaches. Every CPU/RSS observation
must be at or below its baseline median plus the larger of 10% of that median
and three times the range of the three baseline runs. One bar subscription must
cost no more CPU than status polling every ten seconds. A different reference
host or window is rejected. Comparison runs never rewrite the baseline.

To run just the resource check after building `release-perf`, use the following
under the release lock and memory scope. To deliberately record a new reference,
replace `--baseline` with `--write-baseline` and review the values before committing.

```bash
python3 agent_ergonomics_audit/audit/regression_tests/resources.py target/release-perf/fbittorrent \
  --daemon --matrix --runs 3 --seconds 60 \
  --baseline agent_ergonomics_audit/audit/regression_tests/baselines/resources.json \
  --output target/test-logs/resources/measurement.json
```

`resources.py --daemon --torrents N --subscribers K --seconds 60 --runs 3
--baseline <path> --output <path>` measures one workload; add `--matrix` for the
complete matrix.
