# Track D — TUI

## P1-09 — fbt-p1-09-wsl

Implemented per-instance/per-client 256-entry, ten-minute ref receipts, exact
op/args matching, cached errors/pending responses, replay listeners, generated
refs and stale-instance checks from `ops::TABLE`. URL metainfo is validated
(bencoded, ≤16 MiB) before admission through the existing bound/cancellable
engine runtime. Magnets and file deletion have correlated pending outcomes;
file deletion has no timeout. The ring survives engine-slot restarts.
`mutations` is advertised; frozen contracts/docs are unchanged. Split the
inherited server into modules, all below 900 lines; framing/decoder bodies are
unchanged. Neighbouring `src/engine/ops.rs` adds only a per-hash try-lock restore
receipt and cancellable URL fetch; teardown is unchanged. The server split
will overlap P1-06/cod_2 and P1-08/cod_1's concurrent server changes.

Evidence root: `target/test-logs/fbt-p1-09-wsl/`. Commands use `run.py` there to
retain command JSONL/stdout and private XDG; daemons additionally isolate HOME
and TMPDIR and use local HTTP/payloads.

| Command | Result and retained evidence |
|---|---|
| `cargo check --locked` | Exit 0, `check.log` (before split). |
| `cargo test --locked -p fbittorrent api:: -- --nocapture` | Exit 0, 62 passed/1 ignored, `api_unit_final.{jsonl,log}`; expiry prints 599+1 seconds; includes eviction, cached errors, names and pending order. |
| `FBT_MUTATION_ARCHIVE=live_final python3 …/live.py` | Exit 0, 16 real-process cases/380 steps/zero failures; `live_final.{jsonl,log}`, `live_final/daemon/`. All seven implemented acceptance criteria pass in their named cases; CLI list-diff removal remains P1-11. Includes 257/empty hashes, base32/v2, generated refs, URL size/bencoding, engine restart and pool pressure. |
| `cargo test --locked -- --nocapture` (ONE full run) | Exit 0, 282 passed/2 ignored; `full.{jsonl,log}`, `full_scan.json` (zero worker failure matches). Includes the no-timeout deletion regression. |
| `scripts/lint-sizes.sh src/api/server` | Exit 0, `sizes.log`; `preservation.json` records all changed Rust sizes and unchanged fixture hashes/framing bodies. |

The captured URL pending/result matches frozen `mutations.ndjson` through the
P1-06 normalizer; Rust validation passes with optional `duplicate` present and
removed. Earlier split compile failures and the first live pressure failure
are retained (`api-unit.log`, `api_unit_v2.*`, `live_v1/`); corrected admission
preserves shutdown under pressure without weakening assertions. No release or
`all.sh` between beads, per the track policy.

Integration outstanding: P1-08's bus must consume
`Server::confirmed_mutations()` into its events topic/history with the shared
sequence policy. Direct originating/replaying delivery passes; live events-topic
delivery is not yet proven. M3 is not reached.

Continuation: P4-01 awaits P1-15; P4-02–11 and P5-02 depend transitively on the
split/API work or other tracks. P6-10 awaits P1-13 (`src/daemon/notify.rs`) and
P3-03 (`src/api/ops/shows.rs`), both absent here. No further Track D bead is
unblocked on this branch. No branch switch or merge performed.

Implementation commit: `c2269c1c49deddb42a5fd67cab9773b3a876e453` (16 files).
Metadata deviation: its message body contains literal `\n` separators, so the
co-author text is not a parsed trailer. History is preserved under the owner's
no-rewrite rule; this receipt commit has the correctly formatted trailer.

## P4-01 — fbt-p4-01-atf

Moved the TUI into state, connection/quit actions, four screens, four modals,
layout, mouse, footer, help, text, IMDb-card and motion modules. `mod.rs` is
382 lines; the largest TUI source is the unchanged 605-line interaction-test
module. The four intentionally empty modules requested for subsequent beads
are declared. Public TUI paths remain re-exported. State field layout remains
flat so every function body and initializer stays byte-identical; existing
modal structs move with their modal. P1-15 can merge its connection/quit changes
into `src/tui/connection.rs`; the unchanged Model glue remains in `mod.rs`.
Neighbouring `.github/workflows/ci.yml` now runs the existing size gate on TUI
as well as engine. No unrelated behavior or frozen contract changed.

Evidence: `target/test-logs/fbt-p4-01-atf/`. Focused before/after command
`cargo test --locked tui:: -- --nocapture` passes the same 68 named tests,
including all 33 goldens at 80×24, 120×30 and 160×45 (`before_tui.*`,
`after_tui.*`, `test-comparison.json`). `final-proof.json` proves all 165 moved
function bodies and the entire interaction-test contents byte-identical, and
records identical before/after SHA256s for every golden. The existing theme,
poster, golden harness and native.py are unchanged. Before/after
`python3 agent_ergonomics_audit/audit/regression_tests/native.py target/debug/fbittorrent --case file_modal_shows_help_80x24`
both pass; all three raw tmux panes (120×30, 120×30, 80×24) are identical
(`before-native/`, `after-native/`, `tmux-comparison.json`). `cargo check --locked`
passes (`split_check.*`); `scripts/lint-sizes.sh --self-test src/engine src/tui`
passes all 104 files (`sizes.*`). The bead's ONE full
`cargo test --locked -- --nocapture` passes 282 tests/2 ignored (`full.*`),
matching the preceding bead's count. All three acceptance criteria pass.
No release/all.sh between beads. Continue with P4-02 on this branch.

## P4-02 — fbt-p4-02-6bx

One 127-row action catalogue now drives scoped dispatch, conditional native/legacy
bindings, footer priorities, paged help, `robot-docs keys --surface` and generated
README/keymap output. Removal confirms only on lowercase `y`; native `f`/`F`
follows and Search `P` opens Providers. Static key docs skip app/config startup.
An unchanged footer reuses its cached widgets; cache tests cover query, width,
feedback and read-state changes. CLI declarations/local helpers moved out of
`main.rs` (781 lines): 31 existing function bodies are byte-identical; only
`run` gains static docs dispatch (`cli-move-proof.json`). All changed source
files are ≤900 lines; the strict TUI/CLI size gate passes 80 files (`sizes.*`).
Evidence root: `target/test-logs/fbt-p4-02-6bx/`. **All three criteria pass**:
invariants in `cargo test --locked tui:: -- --nocapture` pass (`tui_final.*`;
78 tests at that point, 79 including the final cache regression in `full.*`);
`footer-overflow-80x24.txt` prints the cue with nine or more enabled actions,
and the real `native.py --case file_modal_shows_help_80x24` passes all 9 steps
(`live/native/file_modal_shows_help_80x24.jsonl`, actual 80×24 pane retained);
`scripts/gen-keymap.sh --check` matches both committed consumers
(`generated_check.*`). The bead's ONE full `cargo test --locked -- --nocapture`
passes **294 tests, 2 ignored**, including all 34 render goldens, the CLI check
and all action invariants (`full.*`, `full-summary.json`); the gate's background
failure scan is clean (`background.*`). Outside help, existing golden changes
are confined to the final two footer rows (`golden-review.json`). No release
build or all.sh between track beads. Neighbour edits: CLI split, capabilities
row, README, generated plugin JSON, native harness assertions. P1-15 must port
the catalogue's two lifetime callbacks when its connection/quit change lands;
P1-11 must reconcile the additive key-doc command with its contract-version
and command-discovery changes. Future feature beads add their own action rows;
this table does not advertise handlers that have not landed.

## P4-06 — fbt-p4-06-vp8

Completed the motion change from `e45791b` and the owner-authorized additive
Settings extension. Selected names stay static on every list; unselected hover
moves once at 8 cells/s on the existing 300 ms clock, reaches its last window,
resets and stops. `reduce_motion=true` disables it through config and the fourth
TUI Settings row without an engine restart. API Settings retains optional wire
presence and defaults absence to false; null and non-booleans are rejected.
`settings-motion.ndjson` adds a 12-record conversation covering absent/true/false/
absent with expected models. Documentation lists the field and an exact example;
all **187** old fixture records and all **3** old IPC example lines remain
byte-identical (`contract-proof.json`, `extension-proof.json`).

Both acceptance criteria pass: `selected_long_name_is_static_even_under_pointer`
keeps each rendered frame unchanged across fake times; `marquee_once_then_stops`
prints each cell/offset and finishes at zero permanently, while
`reduce_motion_no_scroll` prints unchanged cells. Evidence under
`target/test-logs/fbt-p4-06-vp8/`: `cargo check --locked` passes (`check.*`);
`cargo test --locked tui:: -- --nocapture` passes **82** tests (`tui_compare.*`);
`cargo test --locked config:: -- --nocapture` passes **4** (`config.*`);
`cargo test --locked api:: -- --nocapture` passes **64**, 1 ignored (`api_extension.*`),
including strict type, roundtrip, default and poisoned-transcript controls.
The bead's ONE full `cargo test --locked -- --nocapture` passes **299**, 2 ignored
(`full.*`), with no background failure matches (`extension_checks.*`).
`scripts/lint-sizes.sh --self-test src/api src/tui` passes **105** files
(`final_sizes.*`); the largest changed source is 687 lines. The 31 non-Settings
render goldens remain unchanged; three Settings goldens add the fourth row.
Nothing remains unmet for this bead. API service emission/attachment belongs
to P3-05/P4-03; this extension is contract proof, and the current TUI consumes
its persisted local preference. No release build or all.sh between track beads.

Next items checked in the requested order: P4-04 awaits P4-03; P4-07 awaits
P4-04/P3-01/P3-18; P4-10 and P4-11 await P4-04; P4-08 awaits P1-14/P4-03;
P4-03 awaits the authorized P1-15 integration and P3-07; P4-05 lacks P2-08's
`contracts/ui/themes/expected.json`; P4-09 awaits P4-03. No next item is
unblocked on this branch. M3 is not reached. No branch switch or merge performed.

## P1-15 — fbt-p1-15-h0l

The TUI now attaches in its own terminal to the daemon, retaining the legacy
2 s data poll and one bounded API presence connection (`tui-<pid>`, interactive
hello and `app.client` heartbeat). It owns no lifecycle object, engine or lock.
`q`/Ctrl+C/SIGHUP detach; `Q` asks while work is active and only lowercase `y`
sends `app.shutdown`. The first `q` prints the canonical notice after terminal
restoration and atomically saves the private, once-only preference. Two failed
polls show the stopped banner; Enter starts/reconnects. Settings restart the
owner's engine slot without closing the UI. RSS/Shows on-open refresh remains
local. Neighbour edits move legacy migration into daemon startup and decrement
API client counts on disconnect (the two-client regression exposed cumulative
counts); daemon supervision/teardown and all frozen wire fixtures stay unchanged.

Evidence root: `target/test-logs/fbt-p1-15-h0l/`. Run the retained
`native-cases.py verified` wrapper for `native.py` with private five-XDG fixtures,
disposable runtime-only static units and actual capped loopback seed/leech engines.
Every new case logs keys, panes and CLI status; `Q_confirms` uses logger-compatible
slug `q_confirms`.

| Criterion | Command/case | JSONL evidence | Observed |
| --- | --- | --- | --- |
| No daemon → TUI starts it | `native.py --case tui_starts_daemon` | `verified/native/tui_starts_daemon.jsonl` | same owner remains after heartbeat and Ctrl+C |
| q keeps download running | `native.py --case q_detaches` | `verified/native/q_detaches.jsonl` | real payload increases after q; same daemon PID |
| Q confirms/stops | `native.py --case q_confirms` | `verified/native/q_confirms.jsonl` | Y/n/Enter cancel; y stops owner within 6 s |
| First-q notice once | `native.py --case first_q_notice_once` | `verified/native/first_q_notice_once.jsonl` | first prints; second omits; saved marker true |
| Daemon-gone banner <5 s | `native.py --case daemon_gone_banner` | `verified/native/daemon_gone_banner.jsonl` | stopped + Enter in 4.026 s; Enter starts new PID |
| Two terminals attach | `native.py --case second_tui_attaches` | `verified/native/second_tui_attaches.jsonl` | both show same torrent; clients 2→1 on detach; no focus helper |

`cargo test --locked tui:: -- --nocapture`: **86 pass** (`focused_final.*`),
including prompts and all **34 unchanged goldens**. The bead's one full
`cargo test --locked -- --nocapture`: **303 pass, 2 ignored** (`full.*`);
subsequent attach-path refinement is covered by the focused run and six real
cases. Agent documentation checks: 4 pass (`agent_docs.*`). Size self-tests and
all 116 API/TUI/daemon files pass (`sizes_final.*`); generated key docs match
(`generated.*`). SIGHUP detaches in **121 ms**, leaves the unfinished torrent,
socket and held lock (`verified/native/terminal_close_is_graceful.jsonl`).

Additional native replay: 24/26 pass in `verified/native/summary.json`.
`quit_restart_down`'s log checker was extended for the exact successful daemon
restart/parked phases, retaining negative controls, and its existing Shows-badge
assertion now waits for the initial asynchronous read model. Its focused replay
is recorded in `restart_final/`. `startup_survives_bad_record` remains red:
CLI `show` intermittently returns exit 4 through the legacy readiness probe
(which has a 200 ms budget), while the daemon remains running (also reproduced separately in
`restore_verified/`); no assertion or timeout was weakened. Follow-up: replace
that false-closed CLI probe through P1-11's API client. The six bead criteria are
met; automatic idle exit is still P1-12, as disclosed in README/robot-docs.
No release build or all.sh between track beads. Continue with P4-03's available
API screens; Track B namespaces and the bus integration still need merging.

## P4-03 — fbt-p4-03-r5p (in progress)

The confirmed mirror is implemented in `src/tui/store.rs` and its snapshot,
sort and pending modules. It commits chunked snapshots atomically, rejects
revision gaps without changing confirmed rows, removes null optional fields,
compares numeric sort values, uses daemon file sort keys and retains rows as
stale across instance/engine changes. Pending/confirmed mutation messages do
not fabricate row state; disconnects mark results unknown and never retry.
Staging is bounded to 32 MiB, 100,000 items and 16 subscriptions; events to 512
and pending mutations to 64. `cargo test --locked -p fbittorrent tui::store::
-- --nocapture` passes **11 tests**, replaying all **9 frozen expect models**
with file/frame/first-JSON-pointer diagnostics and poisoned-expect controls
(`target/test-logs/fbt-p4-03-r5p/mirror_final.{jsonl,log}`). Size self-tests pass
(`sizes.*`); every new source file is below 900 lines. This is an intermediate
component, not completion: UI connection/data-path wiring remains. Current
branch lacks the bus and settings request operations, and Track B feature
namespaces; those integrations remain pending. No full suite or all.sh run for
this intermediate checkpoint; the bead's one full suite is reserved for its
completed integration.

P4-03 read-path checkpoint: native torrent rows, rates, network state, file reads
and file selection now use the existing presence connection. The typed store
keeps unknown values; screen adapters render them as unknown and use explicit
seed intent, including custom caps and completed records without a date.
`cargo test --locked -p fbittorrent tui:: -- --nocapture` passes **99** tests,
including all **34 unchanged goldens** (`rpc_units.*`). The real
`daemon.py --case tui_single_connection` passes with `engine.sock` renamed out
of reach: torrent/file views stay online, Space changes real file selection,
and client counts go **0→1→0** (`rpc_live/daemon/tui_single_connection.jsonl`,
3.826 s). Debug build and size self-tests pass (`rpc_binary.*`, `rpc_sizes.*`).
The native helper now accepts the bead's unique tmux session name. Remaining:
push/ApiSub transport, native mutations, reconnect UI, settings service and
Track B namespaces. The existing timer remains until the push server arrives;
no full-suite/all.sh run for this coherent intermediate integration.

P4-03 mutation checkpoint: native start/stop/seed/remove/add/open and file
selection use the single API connection. Its bounded multiplexing worker emits
ordered mutation refs and events, keeps heartbeats/RPCs responsive during pending
URL fetches, caches completion before waking callers, and cancels on UI close.
Rows remain confirmed; pending verbs dim rows and disable the same actions in
both keys and footer. Search/RSS/Shows keep their current service implementation
behind `remote::ScreenClient`; RSS and Shows mark downloaded only after real
admission, including a failure control. Local provider metadata handoffs are
private and bounded to eight unconfirmed files. Reconnect to the same daemon
resets connection-scoped revisions and stale replies cannot overwrite new data.
`cargo test --locked -p fbittorrent tui:: -- --nocapture`: **106 passed**, all
**34 prior goldens unchanged** (`action_ready_units.*`). Debug build, source size
self-test and Python compilation pass (`action_ready_binary.*`,
`mutations_sizes.*`). Two real tmux cases pass with engine.sock hidden
(`action_ready_live/daemon/{tui_single_connection,tui_pending_rss_failure}.jsonl`):
file selection, start, seed, stop, one HTTP add, API target/open, keep-files
remove, one TUI connection 0→1→0; held/failed HTTP admission stays responsive and
leaves RSS unread. The first repeat exposed a truthful-action race: the footer
advertised Resume before file selection settled. The catalogue now disables
pending row actions and the test waits for their visible readiness (no timeout
or outcome assertion relaxed). Five P1-15 lifetime cases pass in
`lifetime_regression/native/`; q_confirms passes in
`lifetime_confirm_verified/native/` after a minimal adjacent harness fix that
verifies a process has exited when /proc vanishes during shutdown rather than
throwing ProcessLookupError. Every live-child isolation check remains enforced.
The bead remains incomplete: the branch lacks the bus/shared-client/settings
and Track B service integrations; its two-second fallback timer remains until
push is available. Reconnect UI is the next independent step. No full-suite or
all.sh run at this coherent intermediate checkpoint.

P4-03 recovery checkpoint: initial attachment runs off the UI thread and shows
Connecting; connection loss preserves dimmed confirmed rows, marks in-flight
results unknown, and retries only the socket at 0.5/1/2/4/8/10 seconds (then 10).
Only explicit opening/Enter can start a daemon. One handshake runs at a time;
success resets the backoff and the connection-scoped mirror. Unsupported
protocols produce a persistent banner before decoding foreign reply shapes;
newer installed stable versions show the restart-to-update notice. The footer
cache hashes the notice/countdown without formatting on unchanged frames.
The full `cargo test --locked` run is green: **327 passed, 2 ignored**, including
**110 TUI tests** and all **34 unchanged goldens** (`full_cargo_test.*`). Debug
build and source size self-test pass (`recovery_binary.*`, `recovery_sizes.*`).
The original lifecycle/owner-request assertion is unchanged and passes; a
session-less poll still has no effect on the daemon owner's lifecycle.
The six real lifetime cases pass in `recovery_lifetime/native/`; the terminal
helper now waits for the new interactive hello before asserting owner state.
The core mutations and failed RSS admission cases pass in
`recovery_live/daemon/`. The replacement-owner case passes in
`recovery_owner_verified/daemon/tui_reconnect_existing_owner.jsonl` (5.180 s):
shutdown keeps the cached row and terminal, retries leave the socket absent,
an independently started owner has a different instance, the same terminal
reattaches without Enter with exactly one TUI connection, fresh removal clears
the old row, and q releases the connection. The receiver is closed/reopened
between daemon generations so stale STATUS datagrams cannot satisfy or spoil
the successor's exact READY=1 check; failed development attempts are retained.
Python compilation and git diff checks pass. No release build/all.sh is needed
for this track checkpoint. Remaining P4-03 acceptance is explicitly unmet:
remove the native timer and local service/legacy bridge after bus, shared
api::Client, settings and Track B namespaces are merged into this branch.
The regex for qbt::Client alone is insufficient proof while the isolated
LegacyClient bridge still exists. P4-04 is skipped pending the missing
P2-08 contracts/ui/states.json and the remaining P4-03 integration; P4-10 and
P4-11 require its modal/layout, P4-07 additionally needs the Track B services,
and P4-09 requires the completed API migration. No frozen fixture or old golden
was changed; no daemon lifecycle/teardown implementation was touched.

P1-15/P4-03 native gate repair: each attachment fixture now has a sandbox-specific
tmux socket and asserts server creation; q is checked against the same live
daemon PID/instance rather than interactive presence. The stderr allowlist
already recognized daemon startup; it now also accepts the named Settings
restart reason and explicitly rejects unknown lines/reasons. Parked readiness
uses app.info/app.network; unknown completion remains absent on API v1, is
shaped as zero by the API CLI or retained as -1 by the legacy adapter, and the
saved -1 is still asserted unchanged on disk. The TUI now uses saved intent for
Pause/Resume and its footer cache, including VPN-waiting rows; positive
completion dates keep Resume hidden when seeding is off. The timestamp case
presses p twice and verifies both real ledger intents through the API. All
three requested cases pass against the final debug binary via the unmodified
native.py entrypoint with --case (`native_gate_final_{q_detaches,quit_restart_down,
resume_unknown_timestamp}/native/*.jsonl`). The one all-native invocation
(`native_gate_all/native/summary.json`) ran 26 cases: 24 passed, two failed.
The run exposed an additional asynchronous-attach regression: --no-start tui
waited instead of returning exit 4. Attach-only opening now checks the existing
owner before fullscreen and avoids opening a second connection; the adjacent
backend policy getter is identical to the one already on main. Its existing
real-terminal/zero-launcher control now passes
(`native_gate_final_launcher_never_called/native/launcher_never_called.jsonl`),
as do the three requested cases on the final binary. The remaining full-native
failure is startup_survives_bad_record: legacy CLI show exited 4 after a 211 ms
probe, followed by a missing-state KeyError (steps 46/47); retained unchanged
for the API CLI integration, which is not on this branch. Full cargo test
--locked passed once: 328 passed, 2 ignored (`native_gate_cargo_test.*`); after
the small attach-only correction, the touched TUI suite passed 111 tests with
34 unchanged goldens (`native_gate_attach_units.*`). Debug build, Python
compilation, size gate and diff checks pass. settings_queue_vpn_restart was
verified byte-identical and passed locally; its main-gate repair remains the
integrator's. No release/all.sh run, frozen fixture change, or state-table
change. All log paths above are under target/test-logs/fbt-p4-03-r5p/.

### P3-01 search extraction checkpoint

Moved search download resolution, process capture/lifetime, job registry and all existing tests into `src/search/*`; existing function bodies and test bytes are identical, with only sibling visibility and facade wiring added (`split_isomorphism.jsonl`). `cargo test --locked -p fbittorrent search:: -- --nocapture`: 15 passed, including three search goldens; `bash scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src`: passed. Evidence: `target/test-logs/fbt-p3-01-mrt/split_{units_fixed,sizes,isomorphism}.*`. The daemon integration is still in progress; no search acceptance criterion is claimed at this checkpoint.

### P3-01 — daemon search

Added daemon-owned search sessions (four admissions, reclaimable thirty-second disconnect lease, process-group cancellation and drain), typed API operations and keyed bus snapshots/deltas/completion events, provider/browse/grab adapters, ledger `added` annotations, deterministic deduplication and conservative mojibake repair. Provider output/raw session retention is bounded to 2 MiB; metainfo inspection is bounded and runs outside the bus. Added `contracts/ipc/v1/search.ndjson`, capability/operation metadata and contract 1.2.4. Existing IPC records/examples and existing search tests remain unchanged. Neighbouring TUI mirror changes support the additive fixture's search items; the search screen migration follows in P4-03.

| Acceptance | Command | JSONL evidence under `target/test-logs/fbt-p3-01-mrt/` | Observed |
|---|---|---|---|
| Dedupe both providers and repair the name | `python3 …/daemon-case.py live search_dedupe_and_repair` | `live/daemon/search_dedupe_and_repair.jsonl` | PASS; one `辛特尔`, alpha+beta, nine seeds; original damaged UTF-8 bytes retained |
| Disconnect expires at 30 s and kills the process group | `python3 …/daemon-case.py live search_process_group_gone` | `live/daemon/search_process_group_gone.jsonl` | PASS; live children at 28 s, no members at 30.070 s; session not found |
| Fifth session is busy | `python3 …/daemon-case.py live search_fifth_busy` | `live/daemon/search_fifth_busy.jsonl` | PASS; retryable `busy`, `details.limit=sessions`; stop frees admission |
| Reconnect reclaims retained results | `python3 …/daemon-case.py live search_session_grace` | `live/daemon/search_session_grace.jsonl` | PASS; identical results and provider group; search busy reason |

Additional live proof: `grab_live/daemon/search_grab_and_ledger.jsonl` PASS (real loopback metainfo, pending/confirmed ref, origin search, ledger annotation, parked refusal with VPN hint). The first combined live run caught a URL receipt waiting for a magnet-only outcome; fixed and this extra case rerun green. Focused `cargo test --locked -p fbittorrent search:: -- --nocapture`: 19 passed (`service_units.*`). One full `cargo test --locked`: 384 passed, one failed, two ignored (`search_full.*`): the additive fixture exposed the TUI reducer's missing search-item support. Added the reducer and regression; `cargo test --locked -p fbittorrent tui:: -- --nocapture`: 112 passed, including all 34 unchanged goldens and nine original expect models (`search_consumer_units.*`). Per owner cadence the full suite was not repeated; final source size gate passes (`search_final_sizes.*`). All four bead criteria are met; the initial full-suite failure and its focused repair are recorded explicitly. No release build/all.sh (track-level instruction). Follow-up: P3-07 CLI shaping remains with its owner; P4-03 search screen migration is next.

### P4-03 — native search client checkpoint

Native search now starts and subscribes on the existing presence connection, mirrors keyed pushed results, preserves selected row identity and provider lists, shows the daemon's `added` annotation, and sends selected-row grabs, provider toggles and browse requests to the daemon. Completion uses the event topic, including retained results; reconnect reclaims the session only on the same daemon instance, and never replays a query on a replacement owner. `q` releases the connection without stopping its session. Native IMDb cards use `imdb.lookup` and bounded poster resources; local ratings/provider execution remain only on the legacy/service fallback path. The bounded event mirror now matches the daemon's 1000-record retention.

`cargo test --locked -p fbittorrent tui:: -- --nocapture`: 114 passed, all 34 goldens and nine original expect models unchanged (`target/test-logs/fbt-p4-03-r5p/search_client_units.*`). `daemon.py` cases `tui_search_api`, `tui_search_owner_restart`, `tui_single_connection`, `tui_reconnect_existing_owner` all pass against the debug binary (`search_client_live_fixed/daemon/tui_search_api.jsonl`, other three in `search_client_live/daemon/`). Positive search terminal proof: engine.sock hidden, exactly one TUI connection, providers descend from the daemon, provider toggle persisted over API, pending grab creates origin=search, q leaves the provider alive. Replacement-owner proof: old process group gone, terminal reconnects, query is not replayed. An initial ancestry assertion incorrectly used the deliberately hidden legacy socket; it now uses the v1 hello PID, with the assertion's intent preserved. `cargo test --locked`: **388 passed, 0 failed, 2 ignored** (`search_client_full.*`), also validating the P3-01 consumer repair in the preceding commit. Source size gate passes (`search_client_sizes.*`). P4-03 remains open: RSS/Shows migration, removal of their legacy facade and the core two-second refresh are still pending; this checkpoint closes only the search/IMDb slice. No release/all.sh between track beads.

### P4-04 — state/compact-column checkpoint (still in progress)

Embedded the shared state table, with every state glyph measured as one cell and every state cell padded to nine. Torrents now keep STATE and NAME at all supported widths, with compact percentage/rate/ETA columns and Size/Peers at medium/wide; absent native measurements stay blank or unknown as appropriate. The existing 80..160-column filename/mouse test caught truncation beside the old bar/flags; compact columns fix it without relaxing that assertion. The queued/VPN control now checks the exact contract glyph, short word and padding instead of obsolete bar labels. `cargo test --locked -p fbittorrent tui:: -- --nocapture`: 117 passed in ordinary verification mode (`target/test-logs/fbt-p4-04-9lu/state_columns_verify.*`); reviewed 20 golden differences affect only torrent header/rows, their exposed modal backgrounds and the selected state line; frame heights are unchanged. Size gate passes (`state_columns_sizes.*`). Remaining P4-04 work: table virtualization/rendered rectangles, details modal, responsive Search/Shows/RSS, banners and final evidence; full suite reserved for bead completion.

### P4-04 — virtualized torrent table checkpoint (still in progress)

Replaced torrent rows with a cached ftui Table over the Virtualized visible range, typed header sorting (unknown values remain last), right-border overflow scrollbars and hash/rectangle records consumed by mouse selection. The 5,000-row control formats 24 visible rows once over 32 unchanged frames, ignores offscreen edits and invalidates visible/theme changes. Every visible row is clicked at 80/120/160; a reorder between rendering and clicking still selects the recorded hash. `cargo test --locked -p fbittorrent tui:: -- --nocapture`: 121 passed (`target/test-logs/fbt-p4-04-9lu/table_final_verify.*`); size gate passed (`table_final_sizes.*`). Reviewed torrent/background goldens: no internal dividers, state words and full selected episode filename preserved; outer border padding is now zero. Appendix B gates the s/S chords on P4-09; header sorting is enabled now. Details, other list layouts and banners remain in progress; no full suite claimed at this compiling checkpoint.

### P4-04 — Details and banner checkpoint (still in progress)

Enter opens stable-hash Overview at every size; brackets stop at Overview/Files, h/Esc returns, missing records cannot retarget an action. Narrow is fullscreen; medium/wide center 100x30 and fall back below 30 rows. Added the separate medium selection strip and one priority banner (disk > VPN > connection/stopped > update), consuming shared reason phrases. The new Details goldens intentionally expose only the two implemented tabs (later tabs belong to P4-08/P5-02/P6). Header theme text now hides below wide rather than clipping. `cargo test --locked -p fbittorrent tui:: -- --nocapture`: 127 passed in normal mode (`target/test-logs/fbt-p4-04-9lu/details_verify.*`); size gate passes (`details_sizes.*`). An old pane-cutoff assertion matched the newly required Details footer label; it now tests the actual pane border, retaining the same 140-column cutoff, filename and mouse assertions. Reviewed 80/120/160 Details goldens, medium strip and exposed modal backgrounds. Live debug terminal cases `details_live/daemon/{tui_details_narrow,tui_single_connection}.jsonl` PASS: 80x24 navigation, real include/skip selection over the one API connection, return/detach, owner alive. The existing one-connection case now explicitly traverses Overview then Files, retaining all engine assertions. README/keymap are regenerated from the action catalogue. Remaining: responsive Search/Shows/RSS, virtualization for their lists and final bead evidence/full suite.

### P4-04 — responsive layouts complete

Finished narrow single-pane Shows/RSS breadcrumbs, medium extra columns/card placement and wide side cards; Search keeps Size/Seeds/rating at 80 and Uploaded/Provider/Peers at 120/160, with v/Esc card navigation. Every list now renders a cached ftui Table over a Virtualized viewport, with an overflow scrollbar on the right border. Modal rendering drops covered hit regions. Torrent/Search header clicks sort confirmed numeric values, preserve selection identity and keep absent measurements last; pushed search updates retain the chosen order. The 5,000-row controls reach the final row and build formatted visible rows only once over 32 unchanged frames (no claim of allocator-wide zero allocations inside ftui). Added known-zero/absent-size and confirmation-visibility controls; only the modal draws its Details legend. Reviewed the regenerated Search/Shows/RSS/modal goldens: existing P0-16 fixture data is retained, including its values rather than the plan's illustrative numbers; Details exposes Overview/Files per Do step 5. Existing IPC fixtures, examples and state-contract bytes are unchanged.

| Acceptance | Command | Evidence under `target/test-logs/fbt-p4-04-9lu/` | Result |
|---|---|---|---|
| Three-size goldens and truthful words | `cargo test --locked -p fbittorrent tui:: -- --nocapture` | `screens_verify.{log,jsonl}` | 134 passed, including all 37 goldens, glyph-width and state-cell controls; native absent values stay absent |
| Click each visible row at every size | same command, `click_hits_rows_at_each_size`, `click_every_other_visible_list_row_at_each_size` | `screens_verify.log` (size, coordinates, hash/identity printed) | PASS at 80x24, 120x30, 160x45; covered modal regions excluded; numeric header controls pass |
| Enter at 80 opens Overview; brackets navigate and stop | same command; real `daemon.py` cases via `python3 …/daemon-case.py final_live …` | `screens_verify.log`; `final_live/daemon/tui_details_narrow.jsonl` | PASS; real file selection changes over API, h/Esc return, daemon survives q |

Final private debug terminal cases `tui_details_narrow`, `tui_search_narrow`, `tui_single_connection`, `tui_search_api`: all PASS (`final_live/daemon/*.jsonl`), with captured panes and daemon stderr checks. The first narrow search fixture typed v into the deliberately focused query; it now explicitly leaves editing before opening the card. An old filename test now navigates to RSS Items, retaining its assertion at every size. `cargo test --locked` ran once for the completed bead: **408 passed, 0 failed, 2 ignored** (`full.*`); final debug build, generated keymap check, Python compilation, source size gate (`final_sizes.*`) and diff check pass. Every changed src file is <=900 lines. No release/all.sh between track beads. No acceptance item unmet. Follow-ups already assigned: P4-03 RSS/Shows service migration, P4-08 Log tab, P5-02 Peers tab and P4-09 s/S chords after legacy removal; they are not claimed here.

### P4-10 — native three-size terminal smoke

Replaced the legacy live-qBittorrent shell smoke with `tui_smoke.py`: one private network/mount namespace, isolated XDG, a real capped loopback seed, a genuinely checked 7 MiB prefix, a completed torrent and unresolved metadata produce the P0-16 torrent states. One tmux server is resized to 80x24, 120x30 and 160x45; each size exercises worded state, Overview/Files/bracket ends, real file names, cancelled removal, all screens, help, q detach with payload bytes still increasing, reattach and cancelled/confirmed Q. Every UI step stores the full pane as observed and a separate artifact. The native CI job and default all.sh include the smoke plus its forced-pane control and summary/reset suites; the README points to the native script. Minimal neighbouring ci_test.py changes cover those dispatch entries without changing its existing assertions.

| Acceptance | Command | Evidence under `target/test-logs/fbt-p4-10-bce/` | Observed |
|---|---|---|---|
| Green locally and in CI | `python3 …/tui_smoke.py target/debug/fbittorrent --log-root …/verified` | `verified/tui_smoke/layout_{80x24,120x30,160x45}.jsonl`, `verified/tui_smoke_setup/fixtures.jsonl` | Local PASS: 28 pane-bearing steps per size, real payload continues after q; setup/cleanup green. Native CI wired and dispatch-tested; hosted CI not executed here |
| Failed step retains its pane | `python3 …/tui_smoke_test.py target/debug/fbittorrent --log-root …/verified_control` | `verified_control/tui_smoke_control/forced_pane.jsonl` and `fixtures/forced/tui_smoke/layout_80x24.jsonl` | PASS: exactly step 1 forced red, complete 24-row pane retained in observed/artifact, later steps continue, final/summary exit 1 |

`fbtlog_test.py`: all cases pass (`logging.*`); `ci_test.py`: five cases pass (`ci_wiring_fixed.*`). Initial CI dispatch control correctly failed because its copied umbrella fixture lacked the two new scripts; added named dispatch fixtures and an unconditional-smoke/summary check, retaining all prior opt-in/resource assertions. Full `cargo test --locked` once: **408 passed, 0 failed, 2 ignored** (`full.*`). Python compilation, bash syntax and diff checks pass. No src changes, release build or real all.sh run between track beads. **Unmet evidence:** hosted CI green cannot be asserted without the orchestrator's integration/CI run; branch pushes are forbidden. Follow-up: AGENTS.md still names the retired tui_smoke.sh in its tmux example; central policy text was left to the orchestrator.

### P4-11 — poster cards

Added a centered v/Esc poster overlay on Torrents, Search, Shows and RSS, keeping the 24x18-cell image footprint and the legend outside the overlay. Wide Shows reserves a full-height card with room for its actual image. Session cards share an IMDb ID across release aliases (256 card entries/1024 aliases); concurrent name resolutions preserve an existing decoded image. Hover debounce is 300 ms. Ratings are dim, title/year bold and absent images use the selection placeholder. Kitty placements are cleared before covering overlays and restored afterwards; unchanged frames do not retransmit, including release aliases with the same IMDb ID. New controls use the real image decoder, renderer and kitty byte encoder. Delete framing agrees with the [upstream kitty graphics specification](https://sw.kovidgoyal.net/kitty/graphics-protocol/#deleting-images). Existing IPC fixture bytes and examples are untouched.

| Acceptance | Command | Evidence under `target/test-logs/fbt-p4-11-aod/` | Observed |
|---|---|---|---|
| At 80 v shows the card; Esc hides it | `cargo test --locked -p fbittorrent tui:: -- --nocapture`; `python3 …/tui_smoke.py target/debug/fbittorrent --log-root …/final_live` | `verify.*`; `final_live/tui_smoke/poster_overlay_cleanup.jsonl` | PASS on all four screens in unit controls and a real cached-image terminal case; title/year/rating/cast and poster pixels captured |
| Shows card uses its pane height at 160 | same Rust command | `verify.log`, `shows-160x45.txt` and three new half-block poster goldens | PASS: card rectangle x110/y3, 48x38; image remains 24x18 |
| Help removes the kitty placement with no ghost capture | same commands | `verify.log` (`help_deletes_kitty_placement_before_drawing_and_restores_it_once`); `final_live/tui_smoke/poster_overlay_cleanup.jsonl` | Real encoder PASS: one delete before Help, one restore, no retransmit on unchanged frames. Portable terminal capture PASS: no poster cells under Help. **Real kitty image-plane capture unverified:** headless tmux explicitly reports kitty=false |

Final TUI suite: **142 passed**, including 40 reviewed goldens (`verify.*`). Full `cargo test --locked` once: **416 passed, 0 failed, 2 ignored** (`full.*`). Final binary passes all four tmux cases and the forced-pane control (`final_live/`, `final_control/`), plus existing native search/card/single-connection regressions (`regressions/daemon/*.jsonl`). CI wiring controls, Python compilation, generated keymap, size gate (`final_sizes.*`) and diff checks pass. An initial edit-script quoting error left a correction batch unapplied; retained `check.*` shows the resulting compile errors, then the complete batch and positive tests passed. No release/all.sh between track beads. Remaining evidence: a kitty-capable graphical terminal image-plane capture; hosted CI has not run from this worktree. Neighbouring harness changes register `poster_overlay_cleanup` and preserve existing search assertions under the centered overlay.


### P4-05 — readable text and monochrome feedback

Computed dim, accent-text and low-chroma roles once on theme load; secondary text no longer uses muted/dark_foreground. Theme loading respects private XDG_STATE_HOME before the home fallback. Error rows/status/modal messages use worded markers and bold/reverse on low-chroma themes; confirmed actions use plain ✓ feedback, while pending/information stays unmarked. Files/provider/settings errors and Help accents now consume the same roles. Cache signatures include the semantic roles. Existing theme fixtures, IPC records/examples and 40 text goldens are unchanged. Minimal neighbouring changes cover confirmed feed/Shows actions, modal errors, and the smoke/CI case registry.

| Acceptance | Command | Evidence under `target/test-logs/fbt-p4-05-vdb/` | Observed |
|---|---|---|---|
| Dim and foreground reach 4.5:1 on every installed and owner theme | `cargo test --locked -p fbittorrent tui:: -- --nocapture` (`contrast_every_installed_theme`) | `completed_tui.{log,jsonl}` | PASS; each path/rung/hex/ratio printed; owner dim #b6b6b6 = 10.355676:1, foreground 21:1; Rose Pine foreground 6.657281:1 |
| Low-chroma owner=true, Tokyo Night=false | same Rust command (`low_chroma_detection` and feedback controls); `python3 …/tui_smoke.py target/debug/fbittorrent --log-root …/verified_live_cases` | `completed_tui.log`; `verified_live_cases/tui_smoke/low_chroma_theme_markers.jsonl` and full pane artifacts | PASS; real error row and owner ! status prefix; Tokyo Night error text has no ! prefix; confirmed ✓, pending/info and modal role controls pass |
| Accent fallback on Latte/Miasma/Rose Pine/Kanagawa, absent on owner/Tokyo Night | same Rust command (`accent_fallback_table`) | `completed_tui.log` | PASS all six named themes; ratios and selected fallback printed |
| Dim fixture hex agrees within ±1/channel | same Rust command (`dim_matches_fixture`) | `completed_tui.log` | PASS every checked-in reference theme, including rung identity; known-bad theme still fails the contrast gate |

Final focused suite: **150 TUI tests passed**, including 40 unchanged goldens (`completed_tui.*`). Full `cargo test --locked` once: **424 passed, 0 failed, 2 ignored** (`full.*`). All five native tmux smoke cases pass together (`verified_live.*`, `verified_live_cases/`); forced-pane failure control passes (`forced_control.*`, `forced_control_cases/`); five CI wiring controls pass (`ci_wiring.*`, copied `ci_wiring_cases/`). Python compilation, size gate (`final_sizes.*`) and diff checks pass. All changed src files are <=900 lines. No release/all.sh between track beads.

E2E deviation: this branch lacks P1-14's injection hook, so the private fixture makes a real live recheck fail by blocking its metadata-cache writer, then performs a real failed open; no API state or pane text is fabricated. It uses the downloading fixture because a restarted completed torrent is dormant. Earlier failed fixture runs are retained in `live/` and `final_live_cases/`; the positive combined run is `verified_live_cases/`. No acceptance criterion unmet. Hosted CI remains unexecuted from this worktree. Follow-up: the shell's existing Palette.js accentText helper lacks the canonical near-foreground guard (Kanagawa); the frozen reference fixtures were left intact.


### P4-08 — Log, toasts and action history

Added L's daemon-backed Log modal and the shared hash-filtered Details Log tab, level/source filters, cached one-line notifications (two visible, 60 cells, 10 s, no animation), and a 500-entry/2048-byte local outcome history. The ftui NotificationQueue/Toast owns lifetimes; a compact cached adapter avoids Toast's built-in three-row chrome. Incoming logs preserve a scrolled viewport. Modal layering, mouse coverage and kitty cleanup follow the overlay. Neighbouring changes register actions, regenerate README/keymap, subscribe on the existing API connection and extend the native smoke case registry. No existing IPC fixture or example changed. Six existing goldens change only for the new Details tab/Help row; added log-120x30.txt.

| Acceptance | Command | Evidence under `target/test-logs/fbt-p4-08-p3e/` | Observed |
|---|---|---|---|
| Action failure remains after toast expiry | `cargo test --locked -p fbittorrent tui:: -- --nocapture`; `python3 …/tui_smoke.py target/debug/fbittorrent --log-root …/final_live_cases` | `final_tui.*`; `final_live_cases/tui_smoke/log_history.jsonl` | PASS: synthetic clock control expires toast/status; real terminal waits 10.5 s, L retains failed-open message and P1-14 injected engine diagnostic delivered after TUI subscription |
| Overlapping toasts preserve footer | same Rust command (`toasts_stack`) | `final_tui.log` (both rectangles and footer printed at 80/120/160) | PASS: one-row rectangles <=60 cells, newest at bottom, no footer intersection; third message replaces oldest visible while history retains all |

Focused final suite: **158 passed**, including 41 goldens and filter/hash/scroll/bounds/layering controls (`final_tui.*`). All six real native smoke cases and both setup cases pass (`final_live.*`, `final_live_cases/`); forced-pane control, CI wiring, keymap generation/check, Python compilation, size gate and diff checks pass. Full `cargo test --locked` ran once: **481 passed, 1 failed, 3 ignored** (`full.*`). Its sole failure was a new filter assertion searching the whole pane, including the independent status footer; changed it to assert the filtered viewer's exact row count, then all 158 TUI tests passed. No production source changed after that full run; no second full run or all.sh per track cadence. Initial API-type/tuple compile failures and a Path-vs-string fixture setup failure are retained. Acceptance criteria all met; a wholly green full-suite invocation after the test correction and hosted CI are not claimed. Follow-ups: none within this bead.


### P5-04 — deferred TUI limits slice

Added z and the confirmed Summary turtle indicator, plus F6's four rate fields (K/M, empty/0 unlimited, checked range, drafts/cancel). Summary/Settings/Log share one initial subscription on the existing connection. Settings now apply a minimal API patch, request the declared engine restart when necessary, and read back the actual Settings; unedited fields survive concurrent changes. Neighbouring changes update prompt focus/mouse offsets, generated README/keymap, native smoke registration and six Help/Settings goldens. Existing IPC fixtures/examples are untouched.

TUI acceptance: `python3 …/tui_smoke.py target/debug/fbittorrent --case turtle_limits --log-root …/live_turtle_cases` passes; off/on took **630/410 ms**, within the one-second TUI push interval. The final combined seven-case smoke passes (`final_live.*`, `final_live_cases/`; off/on **651/362 ms**). Its real API observer verifies F6 128K→131072 B/s, retained alt_download=65536, alt=true, and every per-torrent upload cap still 32768 under global unlimited. Unit draft/validation/minimal-patch/three-size indicator checks pass; no mutation fabricates the indicator state. Throughput, daemon-restart persistence and other UIs remain the already implemented daemon/dashboard halves; no new credit for those tests here.

`cargo test --locked -p fbittorrent tui:: -- --nocapture`: **161 passed** (`focused_update.*`, deliberate goldens reviewed); full `cargo test --locked` once: **485 passed, 0 failed, 3 ignored** (`full.*`), including P4-08's corrected filter control. CI wiring, keymap check, Python compilation, size gate (`final_sizes.*`) and diff checks pass. Initial check exposed a typed enum conversion and module visibility; both fixed before the green build/tests. All changed src files <=900 lines; no release/all.sh between track beads. TUI criteria met, no TUI item unmet. Follow-up already assigned to P4-09: remove the now-unused legacy queue-settings compatibility helper.


### P5-07 — deferred TUI disk slice

Verified the existing pushed Summary.disk banner and typed error/disk_full→! Disk state at 80/120/160 columns, with three new owner-theme goldens. Made absent free space read “free space unknown”; known binary-unit sentences are unchanged. Tests cover the actual footer position, nine-cell worded state, disk > VPN > stopped > upgrade priority, the record-error fallback and clearing the warning after confirmed recovery. These are renderer/interaction controls; the tmpfs/ENOSPC/automatic-retry and add-sheet criteria belong to the daemon/dashboard halves and are not claimed as new live evidence here.

`FBT_UPDATE_GOLDEN=1 cargo test --locked -p fbittorrent tui:: -- --nocapture`: **166 passed**, including 44 reviewed goldens (`target/test-logs/fbt-p5-07-gei/focused_fixed.*`). Full `cargo test --locked` once: **490 passed, 0 failed, 3 ignored** (`full.*`), verifying the checked-in goldens without regeneration. Size gate (`final_sizes.*`) and diff check pass; src files remain <=900 lines. The first fixture selector selected hello/the empty torrent list and failed; the corrected selector requires the non-empty frozen list. Old goldens and all IPC fixture/example bytes are unchanged. No release/all.sh or repeated daemon acceptance runs. TUI criterion met, no TUI item unmet. Follow-up/dependency for the next slice: P6-01's GET/reannounce is present but Topic/Item/bus support for torrent:<hash>.trackers is absent on this branch.


### P6-09 — deferred TUI peer port slice

Added F6's peer port draft (0 automatic, checked 0–65535), numeric editing/cancel, minimal API settings patch and confirmed readback. Removed the redundant client app.restart: app.settings.set already owns the restart. The native peer_port case proves one epoch increment with starting/running events, unchanged owner and observer/TUI connections, and a real TCP listener on the chosen port. Neighbouring harness registration and three Settings goldens change; existing IPC fixtures/examples are untouched.

`cargo test --locked -p fbittorrent tui:: -- --nocapture`: **167 passed** (`target/test-logs/fbt-p6-09-omx/focused_fixed.*`). Debug build passes (`build_fixed.*`); focused live port case and all eight native smoke cases plus both setup cases pass (`live_port.*`, `final_live.*`, corresponding `_cases/`). The standalone port run applied 60005 with epoch 1→2 and unchanged instance/client counts. CI wiring, Python compilation, size gate (`final_sizes.*`) and diff check pass. Full `cargo test --locked` once: **491 passed, 0 failed, 3 ignored** (`full.*`). Initial fingerprint syntax failure retained in earlier logs. TUI criteria met; daemon inhibitor behavior belongs to the daemon half and is not re-claimed. No release/all.sh between track beads. Follow-ups/dependencies: P6-01 lacks the canonical trackers topic on this branch and main; P6-13's dismiss API is on main d577abe but not yet integrated here.


### M1a — review finding 2, v0.1 CLI compatibility (fbt-m1a-kug)

Routed Status through the existing legacy read path and refused a live old engine before offline mutation fallback, including an owner without the current lock. The new daemon.py `old_engine_compat` case uses an explicitly fake, bounded v0.1 Unix-socket owner (no api.sock), a row absent from the offline ledger and distinctive rates. List/status read successfully; stop/add/start/delete return exactly “an older fbittorrent app is running; quit it with Q and retry”, exit 4 / code_name backend. The case verifies GET-only requests, unchanged config/ledger and no manager/launcher calls, with autostart enabled. Neighbouring harness changes are one registered module plus its fixture; no frozen contract changes.

Evidence under `target/test-logs/fbt-m1a-kug/`: the pre-fix binary reproduces Status's exit 4 and Stop/Delete's wrong refusal (`before.*`, `before_cases/`); `python3 …/daemon.py target/debug/fbittorrent --case old_engine_compat --log-root …/after_cases` passes all **19 steps in 117 ms** (`after.*`, `after_cases/daemon/old_engine_compat.jsonl`, `.requests.json`). Existing `agent_contract.py --case reads_never_start --case offline_records` controls both pass (`offline_controls.*`, `offline_cases/`). `cargo test --locked -p fbittorrent cli:: -- --nocapture`: **7 passed, 1 existing ignore** (`focused.*`); full `cargo test --locked` once: **491 passed, 0 failed, 3 ignored** (`full.*`). Debug build, Python compilation, size gate (`sizes.*`) and diff check pass. Assigned finding met; the real old-binary/manual upgrade check and the milestone-wide main gate are not claimed. No release/all.sh between track fixes. Follow-ups: none for the assigned finding.


### M1a native gate recovery / P4-03 service-screen checkpoint

Merged main 027886f as instructed. Native RSS/Shows now project daemon topics and use rss.grab/shows.grab/read/undo/follow/check operations over the existing TUI connection; no native Shows load/save or on-open client refresh. Removed the two-second poll, added base topic subscriptions and bounded serialized detail-topic replacement, retained confirmed rows and absent metrics. F6 reads an authoritative baseline before displaying its draft, fixing the stale pushed Settings value. IMDb IDs entered in Follow use the id argument. Neighbour src/api/ops/shows.rs maps HTTP sources in today's saved release data to AddSource::Url, retaining the one-explicit-fetch behavior. New service controls exercise unusable client-local rss.json/shows.json with engine.sock hidden, read/undo/mark/read/grab semantics and one active TUI connection.

Harness corrections preserve every behavioral assertion: the on-open RSS fixture is explicitly stale (the daemon owns freshness); mouse targets must be visible after pushed updates/toast expiry before coordinates are used; Settings waits for its second loaded dialog; the seed-count capture accepts the truthful “peers unknown” wording; narrow Details visits Log and returns to loaded Files before Space. Main native recordings now pipe the actual tmux PTY output, retaining the 1003-byte assertion while avoiding nested recording-PTY resize lag. Native --log-root preserves separate runs. Read/undo controls wait for the enabled Undo action. No old fixture record, IPC.md example or render golden changed.

All evidence below is under `target/test-logs/fbt-p4-03-r5p/`. Release built once successfully with `flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope -p MemoryHigh=10G -p MemoryMax=14G --quiet -- env XDG_RUNTIME_DIR=<private> cargo build --release --locked` (`priority_release_build_bus.*`). The first wrapper attempt never reached Cargo: its private runtime hid the user scope bus (`priority_release_build.*`); the scope transport now uses the real user runtime and the scoped build keeps private app XDG.

| Requested release criterion | Command suffix | JSONL evidence | Observed |
|---|---|---|---|
| Scrolled mouse selection/hover identity | `native.py target/release/fbittorrent --case mouse_selection_at_scroll_offset --log-root …` | `mouse_selection_at_scroll_offset_release_cases/native/mouse_selection_at_scroll_offset.jsonl` | PASS |
| Truthful seed/peer counts | same, `--case no_fake_seed_counts` | `no_fake_seed_counts_release_cases/native/no_fake_seed_counts.jsonl` | PASS, real panes retained |
| Shows inbox/read/undo/stable targets | same, `--case shows_inbox_and_targets` | `shows_inbox_and_targets_release_cases/native/shows_inbox_and_targets.jsonl` | PASS, single HTTP grab assertion unchanged |
| Settings queue limit, admission and VPN restart | same, `--case settings_queue_vpn_restart` | `settings_queue_vpn_restart_release_cases/native/settings_queue_vpn_restart.jsonl` | PASS, 58 steps including 3 active/1 queued |
| Narrow Details and real Files selection | `daemon.py target/release/fbittorrent --case tui_details_narrow --log-root … --jobs 1` | `priority_daemon_release_cases/daemon/tui_details_narrow.jsonl` | PASS |

The same daemon release run also passes tui_single_connection and tui_service_screens (3/3 cases, `priority_daemon_release.*`). `cargo check --locked --tests` passes before and after the service batch (`integrated_check.*`, `services_check.*`); focused TUI tests **169 passed** (`priority_unit_fixed.*`), including unchanged goldens and 5000-row cache controls. Full `cargo test --locked` ran once on the final production source: **523 passed, 0 failed, 3 ignored** (`full_after_priority.*`). Later changes are Python fixture synchronization/recording only. Python compilation, source size gate and diff checks pass. Earlier compile, asynchronous-fixture and owned hidden-socket cleanup failures remain in the debug evidence; the final release cases are green. Assigned five-case priority met; no all.sh/main gate or hosted CI claimed. P4-03's native data/interaction criteria are proved here; removal of the old non-native conditional code remains the next cleanup. Compatibility/version-warning work is skipped under the owner's standing no-compat rule.

### P4-03 / fbt-p4-03-r5p — complete API-only TUI

Removed the actual legacy client construction and all TUI-side search/browse/IMDb/RSS/Shows transports and store writers, including the abandoned provider metadata handoff. Screen commands share the presence API connection; no data-poll subscription exists. Release projections retain only the selected topic's slice; metadata supplies full unread counts. Last-known service rows dim on disconnect. Restored the UI-only spinner/theme clock. Ordered API delivery now drains at most 16 queued updates per acknowledged message (128-update source queue), avoiding one render-loop round trip per snapshot fragment. Neighbours `src/daemon/mod.rs`, `src/api/bus.rs` and `src/api/bus/tests.rs` record and deliver engine transitions at their source after releasing the slot lock: a fast restart cannot hide the starting phase between sampler ticks. No frozen wire record, IPC.md example or golden changed. Existing pure reducer/renderer tests remain; the failed-grab callback now uses ServiceDone and the alias-cache test uses native mode.

Evidence root: `target/test-logs/fbt-p4-03-r5p/`; every command has matching `.jsonl` and `.log`.

| Acceptance | Command / artifact | Observed |
|---|---|---|
| No qbt client or local service/store paths in TUI | forbidden-path rg and source size gate, `completed_client_static.*` | No matches; size gate PASS; changed src files <=900 lines |
| No two-second data poll | `cargo test --locked -p fbittorrent tui:: -- --nocapture`, `api_batch_unit.*` | 172/172 PASS; UI clock, ordered 16/16/4 batches and no poll subscription controls |
| One connection | `daemon.py target/debug/fbittorrent --case tui_single_connection …`, `api_batch_live_cases/daemon/tui_single_connection.jsonl` | PASS, engine.sock hidden; one TUI; actual Files selection changed |
| Interactions/search/RSS/Shows/recovery | six cases in `api_batch_live.*` / `api_batch_live_cases/daemon/` | 6/6 PASS, including poisoned client-local stores, search provider ancestry, owner restart, automatic reconnect and narrow Details |
| Every store fixture expect model; unchanged render goldens | same final TUI test command, `api_batch_unit.*` | PASS, all fixture expectations and 44 goldens; 5000-row cache controls unchanged |
| Native TUI smoke | `tui_smoke.py target/debug/fbittorrent --log-root …`, `api_batch_smoke.*` / `api_batch_smoke_cases/` | 8/8 PASS at 80/120/160 columns; turtle off/on 206/310 ms (<1000 ms); exactly starting/running for one port restart |
| Rust suite | `cargo test --locked -- --nocapture`, `api_only_full.*` | 526 PASS, 3 existing ignores; run before the final bounded-batch adjustment, followed by the final 172-test TUI suite and both live suites above |

`api_only_bus_unit.*`: 7/7 bus tests pass, including both rapid phases delivered once per connection without a sampler tick. The initial smoke exposed the turtle backlog and sampled-transition races; assertions and deadlines were retained. The independent wire trace (`turtle-trace.json`) received the changed summary about 160 ms after the key while the old terminal remained stale. One smoke attempt was discarded because it began before the final link completed (`api_only_smoke_final.*`); the final named runs above use the completed debug binary. Earlier diagnostic failures remain in their own log roots. Compatibility/version-warning work is skipped under the owner's standing no-compat rule; old global paths are removed next in P4-09. No all.sh or hosted-CI claim. No unmet P4-03 criterion or remaining follow-up in this slice.

### P6-10 / fbt-p6-10-2mq — Shows release notifications

After a successful saved check, the Shows owner sends one notice for newly discovered unread keys, with a bounded three-name summary and remainder count. Existing or read keys do not count. Interactive TUI/dashboard presence suppresses delivery; unchanged checks stay silent. The existing bounded notifier, notify=false and --no-notify rules apply; the window action runs `dashboard --view shows`. No new API field, operation, fixture or wire change. Neighbour `src/api/bus.rs` adds only the owner-to-notifier forwarding method. README documents the behavior.

Evidence root: `target/test-logs/fbt-p6-10-2mq/` (matching command `.jsonl`/`.log`).

| Acceptance / control | Command / artifact | Observed |
|---|---|---|
| EZTV check with new releases sends one notification | `daemon.py target/debug/fbittorrent --case shows_notify … --log-root …/live`, `live/daemon/shows_notify.jsonl` | PASS: 4 new releases, one recorded notify-send argv, three names plus +1 more, Shows action, unchanged empty torrent ledger |
| Second unchanged check sends none | same, `--case shows_notify_none_when_unchanged`, named JSONL | PASS: actual second EZTV request; new=0, notice count remains one throughout a one-second observation |
| Interactive suppression and resumption | same, `--case shows_notify_suppressed_interactive`, named JSONL | PASS: API-declared TUI and visible dashboard each suppress a positive check; background check after both close notifies once; no automatic source fetch or download |
| Text/action/queue controls | `cargo test --locked -p fbittorrent daemon::notify:: -- --nocapture`, `notify_unit.*` | 3/3 PASS: singular/three/five, markup escaping, at most three names, +2 more, exact Shows dashboard arguments, existing queue bound |
| Existing behavior | same daemon run, `--case shows_inbox_semantics --case notify_off`, `shows_notify_live.*` | All 6 emitted cases PASS, including config/flag notification-off controls |
| Build and full Rust suite | `cargo check --locked --tests`, `cargo build --locked`, `cargo test --locked -- --nocapture`; `check.*`, `debug_build.*`, `full.*` | PASS; full suite 528 passed, 0 failed, 3 existing ignores (also exercises final P4-03 bounded batching) |
| Static checks | Python compilation, source size gate, diff check; `static.*` | PASS; changed src files <=900 lines |

All fixtures use private XDG roots, a loopback EZTV server and the prescribed recording notify-send shim. Exact argv artifacts are retained beside case JSONL; no live desktop notification or window launch is claimed. No unmet criterion, deviation or follow-up. No all.sh between track beads, per owner instruction.

### P4-09 / fbt-p4-09-fja — one native backend and API v1 transport

Removed the WebUI client/tests, old service setup/control, migration writer/importer, four-byte listener/reader, fallback probes and old-owner routing. The daemon retains its lifecycle and stable instance lock across engine restarts. Current CLI 1.x torrent output shapes remain; historical engine characterization inputs now live exclusively in a test fixture DSL that executes typed operations, with all historical JSON bytes and state assertions preserved. Removed scheduling/high/max actions; `s` cycles torrent sorting, `S` reverses and `f` follows. Generated keymap/README updated; five goldens change only eight footer/help lines for the new sort actions. Minimal neighbouring CLI/service helpers retain today's data operations and use API v1 for adds; no backend transport remains in qbt.rs. The old live-service regression entry point now runs the isolated native agent contract. Harness probes, cancellation and UI-connection cases use API v1. Frozen IPC fixtures and docs/IPC.md are unchanged.

Evidence root `target/test-logs/fbt-p4-09-fja/`; every named command has `.jsonl` and `.log`. `files.txt` contains the changed-path manifest.

| Criterion / check | Command / artifact | Result |
|---|---|---|
| No removed backend executable or old socket reference in production sources/plugin | `rg -n 'qbittorrent-nox|engine\.sock' src plugin`; `static.*` | PASS: no matches (rg exit 1); keymap check, size gate (all src files <=900), diff hygiene and unchanged frozen contracts also pass |
| Agent contract, including retained stubs | `agent_contract.py target/debug/fbittorrent --log-root …/agent_final`; `--case move_verb --case events_follow_completion --log-root …/agent_retries_cases` | All 19 distinct cases pass; seq/first-last exit 2 with inspection hints, reconcile exits 0 without starting an owner. Initial move started parked with a transient `Waiting for lo`; initial completion took 4.14 s against 2 s. Both pass isolated reruns with original assertions/deadlines; no causal claim or test relaxation |
| Today's native data opens unchanged | `daemon.py … --case native_data_unchanged --log-root …/data_final_cases` | PASS: real ledger identity, cached RSS read state, Shows history and settings load; SHA256 of all six input files (including payload/metainfo) unchanged on read; only api.sock created. Owner live data was never read or modified |
| Native lifecycle and UI controls | `native.py target/debug/fbittorrent --log-root …/native_final_cases`; daemon cases in `daemon_cases/` and `api_tui/` | PASS: all 26 native cases; readiness, parked intent, in-flight-add shutdown and retained owner/lock; three TUI-only API cases pass with old socket structurally absent |
| Rust suite and current TUI goldens | `cargo test --locked -- --nocapture`; `full_final.*`, `tui_final.*` | PASS: 513 passed, 3 ignored; 173 TUI tests pass, including all 44 goldens. First full run exposed exactly the five intentional Sort golden changes; reviewed/replaced eight lines, then reran the complete suite to establish a green commit |
| Gate wiring | `ci_test.py …`; `ci_wiring.*` | PASS: all five CI gate checks, including the default native TUI wiring |

No unmet P4-09 criterion. The obsolete migration-completion check is skipped under the owner's no-compatibility rule; isolated current-format proof replaces any access to live data. No release build, all.sh or hosted-CI claim (track cadence). Follow-ups: the opt-in historical engine benchmark and standalone namespace-VPN driver still assume a TUI-owned engine; migrate those tools to the managed daemon harness before claiming their budgets. Current CLI search/RSS/Shows service ownership remains the service-CLI track's concern; it is not a legacy backend transport.
