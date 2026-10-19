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
