# Review log for `PLAN.md`

One entry per round. Each entry says what changed structurally, what was added, what was corrected
(including wrong citations) and what was deliberately left alone, so a later round can follow the
trail. Work item IDs are stable across rounds; a removed or merged item keeps its ID with the note
"superseded by …".

## Round 0 (draft, 2026-10-18)

The first plan: the daemon decision, API v1 sketch, Quickshell and TUI designs as view lists, the
feature decision table, 77 work items (P0-01..P7-09 minus gaps) with dependencies, 3,142 lines.
No external review.

## Round 1 (ambition, 2026-10-18)

Brief: the owner found round 0 "a decent start that barely scratches the surface"; make it much
better in every way without losing features, with a real visual design specification per surface,
a complete and precise API v1, re-examined feature decisions, and comprehensive tests with logging
on every item. Skills applied: `planning-workflow`, `idea-wizard`, `ux-audit`, `frankentui`,
`agent-ergonomics-and-intuitiveness-maximization-for-cli-tools`,
`just-say-no-to-process-porn-and-ceremony`.

Result: 4,676 lines (from 3,142); 99 IDs of which 98 are active items (P7-07 superseded; P7-08
conditional on the owner), 8 phases (P0..P7), 7 milestones (M0..M6).

### Structural changes

- §0 gained §0.2 "What optimal means" (a today/target table per dimension), eight assumed
  decisions (A1..A8; A5..A8 are new: magnet handling, ready-to-watch, clipboard watching, the
  monochrome theme as the design constraint) and §0.7 "Round 1 changes at a glance".
- §2 gained §2.7 "Better than qBittorrent, for this user": eleven capabilities with the item and
  the proof for each. §2.6 gained four non-goals (tracker scrape, web seeds, automatic torrent
  management, and the reasons).
- §3.2 is renumbered around a new §3.2.7 "Daemon lifetime: the state machine" (states, transitions,
  observables, crash recovery and stale state). VPN-while-headless is its own subsection (§3.2.9).
  §3.4.3 is now "Handshake and capability negotiation" with the full `caps` table tied to items;
  §3.4.7 specifies revisions, the slow-client policy and the reasoning for the 512-frame queue;
  §3.4.8 keys the ref ring by client name and defines post-restart behaviour.
- §4 is rewritten as a visual design specification: §4.2 tokens (every one verified in
  `/usr/share/omarchy/shell/Commons/`), layout grid, typography roles, iconography; §4.3 row
  anatomy for the popup and the dashboard table, column priority by window width, the shared
  state vocabulary; §4.13 motion rules gated on `Style.duration`/`reduceMotion`; §4.17 test hooks;
  §4.18 "what is borrowed, from where, and why" with file:line into omaqbt, Sprout, Fragments,
  Transmission and qBittorrent.
- §5 gained §5.5 "Visual language for the TUI": colour roles with a contrast rule, low-chroma
  detection, the glyph+word state column with a width-1 constraint, typography and borders, poster
  card rules; §5.6 now has mockups at 80 and 120 columns and a details modal; §5.8 lists the ftui
  0.6 widgets the TUI uses with their registry paths.
- §6.2 re-examined every row and added nine (28..36): ready to watch, show quality, poster grid,
  clipboard watch, undo, agent event stream, tracker scrape (out), web seeds (out), sleep inhibitor.
  §6.3 gained a CLI column and the new features' end-to-end specifications.
- §9 now has: a test logging contract (§9.1), items with Do/Why/Accept/Tests/Depends and named
  unit and e2e cases on every item, an explicit adjacency-list dependency graph with recomputed
  topological levels and an orphan/cycle check (§9.4), a per-phase definition of done with scripts
  and one concrete human check (§9.5), a verification script list (§9.6, Appendix E), and demo
  scripts for M1..M4 (§9.7).
- Appendix A is a complete typed catalogue (every op with arg and result types and `Mutating`,
  every event's fields, every object's fields with "absent when", error codes with `details` and
  CLI exit mapping, field provenance for `Torrent`, two worked examples). Appendix E (test contract
  and scripts) and Appendix F (CLI to API mapping with exit-code reachability and the determinism
  rule) are new.

### Added work items

P0-15 (test logging contract and shared harness module), P0-16 (golden-render harness), P1-18
(agent `events` and `api` verbs), P2-08 (design tokens, visual capture harness, contrast test),
P3-17 (poster-rich browse grid), P3-18 (show quality per show and inbox filters), P4-11 (poster
card rules), P5-05 (ready to watch), P5-06 (fork: have-bitfield snapshot accessor), P6-13
(clipboard magnet watch, opt-in). Every existing item gained named unit and e2e cases.

### Removed or merged

- P7-07 "TUI retirement review" is superseded: a review meeting produces no capability, the owner
  decided to keep the TUI (A2), and the per-kind counters it would have read stay in P1-14 at no
  cost. The ID remains in the table with the note.
- §10.3 dropped Q7 (middle click) and Q8 (marketplace) because the plan decides them with a
  setting (DL-12) and a non-goal (DL-17); it added Q9..Q11 (handler mode, ready-to-watch threshold,
  clipboard watching).

### Corrections to round 0

Citations checked against the working tree: about 100 `file:line` references in `src/`,
`plugin/`, `vendor/`, the Omarchy shell, the ftui registry crates and the research clones. Nearly
all were right. Corrected or tightened:

- `src/tui/mod.rs:1699-1703` → `1699-1702` (the lifecycle checks; 1703 is the tick).
- "ftui ships `Sparkline` [FO §3.3]" cited ftui 0.7; the pinned 0.6 has `sparkline.rs`, `table.rs`,
  `tabs.rs`, `virtualized.rs`, `log_viewer.rs`, `toast.rs`, `notification_queue.rs` in
  `ftui-widgets-0.6.0/src/`; the plan now says so and pins DL-29 (no ftui upgrade for M3).
- D2 is now mechanistically grounded: ftui's signal thread waits `SIGNAL_SHUTDOWN_GRACE = 2 s`
  for the runtime to clear the pending signal, then calls `exit(128+sig)`
  (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`; runtime side
  `ftui-runtime-0.6.0/src/program.rs:111-117`), which explains the observed 2.2 s; P0-08 now names
  the API (`ftui_core::shutdown_signal::{pending_termination_signal, clear_pending_termination_signal}`)
  and two candidate fixes.
- §4.7 (round 0) said the dashboard's `ConfirmDialog` focuses Cancel; the shell's component
  defaults `selectedIndex: 1` (the destructive button, `Ui/ConfirmDialog.qml:12,97-106`). The plan
  now requires `selectedIndex: 0` on every open and a test (DL-25).
- Round 0's "unverified" summon question is answered: `shell.qml:1161-1172` routes a plugin with a
  `panel` kind to the panel loader, `computePanelEntries` (`shell.qml:1356-1372`) includes every
  enabled plugin with a panel kind, and `PluginRegistry.isEnabled` (`:148-163`) counts a bar-layout
  entry as enabled; the owner's `shell.json:38` lists the plugin only in the bar layout.
- Verified on this machine and moved out of "unverified": `notify-send -A/--action` (implies
  `--wait`, prints the action name), `/usr/bin/systemd-inhibit`, `WAYLAND_DISPLAY` and
  `DBUS_SESSION_BUS_ADDRESS` in the user manager, `grim` for captures, the font
  (`JetBrainsMono Nerd Font` via the `monospace` alias).
- The owner's palette is now quoted from `colors.toml`: `muted = #7a7a7a` exists and reaches about
  4.9:1 on black while `dark_foreground = #505050` does not; `theme.rs:66-67` prefers the latter.
  New defect D16 and the §5.5.1 contrast rule follow. D17 (the tab bar's "omarchy" theme name)
  and D18 (the menu's stale qBittorrent gate, `omarchy-menu.jsonc:36`) were added.
- The plugin's dim factor (`Qt.darker(fg, 1.55)`, `Panel.qml:18`) differs from first-party panels
  (1.4, `Ui/PanelHero.qml:23`); the plan standardises on 1.4 (DL-23). The popup's literal 400 ms
  animation (`Panel.qml:410`) becomes `Style.duration(400)`.
- `NativeRuntime` builds 1 worker and 1-2 blocking threads (`src/native_runtime.rs:44-50`) while
  `SessionOptions` asks for 2 (`src/engine.rs:432-434`); §1.2 states both instead of "2 workers".
- Cargo citations: `quick-xml` is `Cargo.toml:34`; the `[patch.crates-io]` block is `:49-52`; the
  workspace is `:54-57`. `Record` is `src/engine.rs:38-53`. The poster-card width check is
  `src/tui/mod.rs:2223` and the overlay check `:2229`. The browse body cap is `src/browse.rs:60`.
- §3.2 cross-references were renumbered after the state-machine insertion (stop policy §3.2.10,
  crash §3.2.11, upgrades §3.2.12, status §3.2.13, no-systemd §3.2.14, logging §3.2.15).

### Deliberately left

- The architecture (daemon + Quickshell + TUI + CLI), the A1..A4 defaults, the NDJSON transport,
  the compat period until v0.4.0, the phase order and the M1 definition. The research still
  supports them and the owner's stated preference matches.
- The item IDs and the phase boundaries, so beads conversion can map one to one.
- The two platform exceptions in the keymap (Space, Enter in the popup).
- The decision not to add a command palette to the TUI until a task needs it ([UX]).
- No CPU optimisation work (DL-18).

### Validation loop (planning-workflow)

- Self-containment: P5-06 (fork accessor) and P2-08 (visual harness) were read alone; both name
  their files, APIs, acceptance and tests without the rest of the plan.
- Dependency graph: the adjacency list in §9.4 was recomputed into levels; no cycle, no orphan
  (P7-08 is intentionally conditional).
- Justification: DL-09 (512-frame queue), DL-23 (dim rule), DL-25 (confirm focus), DL-26
  (ready-to-watch threshold), DL-28 (ref ring) each carry a "why".
- Steady state: not reached; this round was structural by design. Round 2 should be an external
  review with the planning-workflow prompt.

### Still not grounded (carried in §10.4)

`QML_IMPORT_PATH` for isolated `qs -p`; `hyprctl output create headless`; drag and drop on a
`FloatingWindow`; GitHub Actions availability; nova3 licences; explicit caps on RSS/IMDb/poster
reads; the private flag in the fork; a cheap container duration read for `buffered_s`; bounding
`wl-paste --watch`; which of P0-08's two fixes the ftui runtime needs on a dead pty. Each is
assigned to the item that verifies it.

## Round 2 (integration), 2026-10-18

Inputs: `reviews/round-2-architecture.md` (A1–A13 defects, B1–B15 improvements, C1–C3 questions,
D DAG-check output) and `reviews/round-2-product-ux.md` (A1–A16, B1–B16, C1–C9). Both read in full;
every `-` line of every hunk was matched against PLAN.md by exact content before replacing it (a
miss aborts, nothing is applied by line number). Result: PLAN.md 4,676 → 5,075 lines; 101 IDs
(100 active + P7-07 superseded; new: P1-19, P5-07); §9.4 has 107 nodes over 19 levels; DAG and
mockup-width scripts both clean (outputs below). Nothing under `src/`, `plugin/`, `vendor/`,
`Cargo.*`; no git state command.

### Verdict per review item

Verdicts: **applied** (the hunk as written, modulo whitespace), **adapted** (the intent kept, the
text or numbers changed, reason given), **rejected** (not taken, reason given).

**Architecture review**

| Item | Verdict | Reason / where |
|---|---|---|
| A1 M1 demo `cp` over a running binary | applied | step 9 now installs with `install.sh` (`mv` over the file; `cp` gives ETXTBSY) and checks `server.build` is unchanged. |
| A2 P1-10 cannot be tested with the v0.1 TUI | adapted | accepted the analysis (`src/main.rs:787` opens the TUI's own engine); P1-10's accept/tests now run the pinned v0.1.2 **CLI and widget** against the daemon (`native.py --legacy-cli <v0.1> --daemon <v0.2>`); §3.6, §9.5 P1 and Appendix E rows aligned; TUI-as-client cases moved to P1-15. |
| A3 `events` has no replay | applied, merged with B6 | A.7 `events` topic with `seq` ring (1,000 / 10 min, `resync`), A.8 `seq` on every event, P1-18 Do/Tests (`events_since_seq_replay`, `events_resync_when_older`), Appendix F row. |
| A4 `Pieces` RLE unbounded | applied | `have: base64` bitfield in A.9, §6.3 row, P6-02 row, §4.7 Pieces tab. |
| A5 citations | adapted | fixed with my own line checks: `src/config.rs:140-147`/`144-147`, `src/agent.rs:134-210`, `session.rs:140` + `limits.rs:85-91`, search-session row (nova2 pool `min(cpu,4)`); the review's own `src/ipc.rs:147` is `:151` and `src/setup.rs:115` is `:111-114`, used the verified numbers. |
| A6 `autostart` config key ignored | applied | §3.2.3 row (persistent `--no-start`; `up`/`tui`/`dashboard`/`serve` still start), `autostart`, `timeout_secs`, `open_command` in `Settings`. |
| A7 `register-magnet` missing from Appendix F | applied | row added; P1-11 `capabilities.commands[]` derived from the `Cmd` enum with a unit test. |
| A8 `busy` → 1 and `status --quiet` → 1 | applied | both exit 4; reachability paragraph extended; `retryable_never_maps_to_1` test. |
| A9 Draining has no quiesce / mid-request rule | applied | §3.2.7 Draining row (stop accepting → answer in-flight → `bye`); §3.2.4 step 3 (start while deactivating), step 5 (`last-daemon.json`), step 6 (idempotent resend with the same `ref`; exit 8 for `remove`/`recheck`/`move`). |
| A10 unit lacks `KillMode` and start limit | applied | `KillMode=mixed`, `StartLimitIntervalSec=60`, `StartLimitBurst=20` with comments; P1-04 accept/tests (`killmode_mixed`, `start_limit_not_hit`, `start_while_deactivating`). |
| A11 `status` break undeclared | applied | Appendix F intro declares it (`capabilities.changes[]`, `breaking: true`), §0.4 row, P1-11 Do, Q12. |
| A12 offline ledger writes unordered | applied, with B15 | §3.2.3 row gives the lock order and the 2 s retry; P1-11 tests gain `old_engine_sock_reads_only`, the two `wait` cases, `daemon.py::offline_*`. |
| A13 C3 not buildable alone | adapted | C3 (patch) folded into the app commit, which carries manifest + lock + `[patch]`; commits renumbered C3–C6 (§8.1, P0-06, P0-02); §8.2 option B becomes a `cargo package`-shaped copy, not the 57 MB tree. |
| B1 structural frames tick-driven | applied | §3.4.7 bullet: fork accessor `state_changed()` over `state_change_notify` (`torrent_state/mod.rs:212`), P1-08 Do and a fork unit test. |
| B2 snapshot atomicity | applied | §3.4.7 "Atomicity" bullet. |
| B3 split P1-02 | adapted | P1-02 → isomorphic move (M), new **P1-19** typed ops + normalized model (M); P1-03, P1-05, P1-16, P5-06, P6-07 re-pointed in §9.3 and §9.4; the review's hunk quoted an older Tests line, so the split was applied against the current text. |
| B4 slot `Running` only after restores | applied | §3.2.4 paragraph and P0-09 Do: `Running` when the session exists, restores in a spawned task. |
| B5 protocol robustness tests | applied | P1-06 tests: `hello_twice`, `ninth_inflight_busy`, `slow_hello_bytes`, `garbage_table`. |
| B6 `api --list` from one table | applied (in P1-18's Do) | `api::ops::TABLE`, `api --list`, `robot-docs api`, `table_matches_appendix_a`. |
| B7 client identity for refs | applied | `cli-<pid>`/`tui-<pid>`; plugin keeps `oliver.torrents` for resend after reconnect. |
| B8 `hello` carries pid/launcher/log | applied | A.2 result. |
| B9 atomic poster writes | applied | P3-04 Do (`src/imdb.rs:111`). |
| B10 unbounded notifier threads | applied | §3.8: 4-thread queue, `(kind, hash)` dedupe. |
| B11 `flush()` and `SplitParser` cap | applied | §4.4 rules 2 and 5 with the Quickshell source lines. |
| B12 `.torrent` hash known at add | applied | §3.4.8 bullet. |
| B13 `shows.json` re-read | applied | P3-03 Do. |
| B14 seed flag source | applied | §6.3 row names `bitfield: BF`, `has_full_torrent`, `From<&Peer>`. |
| B15 `wait` across restart / idle exit | applied | P1-11 tests; Appendix F `wait` row (one reconnect; exit 8 when the daemon idles out). |
| C1 status break | → Q12 | default: the break, declared. |
| C2 `autostart` | → Q13 | default: keep. |
| C3 Asupersync packaged copy | → Q3 extended | default: fork; packaged copy as fallback. |
| D DAG output and reading | adapted | script re-run after every edit (clean); level table recomputed (19 levels; M6 now 18 as computed); the "plugin lane may start against P1-06" reading added to §9.2 as a note, P2-01's edge unchanged. |

**Product / UX review**

| Item | Verdict | Reason / where |
|---|---|---|
| A1 dashboard geometry negative | adapted | panes 200/360, chrome 636/246/16, fixed columns 748 (26+110+76+120+80+80+64+56+56+80), NAME floor 210, default 1600×900, 1280 claim removed (§4.2.2, §4.3.2, §4.3.3, §4.6, P3-09). The review's tier table re-showed ↑ at 1100–1399 after hiding it at 1400–1599; replaced by monotonic tiers (↑ hidden below 1600, PEERS kept in every tier) with the arithmetic in the table. |
| A2 contrast gate fails on installed themes | applied | computed dim ladder shared by QML and TUI; `muted` never for text; accent fallback; token, typography and P2-08 lines. Fixture format adapted: `contracts/ui/themes/*.toml` copies plus `expected.json`, so CI does not depend on installed themes. |
| A3 TUI 7:1 gate | applied | 4.5:1 with the ratio logged; P4-05 accept/tests extended (accent fallback table, fixture hex). |
| A4 three state tables | applied | one table (`contracts/ui/states.json`) with QML glyph / text glyph / word / `short`; `◌` metadata, `…` queued, `⋯` dropped; §5.5.3 reduced to the cell rule; rows added for `disk_full` and "in your list"; TUI mockups updated. |
| A5 window surface tokens | applied | §4.6 "Surfaces" bullet (`GalleryPanel.qml:71,281`, `ConfirmDialog.qml:13-15,52-58`). |
| A6 vanish under the cursor | applied | §3.2.10 `glance` row, §4.4 rule 8, §4.5 visibility (`root.opened`, `stoppedRecently`), Ready acknowledgement (+ `playable.acked`), P1-12 and P2-04 tests, A.4 `app.client {glance}`. |
| A7 ftui table sorts text | applied | §5.8 row: store sorts typed values, widget renders. |
| A8 Space exception not forced | applied, citation fixed | `PanelKeyCatcher.qml:86-92` (the review's `:289-295` is beyond the 100-line file); §4.12, Appendix B rows and paragraph. |
| A9 key overloads | applied | `U` undo, `=` quality, `P` providers, `s` sort everywhere; paragraph; §5.7 row 12; P4-02 tests. |
| A10 80-column overflow | adapted | the review's four replacement lines were themselves 79/78 cells; every line regenerated to exactly 80 by `mockwidth.py`/`fixwidths.py`, which also found three overflows the review missed (Torrents 80×24 header 84, Shows footer 82, two details-modal rows at 79). |
| A11 impossible 12-cell poster | applied | text card with `(v) poster`. |
| A12 `ConfirmDialog` and `y` | applied, citation fixed | `onEntered` is `ConfirmDialog.qml:122`, not `:51`; §4.9 text; P3-09 accept. |
| A13 "∞" | applied via A2 | token row says "never ∞". |
| A14 idle countdown while downloading | applied | Appendix C popup line. |
| A15 uncheckable acceptance | applied | §0.2 Look row, P3-09 (render timing), P3-16 (README grep), §4.16 `--compare`. |
| A16 upload arrow as idle mark | applied | `󰇚` for idle/stopped, `󰕒` only seeding-only; §4.2.4 rows; "stopped recently" row. |
| B1 chip-row tier as the designed default | adapted | Appendix C 900 px mockup rebuilt to 90 cells with SIZE, PROGRESS, ↓, ETA, PEERS; ↑ not shown (monotonic visibility, see A1). |
| B2 one contrast ladder | applied | P2-08 tests (`dim_ladder`, `accent_fallback`, `states`). |
| B3 `stall_free` | applied | A.9 `Playable`, §3.5, P5-05 Do/Tests, `wait --until stall-free`, §6.3, M4 demo, Q10 reworded; the no-duration fallback is labelled an estimate. |
| B4 disk-full | adapted | new **P5-07** (S) with Do/Why/Accept/Tests; deps P1-08, P2-03, P3-09 **+ P4-03** (the TUI golden needs the TUI on API v1; P4-03 rather than P4-04 keeps M4's weighted path at 40); §6.2 row 37; `Summary.disk`; `reason: disk_full`; M4; §9.4; Daemon lane; §4.11 rows; notification row; `add --json` `disk`. |
| B5 popup quick add previews | applied | §4.8, Appendix B Add row, §4.11 row, W-013/W-014; handler and CLI stay immediate; Q19. |
| B6 Space everywhere, double-click is Enter | applied | §4.3.2 bullet; A8. |
| B7 dashboard closes like the TUI; `y` copy magnet | applied | Appendix B rows (scope note: `y` in a confirm prompt still confirms). |
| B8 one bar mark | applied via A16 | |
| B9 glance pauses the countdown | applied via A6 | |
| B10 "in your list" on search results | applied | A.9 `added: hash?`, P3-12 accept/tests, state table row; Q20. |
| B11 notification hygiene | applied | §3.8 rows (TTY/`--notify` rule, `default` action). |
| B12 first-party hint row and 380 width | applied | §4.2.3 row, §4.2.2, §4.5 heading, mockup hint rows. |
| B13 sidebar never shifts | applied | §4.6 sidebar bullet. |
| B14 remaining streaming decisions | applied | §6.2 rows 38–40; `open_command` in `Settings`; Q15. |
| B15 accent fallback by hue | applied | §5.5.1 accent row (channel difference < 32 **and** ratio < 1.3; plus the 4.5:1 background rule). |
| B16 live buffered | applied | §4.3.1 meta line. |
| C1 player | → Q15 | default `xdg-open`, `open_command` key. |
| C2 ready wording | → Q10 (reworded) | default: notify at threshold, row says "may stall". |
| C3 popup quick add | → Q19 | default: strip then Enter. |
| C4 Space | → Q18 | default: pauses. |
| C5 `U` and `=` | → Q14 | default: as listed. |
| C6 idle exit while glancing | → Q2 extended | default: hidden icon, 10 s stopped-recently, glance pause. |
| C7 Peers tab tier | → Q16 | default: T1. |
| C8 tiled or floating | → Q17 | both tiers designed; tiled is the daily case. |
| C9 "in your list" | → Q20 | default: wanted. |

No item was rejected outright. Decisions recorded as DL-30…DL-52; questions Q12…Q20; §0.8
summarises the round.

### Consequential edits not in any hunk

§4.11 gained rows for the quick-add offer, stopped-recently, may-stall, disk-low and disk-full;
A.4 `app.client {glance}`; P1-12 Do (glance pause); Appendix F `wait` row; §6.3 ready-to-watch
rows; M4 demo script (may-stall, disk-low); Appendix D (`stall_free`, glance, disk low, `seq`);
Appendix C popup hint rows in the caption style; the 900 px dashboard mockup rebuilt for the
tier's columns at one width; P1-11 `changes[]`; §9.1 round-2 IDs; §9.2 parallelism note; §9.4
level table, critical path (17 items, weight 35) and weighted paths; §9.5 P3/P5 rows; §9.2 P5
row; §9.6 and Appendix E (`scripts/dagcheck.py`, `scripts/mockwidth.py`, to be committed by
P0-06); P3-16 captures at 900/1280/1500/1600; header "Round 2"; §10.5.

### Script outputs (final state)

`dagcheck.py` (the architecture reviewer's script, section D, unchanged):

```
items in §9.3: 108 (active 107); rows in §9.4: 107
no cycle; 107 nodes, 19 levels
orphans (no consumer): ['P7-08']
critical path to M1: 16 items, weight 35 (S=1,M=2,L=4): M1 <- P2-05 <- P2-03 <- P2-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M2: 17 items, weight 37 (S=1,M=2,L=4): M2 <- P3-16 <- P3-11 <- P3-09 <- P3-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M3: 18 items, weight 44 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M4: 17 items, weight 40 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
sizes: Counter({'M': 45, 'S': 44, 'L': 11})
```

No `UNDEFINED`, `DISAGREE`, `NOT IN LEVEL TABLE` or `LEVEL differs` lines (the first run after the
edits reported 96 level differences and two missing items; the table was regenerated from the
script's own Kahn ordering).

`mockwidth.py` (new; East Asian W/F = 2, combining and ZW* = 0, Nerd Font PUA = 1, else 1; a
block's declared width comes from its heading; ragged box edges fail too), final run over §5 and
Appendix C:

```
L2225 [OK] '5.3 Module split': no declared width, max 111, box widths []
L2252 [OK] '5.4 One action table': no declared width, max 83, box widths []
L2371 [OK] 'Torrents at 80×24 (narrow):': declared 80, max 80, box widths [80]
L2387 [OK] 'Torrents at 120×30 (medium):': declared 120, max 119, box widths [116]
L2407 [OK] 'Details modal at 80×24 (Enter on the narrow layout):': declared 80, max 80, box widths [80]
L4730 [OK] "Popup, idle with a ready-to-watch row (owner's theme):": no declared width, max 54, box widths [54]
L4748 [OK] 'Popup, waiting for the VPN, with a confirm strip open:': no declared width, max 54, box widths [54]
L4766 [OK] 'Popup, daemon not running (with `showWhenStopped`):': no declared width, max 54, box widths [54]
L4778 [OK] 'Dashboard, Search view with the poster grid (empty query, `v` grid):': no declared width, max 113, box widths [113]
L4798 [OK] 'Dashboard, Search view (rows):': no declared width, max 113, box widths [113]
L4814 [OK] 'Dashboard at 900 px (chip-row tier, §4.3.3: SIZE, PROGRESS, ↓, ETA, PE': no declared width, max 90, box widths [90]
L4835 [OK] 'Dashboard, remove dialog (`ConfirmDialog`, `selectedIndex: 0`):': no declared width, max 90, box widths []
L4852 [OK] 'Dashboard, add sheet:': no declared width, max 67, box widths []
L4868 [OK] 'TUI Search at 80×24 (narrow), card hidden:': declared 80, max 80, box widths [80]
L4885 [OK] 'TUI Shows at 80×24 (narrow), one pane with breadcrumb:': declared 80, max 80, box widths [80]
15 blocks measured, 0 need fixing
```

The first run reported 6 blocks to fix: the four lines the review named (its replacement lines
measured 79, 79, 78 and 81), plus the Torrents 80×24 header (84), the Shows footer (82), two
details-modal rows (79) and five poster-grid lines (112 of 113). The script, so it is not lost
with the scratchpad (P0-06 commits it as `scripts/mockwidth.py`):

```python
#!/usr/bin/env python3
import re, sys, unicodedata
def cw(ch):
    o = ord(ch)
    if unicodedata.combining(ch) or o in (0x200B, 0x200C, 0x200D, 0x2060, 0xFE0F): return 0
    if 0xE000 <= o <= 0xF8FF or o >= 0xF0000: return 1          # Nerd Font private use: one cell
    return 2 if unicodedata.east_asian_width(ch) in ('W', 'F') else 1
def width(s): return sum(cw(c) for c in s)
text = open(sys.argv[1], encoding='utf-8').read().split('\n')
def section(a, b):
    s = next(i for i, l in enumerate(text) if re.match(a, l))
    return s, next(i for i, l in enumerate(text) if i > s and re.match(b, l))
bad = 0
for s, e in [section(r'^## 5\. ', r'^## 6\. '), section(r'^## Appendix C', r'^## Appendix D')]:
    i, heading = s, ''
    while i < e:
        l = text[i]
        if l.startswith(('**', '### ', '#### ')): heading = l.strip('*# ')
        if l.startswith('```'):
            j = i + 1
            while j < e and not text[j].startswith('```'): j += 1
            body = text[i + 1:j]
            m = re.search(r'(\d{2,3})\s*[×x]\s*\d{2}|at (\d{2,3}) (?:columns|text columns)', heading)
            declared = int(m.group(1) or m.group(2)) if m else None
            ws = [(width(x), x) for x in body]
            boxes = sorted({w for w, x in ws if x[:1] in '│┌└├╭╰'})
            over = [(w, x) for w, x in ws if declared and w > declared]
            ok = not over and len(boxes) <= 1; bad += not ok
            print(f"L{i+1} [{'OK' if ok else 'FIX'}] {heading[:70]!r}: declared {declared}, max {max((w for w,_ in ws), default=0)}, box widths {boxes}")
            for w, x in over: print(f"   OVERFLOW {w}: {x}")
            if len(boxes) > 1:
                for w, x in ws:
                    if x[:1] in '│┌└├╭╰' and w != boxes[-1]: print(f"   RAGGED {w} (expected {boxes[-1]}): {x}")
            i = j
        i += 1
print(f"{bad} blocks need fixing"); sys.exit(1 if bad else 0)
```

### Dashboard arithmetic as adopted (product A1/B1)

Chrome: sidebar 200 + details 360 + 2 × panelGap 14 + 3 × (2 × PaneFrame padding 8) = 636 with
both docked; 200 + 14 + 2 × 16 = 246 with the details as a sheet; 16 with chips. Fixed columns:
glyph 26, STATE 110, SIZE 76, PROGRESS 120, ↓ 80, ↑ 80, ETA 64, PEERS 56, RATIO 56, ADDED 80 =
748; without ADDED, RATIO, ↑ = 532. NAME floor 210. Tiers: ≥ 1600 all columns, NAME ≥ 216;
1400–1599 both docked, hide ADDED/RATIO/↑, NAME 232–431 (with ↑ kept it would be 152–351, below
the floor); 1100–1399 details as a sheet, same columns, NAME 322–621; 900–1099 chips, same
columns, NAME 352–551. Visibility is monotonic; the details pane undocks (390 px) before PEERS
(56 px) would be dropped.

### Validation loop (planning-workflow)

- Self-containment: P5-07 and P1-19 read alone name their files, API, acceptance, tests and
  dependencies.
- Dependency graph: recomputed by script after every edit; §9.3 and §9.4 agree on every edge.
- Justification sampled: DL-30 (geometry), DL-33 (Space), DL-36 (glance), DL-39 (`stall_free`),
  DL-43 (exit codes) each carry a "why" in the section they point to.
- Steady state: not reached; the diff against round 1 is large (about 390 net lines). Round 3
  should be a short external review of §3.2.4/§3.2.7 (lifetime rules), §4.3.3 (tiers) and the
  new items only, then the owner's answers to Q12–Q20.

### Still not grounded (carried in §10.4)

Unchanged from round 1, plus: the exact `QSG_RENDER_TIMING=1` output format for P3-09's 32 ms
gate (P3-17 verifies); whether `ManagedTorrent`'s `state_change_notify` fires on every transition
the structural frame needs (the fork unit test in P1-08 asserts the four that matter); whether a
container duration is cheaply readable for `stall_free` (P5-05 omits it otherwise).

## Round 3 (integration), 2026-10-18

Inputs: `reviews/round-3-consistency.md` (A1–A23 defects, B1–B13 cuts, C1–C4 improvements, D1–D5
owner questions, E verification: 127 hunks) and `reviews/round-3-validation.md` (A.1–A.9
self-containment, B.1–B.3 dependency graph, C justification, D.1–D.3 grounding, E owner questions:
76 hunks). Both read in full. Every hunk was applied by exact content (`r3/apply_review.py` parses
the review's diff blocks and refuses a hunk whose removed-plus-context text does not occur exactly
once; 112 of 127 and 45 of 76 went in verbatim; the rest were reconciled by hand, below, or
replaced by a regeneration). Then the systematic pass the validator asked for: every item in §9.3
rewritten to the standard of the new §9.3 preamble, the P6 and P7 tables converted to bullet
items, the §6.3 one-liner rows expanded, §9.4 regenerated from §9.3 by script, four items sampled
with fresh agents. Nothing under `src/`, `plugin/`, `vendor/`, `Cargo.*`; no git state command.

Result: PLAN.md 5,075 → 6,688 lines; 101 IDs (99 active items + P7-07 and P7-08 superseded) and
8 milestones (M0, M1a, M1–M6); §9.4 has 107 nodes over 21 levels, no cycle, no orphan; the
mockup-width script is clean. Outputs below.

### Reconciliation of the two reviews

Where the validator added detail and the consistency reviewer cut machinery, the rule was: keep
the precision, drop the thing that no longer exists.

| Conflict | Decision |
|---|---|
| V-A.4 specifies `tests/common/log.rs` and a host test `tests/log_contract.rs`; C-B1 cuts the Rust twin of the harness | Cut. P0-15's expanded text keeps every Python detail (step handle, slugs, offsets, `FBT_TEST_FORCE_FAIL`, summary exit code) and says there is no Rust twin: every process-level case lives in `daemon.py` or `native.py`. |
| V-A.4 names the `qs` shim `plugin/oliver.torrents/tests/qs-log.py`; C-A13 moves every plugin test out of the symlinked folder | `plugin/tests/qs-log.py`. |
| V-A.5 marks idempotent ops with an `Idem` column in A.4/A.5; C-A3 lists them once in A.1 | A.1 is the single source (it also carries the ops V-A.5 did not name: `force_start`, `transfer.*`, `rss.*`, `shows.*`); P1-09 points at A.1; V-A.5's `instance` request field (the mechanism) is kept in A.1. |
| V-A.7 and C-A12 give two `keymap.json` schemas (surface-keyed with `version` versus action-keyed per Appendix B row) | One schema in Appendix B: action-keyed (one element per Appendix B row, so `no_duplicate_key_per_scope`, the README table and the appendix are the same rows), with V's `version: 1`, scope vocabulary (`screen:…`, `global`, `pane:…`, `modal:…`, `prompt`) and key spelling, and a `note` for surface exceptions; `--surface` filters the same elements. |
| C-A10 makes P1-07 a skeleton that P1-03 depends on, with a Depends line that still names P1-06 in prose; C-C3 moves P1-03's dependency from P1-19 to P1-02 | Both applied. The Depends lines were reworded so that the DAG script does not read a parenthetical ID as an edge (C-A10's "half depends on P1-06" would have formed the cycle P1-07 → P1-06 → P1-03 → P1-07; the same for "until P1-10" in P1-03). |
| C-A16 and V-B.2 fix the critical-path prose in two different ways ("as the script prints it" versus "17 nodes, 15 items") | Neither hunk; §9.4's adjacency table, level table, critical path and weighted paths are now generated from §9.3 by `r3/regen94.py` (the dagcheck parser plus a writer) and state nodes, items and milestones separately. The §9.2 paragraph was rewritten from the new output. |
| C-A2 replaces the §4.2.1 dim row; V-D.1 corrects a line number in the same row | One replacement with the corrected citation (`PanelSectionHeader.qml:19`) and V-D.2's ratio (10.4:1, not 10.6:1). |
| C-C2 adds the milestone `M1a`; the round-2 reviewer's `dagcheck.py` only knows `M[0-6]` | Two regexes in the script widened to `M[0-6]a?` (the ID class and the milestone matcher); nothing else changed; the output format is identical. |
| C-A20 assigns `src/api/client.rs` to P1-11; P1-04 already needs `ensure_daemon()` in that file | P1-04 creates the file with `ensure_daemon()` (probing `engine.sock`); P1-11 fills in the NDJSON client and switches the probe. |
| V-B.1 wants P5-03 and others to depend on P4-03; C-C3 wants fewer serial dependencies | Both: the edges V found are real (a TUI that sends `torrents.recheck` needs the API client), and they do not touch the M1 path. The M1 path fell from 35 to 31 S-units through C-C3 alone. |

### Verdict per review item

Verdicts: **applied** (the hunk as written), **adapted** (the intent kept, text or placement
changed, reason given), **rejected** (not taken, reason given).

**Consistency review**

| Item | Verdict | Reason / where |
|---|---|---|
| A1 slot `Running` at two moments | applied | §3.2.7 diagram label and transition row now say "the librqbit session exists" (DL-49). |
| A2 three dim ladders | adapted | one ladder in §4.2.1 with Qt's HSV formula and the mix rule, ±1 tolerance; §5.5.1, §5.7 rows 3 and 16, P2-08 Do, DL-23 (superseded), DL-31 (corrected) as hunked; the citation is `:19` and the owner's ratio 10.4:1 (V-D.1, V-D.2). |
| A3 idempotent marking | applied | A.1 list; §3.2.4 step 6 and §3.2.11 point at it; P1-09 adapted to it (V-A.5). |
| A4 Esc's last layer | applied | §4.12. |
| A5 who starts the daemon | applied | §4.4 rule 10 and the `startDaemon` paragraph. |
| A6 `q`/`Q`, exception count | applied | Appendix B rows and paragraph (two exceptions: Enter, `w`), §2.2 principle 7, §4.12, §4.5 Quit, DL-33. |
| A7 `s providers` in mockups | applied | both Appendix C footers say `P providers` (same width). |
| A8 client timeout shorter than the server's | applied | §4.4 rule 6: 20 s / 100 s; idempotent resends only (A.1). |
| A9 `log`/`events` offline | applied | §3.2.3 split row: exit 4 with the `journalctl` hint. |
| A10 harness dependency inversion | adapted | P1-07 skeleton (depends on P0-15 only), P1-03 depends on P1-07, P1-03's tests in `daemon.py`, `ensure_daemon()` probes `engine.sock` until P1-11, `READY=1` tied to the daemon's socket; the §9.4 rows were regenerated, not hunked; Depends wording avoids parser edges. |
| A11 "receipt" undefined | applied | Appendix D row; §9.1 bullet; P0-01 names its section. |
| A12 UI contract file shapes | adapted | `states.json` and `expected.json` shapes as hunked; the `keymap.json` schema reconciled with V-A.7 (above). |
| A13 three test paths | applied | `plugin/tests/` everywhere (§4.1, §4.16, P2-05, Appendix E, P0-15's shim). |
| A14 four names for four limits | applied | §6.3 P5-04 row and `Settings` with `alt_limits_on`; "every key is the `config.toml` key". |
| A15 R15 test, `finishes_in_s` | applied | P5-05 tests; A.9 `Playable` formulas. |
| A16 drifted numbers | adapted | §0.5 "17 UX defects" applied; the critical-path sentence is now generated (see reconciliation). |
| A17 five-minute glance test | applied | P1-12 runs `glance_pauses` with `idle_exit_secs = 30`. |
| A18 two frame budgets | applied | 32 ms in P3-17 and the §6.3 poster-grid row. |
| A19 popup `r` key | applied | §4.2.4 refresh row. |
| A20 `src/api/client.rs` owner | adapted | P1-04 creates it (`ensure_daemon()`), P1-11 fills the client; P1-11's Do carries the hunk's client description. |
| A21 right click with the popup closed | applied | §4.5 clicks. |
| A22 `interval_ms` versus per-topic defaults | applied | A.6 `subscribe`. |
| A23 option B premise | applied | verified (`du`: `src/` 56 MB, 959 files; `examples/`, `assets/`, `tests/` under 100 KB); §8.2, P0-02, DL-46, Q3. |
| B1 Rust twin of the harness | applied | §3.3 tree, §9.1, P0-15 (merged with V-A.4), P1-03 tests, Appendix E, §9.6. |
| B2 legacy-binary tests | applied | P1-10 rewritten (the TUI of P1-15 is the adapter's consumer); §3.6; P1-11 tests; Appendix E; §9.5 P1. |
| B3 1 px comparison | applied | §0.2 Look, §4.16, P2-08 accept and tests: the judged pair image. |
| B4 `free-chord.sh` | applied | P2-07 Do and tests; Appendix E row removed. |
| B5 CI jobs for plan lints and doc links | applied | §9.6, Appendix E rows, P0-01 tests. |
| B6 `check-fixtures.sh` | applied | P1-05 tests; Appendix E; §8.4 `lint` job. |
| B7 `proto_minor` | applied | §3.4.4, §3.2.12, both examples, A.2, P1-05. |
| B8 code tested against Appendix A | applied | P1-18 (`table_matches_ipc_md`); Appendix A note. |
| B9 self-test modes | applied | P1-07, P4-10. |
| B10 P7-08 and §7.4 | applied | superseded row; §7.4 two-line note; §2.6 row; §9.4 regenerated without it. |
| B11 duplicate `Summary` fields, `stats.json` counters | applied | A.9 `Summary`; §3.4.3; P1-14. |
| B12 captures under `docs/` | applied | P2-05, §4.16: `target/test-logs/visual/`. |
| B13 rules stated twice | applied | §3.4.6 reduced to one paragraph; §5.7 row 12 → pointer; A1 prose → §3.2.10. |
| C1 Q3 blocking | applied | P0-02 Depends; R18. |
| C2 milestone M1a | applied | §9.3 milestone, §9.2 row, §9.7 table and demo script, §0.6 extended; §9.4 regenerated (the hunk's `M1a` rows superseded by the generator). |
| C3 P1-03 ∥ P1-19 | applied | P1-03 depends on P1-02 and P1-07; P1-10 on P1-03 and P1-19; the M1 path is 31 S-units. |
| C4 prose | applied | §0.1 point 4, §9.2 P3 row, M2 title. |
| D1–D5 owner questions | → Q3 (already blocking), Q21 (popup `q`), Q22 (M1a checkpoint), Q23 (judged pair), Q24 (TUI dim) | defaults as the plan now implements. |
| E verification script | used | the applier is the same idea; 127/127 hunks matched the current text. |

**Validation review**

| Item | Verdict | Reason / where |
|---|---|---|
| A.1 P7-03 row | applied | now a bullet item (P7) with the same content; §6.3 row expanded. |
| A.2 P7-06 row and §6.3 row | applied | bullet item; the §6.3 row as hunked. |
| A.3 P0-08 | applied | the grounded steps, log file, `with_test_signal_serialization`, exit status, `flock -n`, P0-15 dependency. |
| A.4 P0-15 | adapted | every Python detail applied; the Rust helper and `tests/log_contract.rs` dropped (C-B1); shim at `plugin/tests/qs-log.py` (C-A13); `--case`/`--list` added for `native.py`. |
| A.5 P1-09 and A.1 `instance` | adapted | the item as hunked except that the idempotent set is A.1's list (C-A3), not an `Idem` column; §3.4.8 and A.1 hunks applied. |
| A.6 P3-13, W2, P4-07, §4.12, unread row | applied | all five hunks; P3-13 depends on P3-06 and P3-11 (Q26 asks the owner to confirm staging). |
| A.7 P4-02, §5.3 note, keymap schema | adapted | the item and the tree note as hunked; the schema paragraph reconciled with C-A12 (one action-keyed schema). |
| A.8 P6-08 row and §6.3 row | applied | bullet item with the full rules (keep-files only, per-batch undo, `engine/undo/`, the `u`/`U` fix); §6.3 row as hunked. |
| A.9 cross-cutting P6/P7 | applied | the tables became bullet items with Do/Why/Accept/Tests/Depends; every §6.3 one-liner row carries numbers, caps, formats and the test of record. |
| B.1 missing edges (14 items) | applied | every edge added in §9.3 (P2-08↔P4-05 as a one-way edge P4-05 → P2-08 plus a prose note; P5-03 +P4-03, P1-16; P5-04 +P4-03; P6-05 +P4-03; P3-05 +P1-08; P3-06 +P1-12; P3-11 +P3-05; P3-12, P3-14 +P3-06; P3-18 +P3-07; P5-05, P5-07 +P2-04; P6-08 +P4-08; P7-06 +P3-10, P1-11; P0-07, P0-08, P0-09, P0-11, P0-14 +P0-15); P2-01, P3-09 and P2-07 text fixes applied. |
| B.2 critical-path prose | adapted | regenerated from the new graph (nodes, items and milestones counted separately; P1-07 is M). |
| B.3 §9.4 rows and level table | adapted | not hunked: §9.4 is generated from §9.3 (`r3/regen94.py`), so §9.3 and §9.4 agree by construction. |
| C justification (6 hunks) | applied | constraint row, backoff numbers, mojibake safety, Pieces and Log rationale, private flag, contract 1.2.0 minor bump. |
| D.1 citations (6 hunks) | applied | `src/main.rs:550-560`, `Border.qml:217,141,17`, `Bar.qml:78,84-91`, `PanelSectionHeader.qml:19`, `PluginShellApi.qml:30-32`, the D2 row's ftui lines. |
| D.2 numeric (3 hunks) | applied | 10.4:1; the accent-fallback rule names `kanagawa` and excludes the owner's theme; P4-05 accept corrected. |
| D.3 platform APIs | no change | all found. |
| E1–E5 owner questions | → Q26 (Shows stages), Q25 (grey accent), Q27 (rename revert), Q28 (bulk keep-files), Q29 (undo per batch) | defaults as the plan now implements. |

No item was rejected outright. Edits beyond both reviews, made during the self-containment pass
and recorded here: P6-12 depends on P0-05 and P1-07 (not P0-06: the fork test needs the engine
test build and the daemon harness, not CI); P6-09 depends on P4-03 (the TUI's F6 port field sends
`app.settings.set`); P1-02 creates `scripts/lint-sizes.sh` (P4-01 extends it); new scripts named
and placed in Appendix E (`fixtures.py`, `verify-determinism.sh`, `gen-keymap.sh`, `qs-log.py`,
`fbtlog_test.py`); `contracts/ui/format.json` as the shared number-format fixture (P4-07,
`qs-format.sh`); `src/daemon/log.rs`, `src/api/mutations.rs` and `src/api/ops/mod.rs` added to the
§3.3 layout; `state()`'s JSON shape defined (§4.17); `Case slug` and `Fixture state` in Appendix D;
`undo [<hash>]` in Appendix F.

### Self-containment check (the planning-workflow validation loop, run again after the rewrite)

Method: `r3val/extract_items.py` (the validator's extractor) wrote every §9.3 item to a file; the
pool was every active item not among the validator's eight (P7-03, P7-06, P0-08, P0-15, P1-09,
P3-13, P4-02, P6-08); `random.Random(20261018 + 3).sample(sorted(pool), 4)` picked **P0-02, P3-10,
P5-03, P6-01** (seed 20261021). Each went alone to a fresh `general-purpose` subagent (model opus)
with read-only access to the repository, no other context, and this prompt, verbatim:

> You are a fresh implementation agent who has never seen this project. Below, between the
> markers, is one work item from a plan, pasted alone. Is this implementable as written? Give a
> verdict (implementable as written / mostly implementable after reading the sections the item
> cites / not implementable) and then list every fact you would have to guess or look up to
> implement it, saying where you found it if you found it. You have read-only access to the
> repository at /home/oliver/Projects/odcpw/fbittorrent (you may read any file, including
> docs/plan-2026-10-18/PLAN.md and the sources under src/, vendor/, plugin/). Hard rules: do not
> create, modify or delete any file; do not run git stash, checkout, reset, commit, clean or any
> other command that changes state; do not run cargo build/test or any build. Deliver your full
> answer through SubagentHandback.
>
> ===== ITEM BEGIN ===== (the item's text) ===== ITEM END =====

**Result of batch 1: 0 of 4 implementable as written; 4 of 4 "mostly implementable after reading
the sections the item cites".** No item was judged not implementable. Every answer named facts
that were in the plan but not in the item (acceptable: the §9.3 preamble says items anchor into
canonical sections) and facts that were nowhere (fixed below). The answers, verbatim:

#### P0-02 (fresh agent)

> **Verdict: mostly implementable after reading the sections the item cites, but it can't be run today and two of its steps fail if followed literally.**
>
> - It is blocked on the owner's answer to Q3, and Q3 has no answer anywhere in the plan.
> - The mechanical path is clear: import the crates.io tree, commit the vendored tree on top, switch `Cargo.toml`, build, test, archive.
> - The item's description of the patch is wrong. Reproducing what it describes would not compile.
> - The `--locked` build step fails without an unstated lockfile update.
> - `run_timer_wheel_tests.py` fails with its default arguments, and it breaks for good once `vendor/` is archived.
>
> The `diff -r` acceptance check saves the core step: copying `vendor/asupersync` mechanically gives the right result even though the description is inaccurate.
>
> Everything below is read-only. I changed nothing and ran no build. The only network calls were `git ls-remote` and `gh repo view`.
>
> **A. Owner, permissions, licence**
> 1. **Q3 is unanswered.** §10.3 only gives a default ("the fork"). Its preamble says "none blocks work", while P0-02 says it is blocked.
> 2. **Who creates the repository is unclear.**
>    - Q3 (PLAN.md:5468) says "you create the repository", meaning the owner. The P0-02 *Do* reads as if the agent creates it, and `reviews/round-3-consistency.md:1335-1338` (C1) says it needs the owner's account.
>    - I checked: `gh` is logged in as `odcpw` with `repo` scope, and `github.com/odcpw/asupersync` does not exist yet. Creating a public repository is publishing, so it needs explicit permission.
> 3. **Public or private is unsettled.** Q3 says public; P0-02 doesn't say. `odcpw/fbittorrent` itself is **PRIVATE**. A private fork would need cargo git authentication (`net.git-fetch-with-cli`) and a CI token, and the plan covers neither.
> 4. **The licence is not considered anywhere.**
>    - `vendor/asupersync/LICENSE` is "MIT License (with OpenAI/Anthropic Rider)"; `Cargo.toml` has `license = "LicenseRef-MIT-OpenAI-Anthropic-Rider"`.
>    - The rider grants no rights to OpenAI, Anthropic or anyone acting on their behalf. It forbids distributing, hosting or making derivative works available to them, and it defines "use" broadly (modifying, executing, testing, analysing). It also requires every distribution to carry the rider unmodified.
>    - §8.2 and Q3 weigh only repository size against a second repository. P0-03 checks nova3's licences, but nothing checks this one.
>    - This bears on a public fork, on the upstream offer, and possibly on which agents may do the work (the branch is `codex/...`). It's the owner's decision; I'm not drawing a legal conclusion.
>
> **B. What the fork contains**
> 5. **Base tree.** `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/asupersync-0.5.0` exists. It differs from the vendored tree only by `.cargo-ok`, a registry marker the item doesn't say to exclude, plus the patch. The original archive is `~/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f/asupersync-0.5.0.crate` (sha256 `f34b1a19ffd6b74570339a156912436335bb09c594c675c62ea164b19a1f2511`).
> 6. **"Fork" has to mean a new repository built from the crates.io package.** A GitHub fork of `https://github.com/Dicklesworthstone/asupersync` at `78b64636…` has a different tree and would fail the `diff -r` check. (Upstream URL from `vendor/asupersync/Cargo.toml:119`; rev from `.cargo_vcs_info.json`.) Three things are unspecified:
>    - the default branch;
>    - what sits on it besides `fbittorrent-0.5.0`;
>    - whether to tag the pinned commit so the rev survives a branch rewrite.
> 7. **The patch description is wrong.** `diff -r` against the registry copy shows 10 files and about 1,308 added and 11 removed lines.
>    - Modified files: `Cargo.toml` +2, `src/cx/cx.rs` +63, `src/runtime/builder.rs` +26, `src/runtime/epoch_tracker.rs` +113/-10, `src/runtime/mod.rs` +3, `src/runtime/state.rs` +11/-1, `src/time/wheel.rs` +192.
>    - New files: `src/cx/region_owner_tests.rs` (52 lines), `src/runtime/epoch_diagnostics.rs` (196), `src/time/wheel_metamorphic_tests.rs` (626).
>    - "About 690 lines" holds only without `wheel_metamorphic_tests.rs`. That file is unchanged upstream code the crates.io package left out (`docs/validation/2026-10-11/cpu-optimization/restored-upstream-test.json`).
>    - **Two changes are described nowhere:**
>      - `RegionOwnerKey` and `Cx::reusable_region_owner_key()` in `cx.rs`, with their tests. The build needs them: `vendor/rqbit/librqbit/src/spawn_utils.rs`, `peer_connection.rs` and `read_buf.rs` call them. A fork with only "the wheel fix and epoch-diagnostics" will not compile.
>      - An unconditional behaviour change outside the feature. `state.rs` builds the tracker with `EpochConsistencyTracker::with_independent_module_sequences`, which turns off cross-module skew and order checks, and the violations store becomes a `VecDeque`.
>    - `FBITTORRENT_PATCH.md` is stale: it says "The only implementation change is in `src/time/wheel.rs`".
>    - So the commit message, the patch note and the scope of the upstream offer need the real list.
> 8. **The patch note's fate is unspecified.** Should the fork carry `FBITTORRENT_PATCH.md` updated, or not at all? "Empty apart from the patch note" suggests it differs or is absent. It also points into `docs/validation/2026-10-11/cpu-optimization/`, which P0-01 archives.
>
> **C. Cargo**
> 9. **Toolchain.** `cargo` on PATH is a mise shim with no version set. Use `~/.rustup/toolchains/1.98.0-x86_64-unknown-linux-gnu/bin/cargo` (from `all.sh:6` and the project memory). `rust-toolchain.toml` only arrives in P0-05.
> 10. **Missing step: update the lock.** After switching the `[patch]` source, `--locked` fails until `Cargo.lock` is updated (`cargo update -p asupersync`, or one build without `--locked`). The asupersync entry at `Cargo.lock:218` has no `source` line today. Afterwards it will read `git+https://github.com/odcpw/asupersync?rev=<sha>#<sha>`.
> 11. **URL form.** https or the SSH alias `git@github-odcpw:` that `origin` uses? Full 40-character rev? Probably https if the fork is public.
> 12. **Comment text.** The new wording for the comment at `Cargo.toml:50-51` is unspecified. The cited lines 49-52 are correct.
> 13. **README sentence.** §8.2 says the README states that `--offline` works after one online build. It's unclear whether P0-02 or C3 (P0-04) writes it.
>
> **D. Tests**
> 14. **`run_timer_wheel_tests.py` fails with defaults.**
>     - The default `--support-lock`, `target/cpu-optimization/test-support/Cargo.lock`, doesn't exist (no `target/cpu-optimization/`).
>     - The only copy is `docs/validation/2026-10-11/cpu-optimization/timer-wheel-tests/support-Cargo.lock`. No markdown links to it, so P0-01 will archive it.
>     - The script also needs a Rust 1.98 `target/release-perf` cache (6 asupersync fingerprints exist today; the item only builds `--release`) and proptest 1.12.0 offline (present).
>     - The previous green run passed 63 tests (`receipt.json` in that directory). The item states no expected count.
> 15. **The runner is tied to the vendored tree.**
>     - `timer_wheel_tests.rs:65` hard-codes `#[path = "../../../vendor/asupersync/src/time/wheel.rs"]`; `run_timer_wheel_tests.py:55-57` and `:112-113` do the same.
>     - So it tests the vendored file, not the fork, and breaks once the tree is archived.
>     - `select_core` accepts only a registry or vendored source path. An rlib built from a git checkout never matches, so in a clean clone it can never pass.
>     - The item doesn't say whether to point it at the fork's checkout (for example via `cargo metadata`) or retire it. As things stand, C5 would commit it broken.
> 16. **JSONL logs.** `fbtlog.py` (P0-15) depends on P0-04, so P0-02 has to write the lines by hand, using the field set in §9.1 and E.1 (suite `asupersync`, cases `build` and `test`, a `final` line). Two things are unspecified:
>     - which commands count as steps (the lock update? the git fetch?);
>     - whether the worker-panic scan in `all.sh:12-16`, which can fail an otherwise green run, applies.
> 17. **Untested patch code.** `cargo test -p fbittorrent -p rqbit-runtime` doesn't run Asupersync's own tests (`region_owner_tests`, the epoch tests). Without `--features epoch-diagnostics`, the feature code isn't even compiled. Whether to add that build is unspecified.
>
> **E. Archive and receipt**
> 18. **Archiving details.**
>     - `~/Archive` doesn't exist yet (P0-01 creates it, and P0-02 doesn't depend on P0-01).
>     - The tar working directory and member path, a verification step, and recording the archive's SHA256 are all unspecified. P0-01 spells these out for its own archives.
> 19. **Order against P0-01 is undefined.** If P0-02 runs first, the tree is gone before the §8.1 step-1 backup. If P0-01 runs first, the support lock from item 14 is archived.
> 20. **Running the `diff -r` check.** It means extracting the archive and checking out the fork commit, excluding `.git` and `.cargo-ok`. I checked that the tree has no empty directories, symlinks, files ignored by git, or files over 1 MB, so a plain `git add -A` captures all of it.
> 21. **Receipt location.** Appendix D puts it at `## P0-02` in `docs/validation/<date>/README.md`, presumably `2026-10-18`. The item only says "a link in the receipt".
>
> **F. Upstream offer**
> 22. **Form and scope.**
>     - Unspecified: issue or pull request, and the scope (the wheel fix only, or the region-owner and epoch-domain changes too).
>     - Upstream `main` is now `eb89e457…`, not `78b64636`, so a pull request needs a rebase.
>     - The vendored README states no contribution policy.
>     - Posting there is public content, so it needs the owner's permission, and the licence rider in item 4 applies.
>
> **G. Option B fallback**
> 23. **The fallback has no landing slot.** The commit for the full tree has no name or slot in the §8.1 C1-C6 table, and C3's text assumes the git patch. Option B has no Accept or Tests, and the patch note stays stale. Sizes: 57 MB for the whole tree, 56 MB for `src/`.

Fixed in P0-02 (every numbered point): the owner creates the public repository and answers the
licence question (Q3 extended; R19 added); the real patch inventory (10 files, about 1,308 lines,
`RegionOwnerKey`, the `state.rs` change, the restored metamorphic tests) and the rewrite of
`FBITTORRENT_PATCH.md`; `.cargo-ok` excluded; branch, tag and https URL; the rustup toolchain
path; `cargo update -p asupersync` before `--locked`; the `Cargo.toml` comment; the README
sentence; the runner retargeted through `cargo metadata` with its support lock copied to
`regression_tests/fixtures/timer-wheel/` and the 63-test expectation; hand-written JSONL in the
E.1 shape; `cargo test --all-features` in the fork; archive commands, verification and receipt
section; the upstream offer as an issue, wheel fix only, with the owner's permission; option B's
landing slot C0 in §8.1. I verified the claims that mattered: the licence header and
`license =` line, the runner's default paths and hard-coded vendored paths, `Cargo.lock:218`
without a `source` line, `spawn_utils.rs` calling `reusable_region_owner_key()`.

#### P3-10 (fresh agent)

> **Verdict: mostly implementable after reading the sections the item cites, but not as written.** The QML work is well specified once you read §4.7, §4.3.3, §3.4.7 and Appendix A/B. Three things block a clean build:
> - the plan contradicts itself on when the pane becomes a sheet;
> - one Overview field has no API source;
> - the "settings store" the width is persisted in does not exist.
>
> On the test side, two acceptance criteria cannot be checked as written, and the named fixture cannot actually detect natural sort.
>
> None of the dependencies exist yet. The repo has no `src/api/`, `plugin/tests/` or `contracts/`, and `plugin/oliver.torrents/` holds only today's four files. That is expected for a P3 item.
>
> **Facts found by reading (resolved)**
> 1. Overview fields come from A.9 `Torrent` (PLAN.md:5741), `TorrentDetail` (:5742) and `ImdbCard` (:5760). File rows use `File {index, path, size, progress, included, sort_key, ready?}` (:5744).
> 2. Ops: `torrents.set_files {hash, include?, skip?}` (:5629), `torrents.open_target {hash, file?}` (:5632), `imdb.lookup {name|id}` (:5675), `torrents.set_seed {hashes, on}` (A.5).
> 3. Topics: `torrent:<h>.files` is keyed by `index`, default 1,000 ms (:5685). `log` is keyed by `seq` and append-only (:5692).
>    - The files snapshot arrives in index order (:1182), so the client must sort.
>    - `subscribe` takes no filter args, so the hash filter on `log` is client-side. `app.log {hash}` exists as an alternative (A.4).
> 4. Detail-topic lifetime: §3.4.7 (:1179) and §4.4 rule 9 (:1808).
>    - `unsubscribe` works per `sub` (:5677), so the files topic needs its own `subscribe` call.
>    - The limit is 8 subscriptions per connection (:1247).
> 5. Keys: Appendix B rows at :5853 (`u`), :5865 (`i`/`w`), :5867 (`[`/`]`), :5869 (Space/`a`/`n`). Layered Esc is in §4.12.
> 6. The seed-switch meaning strings are in §4.7 (:1997).
> 7. Shell components, checked in `/usr/share/omarchy/shell`:
>    - `Style.space` exists (`Commons/Style.qml:221`).
>    - `Ui/ToggleSwitch.qml` exists. Its caller owns `checked` and it has `busy`, so bind it to the confirmed `seed` value (no optimistic redraw, §4.10).
>    - `Ui/SizedImage.qml` exists.
>    - There is no Checkbox component, so the ☑/☐ rows are text or custom.
> 8. Poster loading uses `asynchronous` and `sourceSize`, local files only (§4.15, :2172). The 300 ms lookup debounce and per-name cache are stated only for the TUI (§5.5.5, :2414), not for the plugin.
> 9. The plan never names a copy mechanism. First-party code pipes to `wl-copy` (`plugins/panels/network/Panel.qml:495`, `panels/tailscale/Service.qml:110`), and E.3 has a `wl-copy` shim. The lint forbids `Process`/`Timer` outside `Service.qml` (E.2 `lint.sh`, P2-03), so copy and debounce must live there.
> 10. The harness is `qs-dashboard.sh` (P3-08), using `daemon.py --serve-fixture <state>` and the `useRuntimeDir`/`summonView` hooks (§4.16–4.17).
>
> **Contradictions and gaps (must be guessed or decided)**
> 1. **Sheet threshold is contradictory.**
>    - "Below 1,100 px": P3-10 (:4199), §4.6 (:1987), M2 demo (:5345).
>    - "Sheet at 1100–1399", i.e. below 1400: §4.3.3 table (:1719), P3-09 Do and Accept (:4161, :4176, "undocks … at 1399").
>    - The arithmetic settles it. A docked pane needs a window of at least 636 + 532 + 210 = 1378 px to keep NAME ≥ 210, so 1400 is right and 1,100 is wrong (1,100 is where the sidebar collapses).
> 2. **"Uploaded" has no field.** Neither `Torrent` nor `TorrentDetail` has uploaded bytes. A.10 gives `ratio = uploaded / progress_bytes`, so it could be approximated as ratio × `completed_bytes`, but only when ratio is present. The options are a new daemon field (outside Do), a derived value, or dropping it. "Downloaded" presumably means `completed_bytes`.
> 3. **The role of `TorrentDetail` is unclear.** No listed Overview item needs a `TorrentDetail`-only field, and `torrents.get` is a one-shot request, not live. Which extras to show and when to refetch is a guess.
> 4. **The "plugin's settings store" does not exist.**
>    - Neither the panel nor the service gets `settings` injected (shell.qml:1396–1406 panel loader, :945–950 service loader). Only bar widgets get them (`plugins/bar/Bar.qml:2719`).
>    - The only write path is `shell.updateEntryInline(id, settings)` (`PluginShellApi.qml:62`). It replaces the whole bar-layout entry in `~/.config/omarchy/shell.json` with what is passed (shell.qml:1100–1140), so the caller must merge keys the dashboard cannot read. Every write also rewrites the user's shell.json and fires a barConfig change.
>    - To decide: route reads through the bar widget into the Service with a debounced write, or use a plugin-owned state file; and whether the key goes in the manifest's defaults/schema.
>    - P3-09's sort persistence (:1701) has the same gap.
> 5. **Resize bounds are missing.** No min/max width is given, and the tier math assumes 360 px. Dragging to 400 px at a 1400 px window gives NAME 192 px, below the 210 floor. The sheet's width is also unspecified.
> 6. **Docked-pane behaviour is unspecified.**
>    - Does the pane follow the cursor, or show nothing until Enter?
>    - What counts as "hides" when docked: Esc, a view switch, or the window hiding?
>    - Should detail resubscription be debounced while `j`/`k` moves fast?
>    - Key routing: Space, `a`, `n` and `o` mean different things in the list and the Files tab. Pane focus (`h`/`l`) presumably decides, but it is not stated whether Enter moves focus into the pane or whether `[`/`]` work while the list has focus.
> 7. **Unbuilt tabs.** Peers, Trackers and Pieces arrive in P5/P6, but the mockups show all six tabs. Hidden, disabled or gated on `caps`?
> 8. **`File.sort_key` has no owner and no algorithm.**
>    - §6.3 (:2643) assigns "natural-sort keys" in `torrents.files` to P3-10/P4-07, but P3-10's Do is QML only. P1-05 only types the field.
>    - The key's format, and whether a plain string compare is valid on it, are undefined.
> 9. **The fixture cannot test natural sort, and its format is undefined.**
>    - Zero-padded E01..E10 already sort correctly as plain strings, so the test only proves "not index order". It needs unpadded names (E1..E10).
>    - The `--serve-fixture` recipe has no schema (P1-07, :3451–3456: "torrents to add and the manipulations to apply").
>    - `fixtures.py` has no N-file torrent builder (only `two_file_fixture` and `mkv_fixture`), so P3-10 must extend both.
> 10. **"Daemon subscription count" cannot be observed.** No API exposes it: `app.info` has only per-kind connection counters (A.4), and P1-08's own `detail_topic_released` doesn't say how it checks. `state().subs` is client-side only. Checking it needs a daemon change, such as an `app.info` field or a log line.
> 11. **The `state()` fields for the pane are undefined.** P3-08, P3-09 and P3-13 each define theirs; P3-10 does not (§4.17, :2226). The tests need at least the tab, hash, width, docked/sheet mode, file order and poster status.
> 12. **The drag cannot be driven by the harness.** It only has `wtype` and `hyprctl`, with no pointer drag and no hook to set the width.
> 13. **"Across reloads" is undefined.** Hot reload, `omarchy-restart-shell` or `rescanPlugins`? §4.15 notes the window closes on reload.
> 14. **"Include/skip persists" across what?** Pane reopen, a re-read of the files topic, or a daemon restart?
> 15. **The poster criterion has no test case.**
>     - None of the four named cases covers it.
>     - P3-04 gives no base-URL override for the IMDb suggestion endpoint or CDN; only EZTV has one (`FBT_EZTV_BASE_URL`).
>     - The blurb comes from en.wikipedia.org (`src/imdb.rs:239,244`), which also needs an override.
>     - No fixture state is named for it.
> 16. **No acceptance criteria or tests** cover the Log tab, the Overview fields, `[`/`]`, or sheet mode.
> 17. **The "Ready to watch" line and the per-file ready dot have no owner.** P3-10 defers them to "after P5-05", but P5-05's Do lists only the sidebar filter and row badge for the dashboard. Rendering them only when `playable` is present now seems safest.
> 18. **Copy and format choices are undefined.**
>     - No user-facing text for each `reason` value (§3.5).
>     - No timestamp format for added/completed (§4.2.3 covers sizes, rates, ETA, percent and counts only).
>     - No meaning text for `upload_policy: custom`.
>     - No empty states for Files before metadata, or for a null card.
>     - Unclear whether non-video torrents (an ISO) should be looked up at all.
> 19. **Copy/open targets differ.** §4.7 says save path is copied and content path is opened; the item gives both "copy/open". It is also unclear whether opening uses `xdg-open` (§4.4) or `Settings.open_command` (A.9).
> 20. **Service and Store work is unlisted.** The §4.4 action list has no `setFiles`, IMDb lookup, copy, URL open, or detail subscribe/unsubscribe. P2-02's `Store.js` reduces only the keyed `torrents` topic plus three unkeyed ones, so the files and log topics need reducer support. The Do names only the two QML files.
> 21. **Files-tab edge cases.**
>     - Filter matching rule (substring? case?), and whether `/` focuses it or the top-bar filter.
>     - Whether `a`/`n` act only on the filtered subset.
>     - Whether `n` (skip all) is allowed. The engine accepts an empty set (`vendor/rqbit/librqbit/src/torrent_state/mod.rs:656–684`), but rejects any change while a torrent is checking (:672).
> 22. **Log tab versus P3-15's Log view.** P3-15's Log view takes "a hash filter from the details pane". Is the tab its own list or that view pre-filtered, and is the `log` subscription per-pane or shared?

Fixed: the sheet threshold is 1,400 px everywhere (§4.6, P3-10, the M2 demo; §4.3.3 was right);
`Torrent.uploaded_bytes` added (A.9, P1-06); `TorrentDetail`'s role (one `torrents.get` on open,
refetched on `engine.changed`); the plugin-owned state file `~/.local/state/fbittorrent/dashboard.json`
written through `Quickshell.Io.FileView` (§4.6; P3-09's sort uses it); resize bounds 300–480 px
and the NAME floor; docked-pane behaviour (cursor follow with a 150 ms debounce, key routing by
pane focus, `[`/`]` from either); unbuilt tabs gated on `caps`; `File.sort_key`'s algorithm and
owner (A.9, P1-06); the fixture is `E1`..`E10` from a new `fixtures.multi_file_fixture`;
`app.info.subscriptions` (A.4, P1-08) makes the released topic observable; `state().view.details`
fields; the `setPaneWidth(px)` hook (§4.17); "reloads" and "persists" defined in Accept; the
poster case and `FBT_IMDB_BASE_URL` (P3-04); cases for the Log tab, `[`/`]` and the sheet; the
ready line only when `playable` is present; the reason phrases and `Model.when`/`Model.ago`
(§4.3.4); copy/open targets and `Settings.open_command`; the service actions and store reducers
named; the Files-tab filter rules; the Log tab as P3-15's component over one shared subscription.

#### P5-03 (fresh agent)

> ## Verdict: mostly implementable once you read the sections it cites
>
> The item is a dense set of pointers. Most of the shapes it points to do exist: the op name and args, the `{state:"pending"}` reply, the `recheck` cap, the CLI exit list, the `R` key, the prompt text, how the dialog defaults, and the fixture and test names. But a fresh implementer still has to decide several behaviours the plan never states, and some cited material contradicts other parts of the plan. I read only; I created, changed and ran nothing.
>
> Plan file: /home/oliver/Projects/odcpw/fbittorrent/docs/plan-2026-10-18/PLAN.md (line numbers below refer to it).
>
> ### A. Facts I looked up and found
> 1. **§6.3 "Recheck" row** (L2640). It adds almost nothing beyond the item: the API is "pending until the check ends", and the test is "see progress below 100%, then a re-download from the local seed restores it".
> 2. **Op shape** (A.5, L5633): `torrents.recheck {hashes: [hash]}` → `{state: "pending"}`, mutating, since P5-03. A.5 does **not** mention `mutation.result`, unlike `search.install` and `shows.check`. The general rule is in §3.4.8 (L1201-1205) and A.8 (L5723).
> 3. **Not idempotent** (A.1, L5557). Clients never resend it. P1-09 (L3515, L3521) says recheck has no timeout and always ends `done` or `failed`. A stale `instance` is refused with `invalid_state`.
> 4. **`hashes` bound 1..256** comes from P1-09 (L3522-3524), not from this item.
> 5. **Cap row** in §3.4.3 (L1084).
> 6. **Busy reason**: a pending recheck must count as `mutation` busy (§3.2.10, L886). The item doesn't say this.
> 7. **CLI**:
>    - Appendix F row (L6221): starts the daemon, JSON shape "new", exits 0/4/6/7.
>    - Start rule: §3.2.3 (L693).
>    - Selector grammar (prefix of 4+ hex, unique name substring, or `all`): `/home/oliver/Projects/odcpw/fbittorrent/src/agent.rs:179` and `src/main.rs:1164-1180`.
>    - The existing bulk-action helper `act()` and its JSON (`{ok, action, count, hashes}`) are at `src/main.rs:717-729`.
> 8. **Keymap**: Appendix B row (L5859) gives `R` for both TUI and dashboard and n/a for the popup. "Cancel a prompt: Esc, Enter, any other key" is at L5880-5881. `R` is free on the TUI Torrents screen; today it is only bound on RSS and Shows (`src/tui/mod.rs:1489,1566`). Footer priority 40 is in P4-02 (L4393).
> 9. **TUI prompt mechanism**: `PromptKind` plus `open_prompt(kind, label, yes_no)` in `src/tui/mod.rs:60-63,1255-1280,1343-1348`. The current convention is `short(name, 50)`. P0-13 (L3166) makes Enter cancel and sets the footer to `(y) <Verb>  (Esc) Cancel`.
> 10. **Dashboard dialog**:
>     - `ConfirmDialog` is at `/usr/share/omarchy/shell/Ui/ConfirmDialog.qml`. I verified `selectedIndex: 1` is the default, plus the `confirmText`/`cancelText` properties and the `confirmed`/`canceled` signals. Index 1 is always styled urgent.
>     - The `y`→`confirmed()` mapping and the hover caveat are in §4.9 (L2047-2068).
>     - The pending verb is "checking…" (§4.3.4, L1768).
>     - `Service.qml` already gets a `recheck` action in P2 (L1827, L3775).
> 11. **Engine path**:
>     - `src/engine.rs:756-760` is the `torrents/start` error-retry branch that calls `self.inner.rebuild(...)`. The function itself is at `src/engine.rs:262-277`: it caches metadata, then `session.delete(...,false)`, then `restore_record(changed, paused=true)`.
>     - Each re-add really is a full check: the fork's `check()` comment says it "verifies every piece from disk", at `vendor/rqbit/librqbit/src/torrent_state/initializing.rs:113-114`.
>     - To learn when the check has ended, `ManagedTorrent::wait_until_initialized` exists (`vendor/rqbit/librqbit/src/torrent_state/mod.rs:602`). The plan doesn't name it.
> 12. **Dormant torrents** (P1-16, L3689): `torrents.recheck` on a dormant record triggers the real restore, which checks first. Missing files give `error/missing_files`.
> 13. **Fixture**: `corrupt_byte(path, offset)` is already listed in P1-07's `fixtures.py` (L3444). P1-07 also supplies the 256 KiB magnet fixture and the seed/leech pair (L3454-3457, as a follow-up commit). Neither `fixtures.py` nor `daemon.py` exists yet, and today `native.py` has only a 33-byte, single-piece `.torrent` with no tracker or seed.
> 14. **Where code goes after the restructure** (§3.3, L966-1004; P1-02/P1-19): `EngineOp` in `src/engine/ops.rs`, the handler in `src/api/ops/torrents.rs`, and an `api::ops::TABLE` row. The line numbers cited in the item will be stale after P1-02.
>
> ### B. Facts I would have to guess (not stated anywhere)
> 1. **What `mutation.result` carries for a recheck.** The `result` object is undefined (for example per-hash `have/needed` bytes, or `{rechecked, failed}`).
> 2. **Several hashes in one call.** Is there one `mutation.result` for the batch or one per hash? What is the `outcome` if one fails? §3.4.8's pending reply has a singular `hash` (L1202), and A.5 shows only `{state}`.
> 3. **Visible state during the check.** `rebuild` restores with `paused=true`, which gives `Initializing{paused:true}`. §3.5 maps only `Initializing{paused:false}` to `checking` (L1276). Today `Initializing{paused:true}` maps to queued/stopped (`src/engine.rs:1416-1430`).
>    - So a recheck would show `queued`, `paused` or `done` unless `reconcile_queue` happens to unpause it.
>    - Unpausing an initializing torrent calls `start()` → `close_run()`, which cancels the run region and may cancel and restart the check (`mod.rs:340-490`).
>    - The item doesn't say whether recheck should restore unpaused, or whether the mapping should change.
> 4. **Queue interaction.** While checking, a `done` torrent counts as not finished (`finished=false` during Initializing, `mod.rs:560-562`), so it takes a download slot. If the queue is full it shows `queued/queue_full`. Should a recheck be subject to `max_active_downloads`?
> 5. **Paused torrents.** Does a recheck of a torrent with intent `pause` keep it paused, so it never "recovers"? It is unspecified; the acceptance only works for `paused=false`.
> 6. **`completion_on` after a failed check.** If the check finds missing data, nothing says to reset it to 0.
>    - If it isn't reset, P1-13's completion event (fired only when `completion_on` is first set) never fires after the repair.
>    - Worse, P1-16 restores any `completion_on != 0 && ratio_limit == 0` record as dormant `done, checked:false`. A `down`, crash or reboot mid-repair would then bring back a corrupt torrent as "done". This is a real correctness hole.
> 7. **Dormant double check.** On a dormant torrent, is the P1-16 restore's own check the recheck, or does a second rebuild check follow?
> 8. **Fast-resume clause.** "fast-resume data is invalidated, P6-06" refers to work that doesn't exist yet. P6-06 (L4821) itself takes on "recheck deletes the resume file first". Should P5-03 do nothing here? I assume so. The §9.4 row for P6-06 (L5173) doesn't list P5-03 as a dependency even though P6-06 changes recheck.
> 9. **Refusals.** Nothing says how recheck answers on a staged torrent (P3-06), on one without metadata (`rebuild` needs metadata, `engine.rs:264`), on one already `checking`, or while the engine is parked. `engine_unavailable`/`invalid_state` are plausible guesses.
> 10. **Ledger lock and dispatch.** Today `rebuild` runs under the ledger mutex, and `torrents/info` takes the same lock (`engine.rs:624-628`). The pending wait must happen outside the lock and off the 8-thread dispatch pool (§3.4.10). This isn't stated.
> 11. **`rebuild` error wording.** Its messages say "changing upload cap…" (`engine.rs:272-274`) and need generalising.
> 12. **CLI behaviour.**
>     - Does `recheck` wait for `mutation.result` or exit 0 at `pending`? Is there a `--wait`?
>     - Its "new" JSON shape isn't defined.
>     - The exit list 0/4/6/7 contradicts §3.2.4 step 6 (L725: recheck exits 8 when the request is lost mid-flight). It also leaves out 1 (stale `instance` → `invalid_state`; `internal`) and 2 (more than 256 hashes from `all`; a bad flag).
>     - What happens when `all` expands past 256: chunk the calls, or error?
> 13. **Contract bookkeeping** that other parts of the plan require but the item doesn't mention: the `api::ops::TABLE` row, the `docs/IPC.md` changelog entry (§3.4.4), `capabilities`/`robot-docs` `commands[]` (P1-11's `commands_match_clap`), any `contract_version`/`changes[]` bump, regenerating `keymap.json` and the README keys block (P4-02 tests).
> 14. **Dashboard details.**
>     - The Overview button's label and placement: neither P3-10/§4.7 nor any mockup has an action row.
>     - The dialog's confirm-button text (the default is "Confirm"), and whether the dashboard message keeps the "y/N" suffix.
>     - Whether `R` also works inside the details pane.
>     - Hiding the action when the `recheck` cap is missing (§3.4.3 says so generally).
>     - How long "checking…" lasts, since the service's 20 s pending timeout covers only the reply.
> 15. **Tests.**
>     - No dashboard test is listed (no `qs-dashboard.sh` case for cancel-first or the Overview button), although T1 must land in both UIs.
>     - The TUI test has no name or file; §5.3 says `src/tui/tests/*.rs`.
>     - What `recheck_exit_codes` exercises for 4, 6 and 7 must be inferred.
>     - `recheck_corrupt_byte` can race. During Initializing, `progress_bytes = checked_bytes` (`mod.rs:562`), so progress is below 1 even with clean data. With a live local seed, a 256 KiB payload re-downloads almost at once. The test must wait for the check to end and probably hold the seed offline during it. The fixture's piece size isn't stated either.
> 16. **Size.** "S" (under ~300 lines) looks low for daemon, CLI, TUI, dashboard and three test suites; it is closer to M.
>
> ### Bottom line
> You can build the API, cap, CLI verb, TUI prompt and dashboard dialog from the cited sections plus the code. The behaviour of an actual recheck is not settled by the plan: the state shown during the check, what happens to queue slots and pause intent, resetting `completion_on`, the result payload, and whether the CLI waits. B3 and B6 are the most serious: without explicit decisions there, the item can pass its acceptance tests and still show wrong states, or later restore a corrupt torrent as done.

Fixed in P5-03 (now M): the `mutation.result` payload (one per call: `rechecked`, `failed`,
`have_bytes`); the `rechecking` set that maps any `Initializing` to `checking` (§3.5 row
updated); no download slot taken; pause intent kept; `completion_on` and `completed_at` reset
when the check finds missing data (the correctness hole the agent found: P1-16 could otherwise
restore a corrupt torrent as dormant `done`); one check for a dormant torrent; resume files left
to P6-06, which now depends on P5-03; refusals (staged, no metadata, already checking, parked);
the wait off the ledger lock and the dispatch pool; the `rebuild` wording; CLI `--wait`,
`--timeout`, the JSON shape, exit codes 1, 2 and 8, chunking of `all`; the contract bookkeeping
moved into the §9.3 preamble as a rule for every item; the Overview action row (§4.7), dialog
texts, `R` in the pane, cap gating; dashboard and TUI test cases named; the seed held offline in
`recheck_corrupt_byte`; the 16 KiB piece size.

#### P6-01 (fresh agent)

> **Verdict: mostly implementable after reading the sections the item cites.** It is at the weak end of that grade. The API half is fully pinned down by the sections it cites. The fork half names two methods but gives no design and no fork file paths. The UI half has no layout, no status words and no keymap entry. The TUI half rests on files that items outside its Depends list create. Several behaviours are left open: torrents that are not live, `tier`, private torrents, row order, passkeys and idempotency. Nothing was modified; I only read files.
>
> ## A. Facts the item cites that I found and checked
> - **Operations** (PLAN.md A.5, lines 5635-5636): `torrents.trackers {hash}` returns `{trackers:[Tracker]}` and is not mutating. `torrents.reannounce {hashes}` returns `{state:"done"}` and is mutating.
> - **`Tracker` object** (A.9, line 5746): `url`, `tier: u64?`, `status: enum(ok|error|updating|not_contacted)`, `last_announce_at: ts?`, `next_announce_at: ts?`, `peers: u64?`, `message: str?`. `TorrentDetail.tracker_count: u64?` is at line 5742.
> - **Topic** (A.7, line 5687): `torrent:<h>.trackers` is keyed by `url`, default 5,000 ms.
>   - Detail-topic rules are in §3.4.7: it exists only while details are visible and its sampling is released when the last subscriber leaves.
>   - An unknown hash on subscribe is `not_found` (A.7).
> - **Cap** (§3.4.3, line 1087): `trackers` covers both operations and the topic.
> - **CLI** (Appendix F): `reannounce <sel>…` starts the daemon, its JSON shape is "new", exit codes 0, 4, 6, 7.
> - **Dashboard tab columns** (§4.7, line 2000): URL, status, last announce, peers returned, error; `e` reannounces. The mockups at lines 1957 and 6004 show only the tab label, never the tab's contents.
> - **Files that earlier items create:**
>   - `src/api/ops/torrents.rs`: P1-06.
>   - `fixtures.py` and `daemon.py` with its `ApiConn` helper: P1-07 and P1-06. P1-07 already gives `fixtures.py` a local HTTP tracker for the magnet fixture.
>   - `agent_contract.py`: P1-11.
>   - `plugin/tests/qs-dashboard.sh`: P3-08.
>   - `views/DetailsPane.qml`: P3-10.
>   - TUI goldens: P0-16.
>   - All of these are upstream of P6-01's dependencies.
> - **Fork facts from the code:**
>   - **Tracker list:** `ManagedTorrentShared.trackers: HashSet<Url>` (`vendor/rqbit/librqbit/src/torrent_state/mod.rs:186`).
>   - **Where announcing happens:** `vendor/rqbit/librqbit-tracker-comms/src/tracker_comms.rs`, one loop per tracker. This file is not cited by the item.
>   - **Retry timing:** an HTTP error retries after 10 s, doubling up to 600 s (lines 257-265). A UDP error falls back to 60 s (lines 385, 400).
>   - **Failure message:** a bencoded failure reason becomes `"tracker returned failure. Failure reason: …"` (line 314).
>   - **Unsupported URLs:** schemes other than http, https and udp are dropped (line 163).
>   - **Where comms are created:** `Session::make_peer_rx` (`session.rs:1378-1440`). Private torrents keep only their first tracker (lines 1408-1413). It is called at torrent start (`torrent_state/mod.rs:475`).
>   - **Shared state:** none exists today. Comms live inside the torrent's run region, so a pause discards them.
>
> ## B. Facts I had to look up because the item does not give them
> 1. **Fork design.** No paths are given (P5-01, by contrast, cites its fork lines). The implementer must decide:
>    - where per-tracker state lives so it survives a pause, for example an `Arc` on `ManagedTorrent` passed into `TrackerComms::start`;
>    - how `announce_now` wakes both the success sleep and the backoff sleep;
>    - the fields of `TrackerStat` and whether times use `SystemTime` or `Instant`;
>    - the signature of `announce_now(url?)`.
>    The API never passes a URL, so the `url?` parameter has no caller.
> 2. **Where the fork unit test goes.** It could live in `librqbit-tracker-comms` or `librqbit`. CI's `engine-libs` job (§8.4, P0-05) runs only `-p librqbit-core -p librqbit --lib`. A test placed in tracker-comms would not run in CI unless the command changes.
> 3. **Missing dependencies.**
>    - The TUI details modal `src/tui/modals/details.rs` is created by P4-04 (P4-01 says so), and P4-04 is not upstream of P6-01.
>    - The action table `src/tui/actions.rs` comes from P4-02, also not upstream.
>    - That table's tests `table_covers_appendix_b` and `keymap_json_matches` need a new key row.
>    - Appendix B has no reannounce row, and `keymap.json` and the README keys block are not mentioned either.
>    - The scope of `e` is unstated: the whole modal or only the Trackers tab.
> 4. **Tiers are lost.** `iter_announce().unique()` flattens the announce list (`session.rs:1021-1031`). Re-adding a magnet from cached bytes writes every tracker into one tier (`session.rs:505-518`; `src/engine.rs:229-236`). So `tier` is always absent unless the daemon re-reads the `.torrent` file.
> 5. **The test fixture is underspecified.**
>    - N is not given.
>    - "Error" could mean HTTP 500 or a bencoded failure reason; the two produce different `message` text.
>    - The announce interval it returns is not given.
>    - Whether it returns the local seed as a peer is not given. Appendix E.3 lists the seed/leech pair for P6-01, but the item doesn't.
>    - Because of the 10 s, 20 s… backoff, error-then-ok takes at least 10 s unless the test uses `force_tracker_interval` or reannounces.
>    - E.3 claims the tracker fixture "exists in `native.py`". It doesn't: that fixture has no `announce` key. Only `engine_benchmark.py` has a tracker.
>
> ## C. Facts not in the plan or code, which I would have to guess
> 1. **Torrents that are not live.** What `torrents.trackers` and `reannounce` return for torrents that are paused, done-and-stopped, dormant (P1-16; no `ManagedTorrent` exists), in `metadata` (the engine resolves it inside `add_torrent`, so no handle exists) or `waiting_vpn`. The exit codes exclude 1, which suggests no `invalid_state`, but a silent no-op is unconfirmed.
> 2. **Status edge cases.**
>    - The status after a pause tears down the comms: keep the last values or reset to `not_contacted`.
>    - The status for URLs dropped by the scheme filter or by private truncation, which would otherwise stay `not_contacted` forever.
>    - Whether `tracker_count` counts all URLs or only the ones actually announced to.
> 3. **Row order.** The trackers array and the keyed snapshot have no documented order (`HashSet`), but the determinism rule of Appendix F and §3.4.7's "natural order" require one.
> 4. **Message content.**
>    - The `message` text, its length cap, and whether an `ok` row carries a tracker's warning message.
>    - **Passkey leak:** the HTTP error text includes the full announce URL with its query string (`rqbit-runtime/src/http.rs:139`). Private-tracker passkeys would leak into the API, both UIs and the log. Showing or redacting `url` is not decided, and §3.9 is silent.
> 5. **What a reannounce does.** Whether it respects the tracker's minimum interval or throttles repeated `e` presses (a private-tracker ban risk). Whether it also re-queries DHT. Which `event` it sends. Whether `done` means "triggered" or "announced".
> 6. **Idempotency.** The A.1 list is declared "the source" but lists `torrents.reannounce` in neither its idempotent nor its non-idempotent group. `api::ops::TABLE` needs the flag.
> 7. **Contract files.** No `contracts/ipc/v1/trackers.ndjson` is named, though §3.4.11 wants one per group and P5-01 names `peers.ndjson`. No `docs/IPC.md` changelog entry is named either.
> 8. **CLI output.** The `reannounce --json` shape is unspecified. The existing `act()` helper's shape `{ok, action, count, hashes}` (`src/main.rs:718-728`) is the likely model. Whether `show --json` gains `tracker_count` is also unstated.
> 9. **UI rendering in both UIs.**
>    - The words, glyphs and colour roles for the four statuses (§5.5 says state is never shown by colour alone).
>    - The time format for last announce: `text.rs::ago` exists in the TUI module plan; there is nothing for QML.
>    - Column widths and URL elision, and whether next announce or tier is shown.
>    - Whether the dashboard subscribes per tab (P5-02's rule) or per pane.
>    - Whether the tab is hidden when the `trackers` cap is missing.
> 10. **Tests.**
>     - The name of the fixture state or golden file for the TUI golden at 120×30. At that width the modal renders at 100×30 (P4-04).
>     - What `qs-dashboard.sh::trackers_tab` and `daemon.py::tracker_status` assert, and whether they poll or subscribe.
>     - Whether a status change is a "structural" frame sent at once or waits for the 5 s coalesce.
>
> Main files read: `docs/plan-2026-10-18/PLAN.md` (§3.4, §4.7, §5.6, §6.2-6.3, §9.1, §9.4, P1-xx/P3-10/P4-0x/P5-0x/P7-04, Appendices A, B, E, F), `vendor/rqbit/librqbit-tracker-comms/src/tracker_comms.rs`, `vendor/rqbit/librqbit/src/session.rs`, `vendor/rqbit/librqbit/src/torrent_state/mod.rs`, `vendor/rqbit/rqbit-runtime/src/http.rs`, `src/engine.rs`, `src/main.rs`, `agent_ergonomics_audit/audit/regression_tests/{native.py,engine_benchmark.py}`.

Fixed in P6-01 (every point; the fork citations were verified against the vendored source
first): the fork design with file paths and lines (`TrackerStats` on `ManagedTorrentShared`, the
announce loops, the failure-reason text, `announce_now()` without a URL, the 60 s rate limit,
`event=none`, DHT untouched); the fork test in `librqbit` (inside CI's command); P4-02 and P4-04
added to Depends; the Appendix B "Reannounce" row; `tier` absent (flattened by the fork); the
fixture (`fail_first=2`, bencoded failure reason, `interval 5`, the local seed as a peer, an
announce log; the test uses `reannounce` to skip the backoff); behaviour for torrents that are
not live (stats if recorded, else `not_contacted`; `reannounce` → `invalid_state`, exit 1);
status after a pause kept; dropped URLs labelled; `tracker_count` counts every URL; rows sorted
by URL; `message` capped at 200 and URLs redacted of their query strings before they reach a
message or a log line (the passkey leak); `torrents.reannounce` idempotent in A.1 (with
`set_download_limit`, `add_tracker`, `remove_tracker`); `contracts/ipc/v1/trackers.ndjson` and
the changelog through the preamble rule; the CLI shape and `show --json` additions; status words
and glyphs as rows of `contracts/ui/states.json`, column widths, `Model.ago`/`text.rs::ago`,
per-tab subscription, cap gating; the golden's name and fixture state; what each test asserts;
status changes are structural frames. E.3's claim that the tracker fixture "exists in
`native.py`" was corrected: `fixtures.tracker_server()` is new in P6-01.

Two cross-cutting rules were added to the §9.3 preamble because three of the four answers asked
for the same things: contract bookkeeping (the `api::ops::TABLE` row, the `docs/IPC.md`
changelog, the `caps` entry, the fixture file, `commands[]`, the `act()` JSON shape, the keymap
regeneration) and defaults for edge states (a mutation on a torrent that is not live is
`invalid_state`; the engine's absence is `engine_unavailable`; a read answers what the ledger
knows).

#### Batch 2

Because 0 of 4 passed "as written", a second batch of four was drawn after the batch-1 fixes and
the two preamble rules were in: `random.Random(20261018 + 4).sample(sorted(pool), 4)` over the
pool minus the twelve items already sampled and minus the two superseded rows (seed 20261022):
**P1-11, P2-06, P3-07, P6-05**. Same prompt, same conditions.

**Result of batch 2: 0 of 4 implementable as written; 1 of 4 "mostly implementable after reading
the sections the item cites" (P6-05); 3 of 4 "not implementable as written" (P1-11, P2-06,
P3-07).** The three failures were not vagueness but contradictions and gaps the rewrite had
inherited: P1-11 depended on items that are not upstream of it and its "unchanged" JSON shapes
collided with the new `state` field; P2-06's scenario could not pass against the daemon its
dependencies deliver (a magnet row did not exist before metadata) and its acceptance read a field
round 3 had removed; P3-07's "one shared shape, byte-identical" was a design decision the plan
had never made. All three are now decided in the plan (below). The answers, verbatim:

#### P6-05 (fresh agent)

> ## Verdict: mostly implementable, once you've read the sections it cites
>
> The plumbing is fully specified and checks out against the repo. The item gives the engine hook, the API operations and their shapes, the Torrent fields, the cap, the key letters, the CLI verbs and the test names, and all of them match their canonical sections. What it leaves open is the queue semantics. An implementer would have to decide about ten behaviours that change how the feature works. The most important are: whether reordering pauses a running torrent, whether forced torrents use up a slot, what turns `force` off, and how `queue_move` behaves on a torrent that isn't in the queue. Nothing blocks a start. Two different implementers would produce noticeably different behaviour.
>
> ## Facts I looked up and found (where)
>
> 1. **Engine hook.** `reconcile_queue` is at `src/engine.rs:280-342`; the doc comment is at 278 and the cited 280-343 is accurate. It walks `ledger.records` in order and admits while `slots < limit`. It skips records with `blocked || record.paused` or without a handle. Errors are retried only via `retry_errors`. It pauses first and resumes second.
> 2. **Ledger order.** It is FIFO today: the only insert is `ledger.records.push` at `src/engine.rs:949`. `torrents/info` sorts a copy by `added_on` (around line 625), and so does the parked path (line 1297), but neither saves the sorted order.
> 3. **`Record` struct.** It is at `src/engine.rs:37-53`. The serde-default pattern already exists (`resume_after_migration`). After P1-02 it lives in `src/engine/ledger.rs` (§3.3). The item doesn't say this.
> 4. **Other files the item doesn't name.**
>    - `TorrentView` in `src/engine/view.rs` (P1-19)
>    - `EngineOp` in `src/engine/ops.rs`
>    - handlers in `src/api/ops/torrents.rs` (§3.3, P1-06)
>    - the CLI in `src/main.rs`
>    - `capabilities.commands[]` in `src/agent.rs` (P1-11)
> 5. **API shapes.**
>    - A.5 has `torrents.force_start {hashes, on}` → `{state:"done"}`, `torrents.queue_move {hash, to}` → `{state:"done", queue_pos: u64}`, and `torrents.start {force: bool?}`, whose result includes `skipped`.
>    - A.9 has `queue_pos: u64?` and `force: bool` (not optional).
>    - A.10 says where each field comes from.
>    - §3.4.3 has the cap row `queue`.
> 6. **Idempotency.** It is only in A.1, not in the item: `force_start` is idempotent and `queue_move` is not.
> 7. **`queued` / `queue_full`.** These come from P1-19's §3.5 mapping (handle paused, record not paused, not finished, no error). §4.3.4 has the wording: "waiting for a download slot" and the secondary text "3 of 3 slots busy".
> 8. **Keys.** The Appendix B rows exist: `P` is force start; `K`/`J` are up/down; the dashboard also gets Ctrl+k/Ctrl+j; the popup gets none. `K`, `J` and `P` are unbound on the TUI Torrents screen today (only lowercase `j`/`k` are used, `src/tui/mod.rs:1321-1322`). On Search, `P` means providers (Appendix B).
> 9. **`PanelKeyCatcher`.** It is at `/usr/share/omarchy/shell/Ui/PanelKeyCatcher.qml`. It has a `reorderable` property (line 39) and a single signal, `reorderRequested(int dy)` (line 42), with dy -1 for up and +1 for down. When `reorderable` is set, Ctrl+Up/Down also fire it (lines 64-70), not only Ctrl+k/j. Shift+K and Shift+J reach the view through `textKey`. The item's "Ctrl+k/Ctrl+j `reorderable` signals" is loose wording: the real API is one property plus one signal.
> 10. **CLI.**
>     - Appendix F has `start <sel>… [--force]` (exits 0, 1, 4, 6, 7) and `queue <sel> top|up|down|bottom` ("new" shape, exits 0, 4, 6, 7).
>     - The JSON shape falls back to `act()` `{ok, action, count, hashes}` (`src/main.rs:718-728`, per the §9.3 preamble).
>     - Selector rules are in `resolve_in` (`src/main.rs:1170-1196`), which accepts `all` and several selectors.
> 11. **Bookkeeping (§9.3 preamble).** Each new operation needs an `api::ops::TABLE` row, a `docs/IPC.md` changelog entry, a `contracts/ipc/v1/<group>.ndjson` fixture, and a run of `scripts/gen-keymap.sh`.
> 12. **Settings and test harness.**
>     - `max_active_downloads` comes from `src/config.rs:15` (default 3).
>     - A real-session engine test harness exists at `src/engine.rs:1515-1570` (no trackers, local listener), with queue tests nearby (around line 1700). `queue::tests` can reuse it.
>     - `daemon.py` and `agent_contract.py` don't exist yet. P1-07 and P1-11 create them.
> 13. **Dependencies.** P1-02, P1-19, P1-08 and P1-11 are not listed but are reached through the listed ones (§9.4 table).
>
> ## Facts I would have to guess (not specified anywhere)
>
> 1. **Does reordering pause a running torrent?** The current `reconcile_queue` admits strictly in ledger order. So `queue_move {to: top}` on a queued torrent while the slots are full would pause a running one immediately. The acceptance line "the next slot goes to it" could mean that, or it could mean "when a slot frees". Not specified.
> 2. **Do forced torrents use up a slot?** "Skips the slot limit" doesn't say whether a forced record does `slots += 1`. This decides whether forcing an earlier record pushes a later one out. It also changes the "N of M slots busy" text and the `Summary.counts` meaning.
> 3. **What turns `force` off?** Candidates are `torrents.stop`, completion, `torrents.start` without `force`, and `force_start {on:false}`. Also unstated: does `on:false` put the torrent back in the queue or stop it?
> 4. **Force on a paused or other non-running torrent.** Should `force_start {on:true}` on a user-paused record also set `paused=false`? The §9.3 default rule ("a mutation on a torrent that is not live → `invalid_state` unless the item says otherwise") would refuse it. Also unstated: force on a completed torrent with seeding off (does it seed?), on an `error` torrent (retry?), on a staged torrent, and while the engine is parked or blocked.
> 5. **`torrents.start {force:true}` on an already-running torrent.** Does it set `force` but report `skipped: already_running`?
> 6. **`queue_pos` details.**
>    - Is it 0-based or 1-based? ("index" suggests 0; qBittorrent uses 1.)
>    - What counts as "incomplete": `stats.finished` (which only looks at selected files), `completion_on == 0`, or progress < 1?
>    - Do records in `error`, `metadata`, without a handle, or `waiting_vpn` get a position?
> 7. **What `up`/`down` move past.** Should they swap with the nearest record that is in the queue, skipping paused, done and forced records in the ledger? Or move one ledger slot, which may show no visible change? Similarly for `top`/`bottom`.
> 8. **`queue_move` on a torrent with no `queue_pos`** (forced, paused, done, staged). The result requires `queue_pos: u64`, so the call must fail or move silently. But the CLI exit list for `queue` has no 1 (`invalid_state`). Contradiction.
> 9. **`queue_move` with a stale `instance`.** A.1 says it's not idempotent, but P1-09 only lists remove, recheck and move as refused. So the rule for `queue_move` is missing.
> 10. **`queue_move` while parked.** Is it `engine_unavailable`, or a ledger-only reorder? The parked path already lets start and stop write the ledger offline (`src/engine.rs:1341-1348`).
> 11. **CLI `queue`.**
>     - With several matches or `all`: loop, refuse with exit 7, or something else? The verb takes one `<sel>`, but `queue_move` takes one hash.
>     - Should the JSON output include `queue_pos`?
>     - What is the `action` string? Probably `"queue"`.
>     - `start --force` presumably uses `action: "start"`.
> 12. **How the user sees the result in either UI.** No column, sort key, glyph or `states.json` row shows `queue_pos` or `force`. TUI and dashboard lists are sorted by added date (Appendix B sort keys and the Appendix F determinism rule have no "queue" key). So `K`/`J` would make no visible change. Unknowns:
>     - Is a forced marker wanted, like qBittorrent's "[F]"?
>     - Is `P` a toggle? The prior art in `omatorrent-research.md:248` is a "Forced: ON" toggle.
>     - Does `P` work on a multi-selection (`hashes`), and what do `K`/`J` do with one?
>     - Does the cursor follow the moved row?
>     - What feedback message appears?
>     - Should `P`/`K`/`J` be hidden when `caps` lacks `queue`? Implied by §3.4.3, not stated.
> 13. **Dashboard keymap drift.** Setting `reorderable` also binds Ctrl+Up/Down, which Appendix B and `keymap.json` don't list. That may break `table_covers_appendix_b` or let the keymap drift.
> 14. **Tests.**
>     - No dashboard test is named for `P`/`K`/`J`/Ctrl+k (e.g. a `qs-dashboard.sh` case), although the Do section requires those keys.
>     - The TUI `K`/`J` test has no name and no assertion target. Should it check the request sent, or the store, against what seam after P4-03?
>     - `daemon.py::queue_move_order` has no steps.
>     - "Two fixtures" is undefined. The standard magnet fixture is 256 KiB and finishes almost at once, so "both run" needs the throttled seed/leech pair or larger payloads. Appendix E.3 doesn't list P6-05 for the throttled pair.
>     - "Both run" needs a concrete assertion, such as both states being downloading/stalled/checking with intent `run`.
>     - The fixture filename is unknown (`contracts/ipc/v1/queue.ndjson` or `torrents.ndjson`).
> 15. **Unit test seam.** Should `queue::tests::admission_with_force` drive the real-session harness, or should admission be pulled out into a pure function over (records, stats)? Not stated.
> 16. **Records that move back in.** Where does a restored or undone record (P6-08) or a deferred add (P7-09) go in the queue order: the end, or its old position? Neighbouring items don't say. Any future code that sorts and saves `ledger.records` would also silently destroy the queue order. Nothing in the item says to guard against that.

Fixed in P6-05 (every point): reordering never pauses a running torrent; forced records do not
count toward the limit; what turns `force` off and where the record returns; force on paused,
done, error, staged and parked torrents; `start {force: true}` on a running torrent; `queue_pos`
1-based, "incomplete" as `completion_on == 0`, which records are positioned; `up`/`down` skip
unpositioned records; `queue_move` without a position is `invalid_state` (exit 1 added in Appendix
F); the stale-`instance` refusal generalised in P1-09 to every non-idempotent op; `queue_move`
while parked reorders the ledger offline; the `queue` verb takes exactly one torrent and its JSON;
the visible result (`· forced` and `⚡`, `#pos`, the sort key "Queue", the cursor follows, a toast,
cap gating); Ctrl+↑/↓ added to Appendix B with the catcher's real API (`reorderable` plus
`reorderRequested(dy)`); the pure `queue::admit` seam; named dashboard and TUI tests; the
three-fixture `queue_move_order` steps with the throttled pair (E.3 updated);
`contracts/ipc/v1/queue.ndjson`; restored, undone and deferred records go to the end and
`ledger::tests::save_preserves_order` guards the order.

#### P3-07 (fresh agent)

> **Verdict: not implementable as written.** The routing half is mostly implementable once you read the sections it cites. You call `hello`, use the API if it answers, and otherwise run in-process. Which verbs never start the daemon and which do comes from §3.2.3 and Appendix F, the ops from A.6, and the client from P1-11. The central requirement is a design decision the plan never makes: one shared JSON shape, byte-identical on both paths, with Appendix F's "unchanged" output. The cited TUI lines are also the wrong place to make the TUI change. Nothing was modified; I only read files.
>
> **What I could look up (resolved)**
> - **Which verbs and the start rule.**
>   - Appendix F rows (PLAN.md 6395-6450) and the §3.2.3 table (687-704).
>   - `search`, `imdb`, `rss list/add/remove/refresh/items` and `shows list/follow/unfollow/check/episodes/seen` never start the daemon and fall back to in-process.
>   - `rss grab` and `shows grab` start it (`ensure_daemon`), so they have no fallback.
>   - `shows quality` belongs to P3-18 (line 4476), not P3-07.
> - **Client and handshake.**
>   - P1-11 (3644-3680) defines `src/api/client.rs`: `hello` with `kind: "cli"` and a 200 ms connect timeout (§3.2.4).
>   - The cap names are `search`, `rss`, `shows` and `imdb` (§3.4.3).
>   - The ops are in A.6 (5845-5877) and the objects in A.9 (5937-5960).
>   - None of `src/api/`, `daemon.py`, `agent_contract.py`, `docs/IPC.md` or `contracts/` exists yet; the dependencies create them.
> - **Today's code.**
>   - `src/main.rs`: 340-366 (`imdb`), 643-712 (rss, shows and search dispatch), 963-1020 (`rss`), 1022-1150 (`shows_cmd`), 1199-1206 (`tui_free_dedupe`).
>   - P3-01's tests are named `search::tests::…`, so the dedupe and mojibake repair live in `src/search.rs` and both paths can share them.
> - **The TUI's presence `hello`** has `kind: "tui"` and `interactive: true` (P1-15, 3746-3758). The daemon refreshes feeds and checks shows on that hello (P3-02, P3-03).
> - **Fixtures and determinism.**
>   - Fixtures: the fake providers `alpha`/`beta` (P3-01), a local feed (P3-02), EZTV with `FBT_EZTV_BASE_URL` (P3-03), IMDb with `FBT_IMDB_BASE_URL` (P3-04).
>   - Determinism rules are at the end of Appendix F.
>
> **What I would have to guess, or what conflicts**
>
> 1. **The shared shape (the main blocker).** Appendix F says these commands' output is "unchanged" (search: "plus `providers[]`"), and contract 1.2.0 only allows additive changes (line 179). The A.9 objects cannot rebuild today's output:
>    - **`rss list`** today prints the full `qbt::RssFeed` (`src/qbt.rs:212-221`): `uid`, `last_build_date`, `has_error`, `is_loading` and every article. `Feed` has no articles, no `uid`, no `last_build_date`, and an `error` string instead of `has_error`.
>    - **`rss items`**: `RssArticle` (`qbt.rs:162-176`) has `description`, a `date` string, `torrentURL` and `isRead`. `FeedItem` has none of the first three.
>    - **`shows list`**: `Show::to_json` (`src/shows.rs:175-183`) has `added` and counts of `episodes`, `seen` and `grabbed`. The API's `Show` has none of these.
>    - **`shows episodes`**: `episode_json` (`shows.rs:184-197`) has `season`, `episode`, `state`, `season_pack`, `magnet`, `candidates`, `released` and a `grabbed` bool. `Release` has none of them, and a `transfer` enum instead.
>    - **`shows check`**: today it reports `fresh` per show, plus `checked` and `failed`, and exits 5 when every show fails (`main.rs:1067-1093`). The API gives only `mutation.result {new}` and `shows.checked {imdb?, new, warnings}`.
>    - **`shows follow`**: today it reports `added`, `kind`, `fetched` and an episode count. The API returns `{show: Show}`, and it is not specified whether follow fetches straight away.
>    - **`search`**: today's results use qBittorrent names (`fileName`, `fileUrl`, `siteUrl`, `descrLink`, `engineName`, `nbSeeders`, `pubDate`, `imdb`). `SearchResult` has no `siteUrl`, nothing says which provider becomes `engineName` when there are several, nothing says what `fileUrl` maps to, and an unknown value means an absent field instead of a number.
>    - **`search --plugins`**: today's output has `fullName`; `Provider` does not.
>    - **`search --install`**: today it polls and prints a count and the plugin names; the API returns pending, then `{installed, failed}`.
>    - **`imdb`**: today it serializes `imdb::Title` (`src/imdb.rs:17-28`: `name`, `poster_url`, empty strings for missing values). `ImdbCard` uses `title`, has no `poster_url`, and leaves missing fields absent.
>
>    So you must choose between extending the API objects (which reopens the P3-01 to P3-04 contracts and `docs/IPC.md`), changing the CLI shapes (which breaks "unchanged" and needs a `changes[]` entry like DL-22), or a lossy mapping (which cannot be byte-identical). No section decides this.
> 2. **Byte-identical or same shape?** The Do says "byte-identical"; the Accept says "shapes identical (golden comparison)". Search result order is not deterministic today: `HashMap::into_values` followed by a sort on seeders leaves ties unordered (`main.rs:700-701`, `1199-1206`). Appendix F's list of fixed array orders does not include search results. The version-skew `warnings[]` appears only on the API path (§3.2.12). Where the goldens live, and whether they are stored files or a comparison of the two paths, is not specified.
> 3. **The TUI cite is wrong.** `src/tui/mod.rs:1664-1666` is the init batch: `poll`, `load_plugins`, `load_ratings`, `load_rss`, then `refresh_shows(false)`.
>    - `load_rss` (394-403) only reads the cache; `src/rss.rs:1-4` says listing never touches the network.
>    - The real on-open RSS refresh is the `rss_initial_refresh_pending` flag: declared at 193, set at 282, consumed at 1921-1924, which calls `refresh_rss`.
>    - The interaction test `rss_initial_check_and_download_dispatch_only_once` (3437-3450) and the test helper at 3042 cover it and would need changing.
>    - Only line 1666 (`refresh_shows(false)`) is a real refresh. Deleting 1664-1666 literally would stop the poll, the plugin list, the ratings and the feed display.
> 4. **An unannounced behaviour change.** Today the TUI refreshes every feed on every open, whatever its age (README.md:50-51). §3.7 calls the 30-minute rule "today's rule", but its citation, README.md:36, is about Shows. After this change, opening the TUI within 30 minutes refreshes no feeds. No README update is listed.
> 5. **The TUI never sees the daemon's refresh.** Until P4-03 the TUI reads `rss.json` directly in its own process (`qbt.rs:576-578` calls `rss::list`). It loads `shows.json` once at start (`tui/mod.rs:219`) and never reloads it. Nothing tells it to reload after the daemon writes. The options would be subscribing to `rss.feeds` and `shows` on the presence connection, an mtime poll, or living with stale data until `r`.
> 6. **Fallback edge cases are unspecified:**
>    - `hello` answers but the daemon is older and lacks a cap (in-process or error?).
>    - `hello` answers with `state: "stopping"` or `"parked"`.
>    - Legacy mode (`engine = "qbittorrent"`, kept until P4-09): today these verbs go to the WebUI (`main.rs:644`, `649`), and the item does not say which path applies.
> 7. **Async ops versus today's synchronous CLI.** `rss.refresh` returns `{state: "pending"}` and no `mutation.result` is defined for it; today `rss refresh` waits for the fetch (`rss.rs:222`). Should the CLI wait, and how? The same question applies to `shows.check` and `search.install`.
> 8. **Selector resolution over the API.** What `Feed.id` is (the `uid` from `stable_id(url)` at `rss.rs:175`, or the name) is never defined. The CLI would have to resolve feed names by substring (`main.rs:1152-1160`) and run `Shows::find` locally on `rss.feeds` and `shows.list`. A.3 ("ambiguous, raised by the CLI before calling") supports doing it client-side.
> 9. **`shows seen` toggles; the API only sets.** `shows.set_read {read}` needs the current state read first, which can race. Whether `all` maps to `mark_show_read` and its `{count}` to today's `marked` is unstated.
> 10. **`search --limit` and `--wait`.** It is unstated whether `limit` goes to `search.start` (at most 5,000) or the CLI fetches everything and truncates after the shared dedupe and sort, as today (`main.rs:686-701`). It is also unstated whether `complete` means `search.done` arrived before `--wait` ran out, and how `search.done.warnings` maps to exit 5.
> 11. **`imdb --refresh` and the stale-ratings auto-refresh (`main.rs:341-344`).** No API op refreshes the ratings dataset. P3-04 loads the index once per daemon life, so an in-process refresh leaves a running daemon on the old index.
> 12. **Missing dependency.** `imdb` through `imdb.lookup` needs P3-04, which "Depends on" does not list.
> 13. **`rss grab` and `shows grab`.** It is unclear whether P3-07 should switch them from P1-11's `torrents.add` path to `rss.grab` and `shows.grab`. Mapping the "as `torrents.add`" result back to today's `{action, hash, name, feed}` needs extra reads, and today's `hash` is `magnet_hash(url)`, which is null for a `.torrent` URL.
> 14. **"`agent_contract.py` reruns its search cases":** there are none. `run.sh` (R-001 to R-012, the cases P1-11 ports) has no search, rss, shows or imdb cases, and P1-11's test list adds none.
> 15. **`tui_open_one_refresh`.** How to count refreshes is undefined (fixture-server hit count or a log line), and so is whether "one" means per feed or in total. The fixture must first make the feed older than 30 minutes, presumably by editing `last_refresh_at` in `rss.json`.
> 16. **Smaller gaps:**
>     - Must the human (non-JSON) output also be identical?
>     - Where does the shared shape builder live? §3.3 says `main.rs` is dispatch only but names no CLI module.
>     - The size label S does not fit the scope (about 14 verbs, the TUI and three test suites); M to L is more realistic.

Decided and fixed in P3-07 (now L): the CLI keeps contract 1.x's shapes, rebuilt from A.9 objects
by `src/cli/shapes.rs` (new, §3.3) with the in-process path producing the same objects
(`src/cli/fallback.rs`); A.9 gains the additive fields the old shapes need (`FeedItem.description`,
`torrent_url`; `Show.added_at`, `counts`; `Release.season`, `episode`, `season_pack`, `magnet`,
`candidates`; `SearchResult.provider_url`; `Provider.full_name`; `ImdbCard.poster_url`) and
`Feed.id` is defined; byte-identical after `warnings[]` is removed; the orders of every list
(Appendix F determinism); the TUI citations corrected (`rss_initial_refresh_pending`,
`refresh_shows(false)`) and the test renamed; the behaviour change to the 30-minute rule named
and the README update listed; the TUI's presence connection subscribes to `rss.feeds` and `shows`
to reload its files until P4-03; fallback edge cases (missing cap, `stopping`, `parked`, legacy
mode); pending operations awaited as today; client-side selector resolution; `shows seen`
semantics; `search --limit`/`--wait`/exit 5; `imdb.lookup {refresh}` (P3-04, A.6); `rss grab` and
`shows grab` moved to their ops; the four shape goldens in `agent_contract.py`; the
`tui_open_one_refresh` counting; human output through the same shaper; P3-04 and P1-15 added to
Depends.

#### P1-11 (fresh agent)

> **Verdict: not implementable as written.** The core of the item can be built after reading the sections it cites: the NDJSON client, moving the existing verbs onto it, the offline ledger writes, `status`/`--quiet`, `capabilities` 1.2.0 and the `agent_contract.py` skeleton. But it is placed in the plan's dependency graph after only P1-04 and P1-09. Several of its own Do clauses and named tests need items that are not upstream, and its JSON-shape rule contradicts itself. An agent would have to stub or pull other items forward and make product decisions on its own.
>
> ## A. Blocking problems (contradictions and dependency gaps)
>
> 1. **Tests and Do clauses that need items that are not upstream.** I checked the §9.4 table: P1-11 needs only P1-04 and P1-09.
>    - `wait_exits_8_when_daemon_idles_out` needs idle exit, which is P1-12 (it depends on P1-08).
>    - `wait_survives_upgrade_restart` needs exit 75 or `app.restart {scope: daemon}`. Both are P1-17; A.4 lists `app.restart` as "P1-06, P1-17". Also, in `--no-systemd` CI nothing restarts a daemon that exits 75.
>    - `version_skew_warning` needs `FBT_BUILD_ID` from `build.rs`, `FBT_BUILD_ID_OVERRIDE` and two builds. All are P1-17. Without them the CLI has no build id to compare against `hello.server.build`.
>    - "`events` exit 4 when down": the `events` verb comes from P1-18, which depends on P1-11. P1-18 has its own `events_down_exit_4`.
>    - "`log` wired": `fbittorrent log` and `app.log` are P1-14, not upstream.
>    - "(P1-04 added the verbs)" is wrong for `serve` (P1-03 adds it) and `log` (P1-14 adds it).
>    - `restart` is also implemented by P1-17 ("`fbittorrent restart` sends it and waits for the new hello"), so the two items overlap.
>    - The `subscribe` iterator has no live server until P1-08, which is not upstream. It can only be tested against the P1-05 fixtures `subscribe*.ndjson`, and `wait` must use the `torrents.get` polling fallback.
>    - The new `status` shape adds `daemon`, and `triage` adds `backend.daemon` with `idle_exit_in_s` and `busy_reasons`. Those exist only in `Summary` (the P1-08 topic, values from P1-12). No op in `status`'s listed set (`hello`, `transfer.info`, `torrents.list`) returns them, yet M1a demo step 6 (§9.7) expects `status --json` to count down.
>    - The client is "reused unchanged by the TUI (P1-15)", but P1-15 does not depend on P1-11 in §9.4.
> 2. **Appendix F describes the end state through P7, and the item says "every CLI command in Appendix F".** The agent must work out which rows and columns are in scope now. Examples that belong to later items:
>    - `undo` (P6-08), `recheck` (P5-03), `reannounce` (P6-01), `queue` (P6-05), `limits` (P5-04), `limit` (P7-06), `move` (P7-02), `rename` (P7-03).
>    - `shows quality` (P3-18), `api` and `events` (P1-18), `wait --until` (P5-05), `list --state/--origin`.
>    - `status.history` and `all_time` (P6-03/P6-04), `triage` `counts.ready` (P5-05), `add.deferred` (P7-09), `show` `seeds` (P5-01) and `undo_until` (P6-08), `setup` `actions[]` (P2-07).
>    - The §8.5 list that `robot-docs` must cover includes `events`, `api` and `--until playable`, none of which exist at P1-11. Listing them in `commands[]` would fail `commands_match_clap`.
> 3. **The JSON-shape rule contradicts itself.**
>    - Appendix F says existing shapes are unchanged or extended. Today `list`, `show`, `status` and `wait` print the qBittorrent-shaped `qbt::Torrent` (`src/qbt.rs:11-36`), which `robot-docs` calls "stable within contract 1.x".
>    - "`list` extended with `state`" collides with the existing `state` field, which uses qBittorrent values (`stoppedDL`, `metaDL`, …).
>    - The API `Torrent`/`File` (A.9) lacks `category`, `tags`, `ratio_limit`, `up_limit`, `seq_dl`, `f_l_piece_prio`, `num_leechs`, `progress_known`, `eta`, `connection`, and the file fields `name`, `priority` and `progress_known`. So either the CLI rebuilds the old shape on its side (rules not given) or the shape changes.
>    - P1-18's Accept ("`api torrents.list` equals `list --json`'s `torrents` array") implies the API shape.
>    - A.12's `add --json` example (`source: "magnet"`, no `sequential`/`first_last`) differs from today's output (`src/main.rs:541,576`).
> 4. **Ops that do not exist yet.**
>    - `settings` needs `app.settings.set`, which is P3-05. Today it uses `app/setQueue` and `app/restart` over `engine.sock`.
>    - `rss grab` and `shows grab` need `rss.grab`/`shows.grab` (P3-02/P3-03).
>    - Appendix F calls `reconcile` a stub that exits 2, but today it runs in native mode; only `seq`/`first-last` refuse (`src/main.rs:314-318`).
> 5. **`--no-start` and `autostart = false` against reads.**
>    - §3.2.3 says any command with `--no-start` exits 4. Appendix F `list` says "4 only with `--no-start`". So does a read that would otherwise fall back to the ledger fail instead?
>    - Does `status` with `autostart = false` exit 4, or answer `running: false`? `native.py` writes `autostart = false` into every test config.
>
> ## B. Facts I would have to guess (not answered anywhere)
>
> - **`origin` on `torrents.add`.**
>   - Whether P1-06's args accept `origin` at all. It is not in A.5's Since column, the `origin` cap is P6-08, and `deny_unknown_fields` would make it `invalid_args`.
>   - How `add` knows it was launched by the desktop handler (`origin: "handler"`). No flag or env var is defined.
> - **The deprecation warning when `engine = "qbittorrent"`:** its text, whether it goes to stderr or `warnings[]`, and which commands print it.
> - **Legacy engine until P4-09:** whether the CLI keeps the `qbt::Client` WebUI path in that mode.
> - **Compat period (§3.6):**
>   - The probe order between `api.sock` and `engine.sock`.
>   - Whether P1-11 must build "reads through an old TUI-hosted `engine.sock`, writes refused with 'an older fbittorrent app is running; quit it with Q and retry'". It is in §3.6 and M1a demo step 1, but not in the item. The test `old_engine_sock_reads_only` was dropped in round 3 (`reviews/round-3-consistency.md` B2).
> - **`warnings[]`:** whether each entry is a string or an object, and where it goes in outputs that have no `warnings` today (the `act()` shape, `add`).
> - **Ledger-fallback reads:** where `running: false` goes in `list`, `show` and `triage` (top level or per torrent).
> - **`status`:**
>   - The running shape: whether to add `running: true`, and where `connection` comes from (no API source).
>   - Whether the stopped shape keeps `engine`. Today's output has it; §3.2.13 omits it.
> - **`capabilities` 1.2.0:**
>   - `CommandDoc` fields: keep `needs_backend`? add a "starts" field?
>   - Whether nested `rss`/`shows` subcommands are listed.
>   - Whether `commands[]` is generated at runtime from clap or hand-written and checked by a unit test.
>   - The full `changes[]` entry schema (the "…" in Appendix F) and which non-breaking changes to list.
>   - New text for `engines`, `conventions.native_connect`, `defaults` and `files.socket`.
> - **How the named tests work:**
>   - `retryable_never_maps_to_1`: how a black-box Python test lists API error codes and their exit mapping. Nothing exposes A.3; maybe it should be a Rust unit test.
>   - `commands_match_clap`: it is named both as a unit test and as an `agent_contract.py` case. How does Python list the clap subcommands (parse `--help`)?
>   - `exit_code_dictionary_total`:
>     - Exit 5 needs a failing search provider. P3-01's fake providers come later, and today "no providers" exits 4.
>     - Exit 1 needs a completed torrent fixture for `start` on `done`; `internal` has no trigger.
>     - `busy` has no stated trigger.
> - **`scripts/verify-determinism.sh`:**
>   - Which commands count as "every `--json` read" (`status`, `list`, `show`, `triage`, `config`, `capabilities`; also `rss list`, `shows list`, `search --plugins`, `imdb`?).
>   - What `SOURCE_DATE_EPOCH` is meant to change. No code reads it, and Appendix F says it does not touch `*_at`.
>   - Which fields to mask. The old shapes use `added_on`/`completion_on`, not `*_at`, and `wait` has `elapsed_secs`.
>   - It needs a built binary and an idle disposable daemon, but the `lint` CI job has no build step (§8.4) and E.2 lists only python3. Does the script start its own daemon?
> - **Isolating the test daemon:** how the CLI under test avoids `systemctl --user start fbittorrent.service` reaching the owner's real unit. P1-04 and P1-07 mention "a throwaway unit name derived from the temp dir" but not how the CLI picks it up. Also, does `agent_contract.py` run in `--systemd` or `--no-systemd` mode?
> - **Offline writes:**
>   - How many hello/lock retry rounds to allow before giving up. The two steps can loop: lock held for 2 s, retry `hello`, `ECONNREFUSED`, back to the lock.
>   - What to do when `hello` succeeds but the daemon answers `state: stopping`.
>   - The JSON of an offline `stop`/`delete` (add `running: false`?).
>   - Whether an offline delete also removes `<hash>.torrent`, as the parked path does (`src/engine.rs:1338-1340`).
>   - What "both intents" means in `offline_write_waits_for_draining_daemon`.
> - **Selectors:**
>   - `stop all` with zero torrents: the API rejects an empty `hashes` (1..256 per P1-09), but today it returns `count: 0`.
>   - Whether `all` is sent in chunks of 256 for start, stop and seed. Only `recheck` says so.
> - **`wait`:**
>   - What "one reconnect" means, given that in `--no-systemd` mode `ensure_daemon` would have to spawn `serve --detach`.
>   - Which `bye` reasons reconnect and which end in exit 8.
>   - The JSON for the idle-out case. Presumably today's "every waited torrent is stopped" envelope (`src/main.rs:761-763`).
> - **Client design:**
>   - How a blocking client keeps 8 requests in flight (reader thread or pipelining).
>   - How unsolicited events interleave with responses.
>   - The 200 ms connect timeout, given that Unix-socket connect in std has no timeout.
>   - Idempotency comes from the hard-coded A.1 list, since `api::ops::TABLE` is P1-18.
> - **README:** there is no Lifecycle section today (`## CLI` at line 102, `### For agents` at 137); its content has to be written from §3.2.
> - **Size:** "M" looks understated for a new client, every verb moved over, offline writes, `agent.rs`, about 15 test cases, a script and the README.
>
> ## C. Facts I looked up, and where they are
>
> | Fact | Where |
> |---|---|
> | `hello` args and result, `limits.inflight` = 8 | §3.4.3, A.2 |
> | Envelope, `id` and `ref` rules, `instance` on resend | §3.4.2, A.1 |
> | Ref format `<name>-<start-millis>-<counter>` | §3.4.8 |
> | `rev`/gap handling, 15 s heartbeat, 45 s dead | §3.4.7, A.8 |
> | Backoff 0.5, 1, 2, 4, 8 s, then every 10 s | §3.2.11 |
> | What to do when the connection drops mid-request; the idempotent ops list | §3.2.4 step 6, A.1 |
> | `ensure_daemon` steps | §3.2.4 (P1-04 does steps 1-5) |
> | "Starts" rule | §3.2.3, Appendix F |
> | Offline lock order | §3.2.3 row; ledger `data_dir/engine/torrents.json`; parked code `src/engine.rs:1233-1355`; `engine.lock` `src/ipc.rs:136-164` |
> | `log`/`events` exit-4 hint | §3.2.3 |
> | Add hash and URL `pending` → `mutation.result` | §3.4.8, P1-09, A.8 |
> | Version-skew message | §3.2.12 |
> | `status` stopped shape | §3.2.13, Appendix F intro, Q12 |
> | `--quiet` exit 4 and `busy` → 4 | DL-43, A.3 |
> | Error-to-exit table and how each exit code is reached | A.3, Appendix F |
> | Current `CommandDoc` and `Cmd` enum | `src/agent.rs:126-160`, `src/main.rs:48-234` |
> | §8.5 list; menu gate | §8.5; §8.6 |
> | R-cases: R-001 to R-012, so "R-0xx" means R-012 | `regression_tests/run.sh` |
> | `fbtlog.py` API; `ApiConn` | P0-15; P1-06 |
> | Log shape | §9.1, E.1 |
> | `capabilities_upload_text` (32 KiB/s from `NO_UPLOAD_LIMIT`) | P0-12, `src/qbt.rs:44` |
> | CI jobs | §8.4 |
> | Determinism rule | Appendix F, last paragraph |
> | `autostart = false` row | §3.2.3 |
>
> The cited code lines (`src/main.rs:407-415`, `525-532`, `550-560`, `718-728`) are accurate today. They will drift once P1-02 to P1-04 edit `main.rs`.
>
> I created and changed no files and ran nothing that changes state.

Decided and fixed in P1-11 (now L): it depends on P1-12, P1-14 and P1-17 as well (its `wait`,
skew and `status.daemon` clauses need them; P1-15 now depends on P1-11); the scope is the verbs
that exist at P1, with every later Appendix F row landing with its item; contract 1.2.0 is
introduced here and grows by patch versions (the §9.3 preamble rule; §8.5 and P1-18 adjusted);
`list`, `show`, `status` and `wait` keep the qBittorrent-shaped torrent of contract 1.x, built
from the A.9 `Torrent` by `cli::shapes::torrent()`, and gain only fields with new names
(`native_state`, `reason`, `checked`, `origin`, later `playable`; Appendix F `list` row and A.12's
`add` example corrected; P1-18's accept goes through the shaper); `settings`, `rss grab` and
`shows grab` keep their interim paths until P3-05 and P3-07; `reconcile` is the P0-07 stub, exit 0
(Appendix F); `--no-start` and `autostart = false` stop only commands that would start (§3.2.3
rows and the `list` row); `--origin` flag and the handler's `--origin handler`; the deprecation
line and `warnings: [{code, message}]`; the compat-period probe order and refusal; `running: false`
top-level; the full `status` shapes; `CommandDoc.starts`/`since`, runtime-generated `commands[]`,
the `changes[]` schema; `retryable_never_maps_to_1` as a Rust unit test; triggers for every exit
code; `verify-determinism.sh` scope, `SOURCE_DATE_EPOCH` semantics and its move to the `native`
CI job (§8.4, Appendix E); `FBITTORRENT_UNIT` (P1-04) for isolation; offline-write rounds and
JSON; `all` chunking and `stop all` with nothing; `wait`'s reconnect and `bye` handling; the
client's threading and connect timeout; the README Lifecycle section.

#### P2-06 (fresh agent)

> ## Verdict: not implementable as written
>
> The plumbing is well specified once you read the cited sections: the files, the `serve` flags, the temporary XDG layout, the `FBTQS-STEP` log contract and the Service actions. What breaks it is the test scenario itself. It cannot pass against the daemon its dependencies deliver, and the acceptance check reads a field the plan has removed. Two more points contradict each other. These need a decision from whoever owns the plan; they cannot be looked up. Once they are settled, the item is a normal M.
>
> I created and changed nothing. I also did not run `qs` on a test file, because that would have meant writing one.
>
> ## A. Problems that need a decision
>
> 1. **Wrong entry file name.** `qs -p <dir>` loads `<dir>/shell.qml` (from `qs --help`, Quickshell 0.3.2 as installed). The item says `plugin/tests/smoke/main.qml` but runs `qs -p plugin/tests/smoke`. Either the file is called `shell.qml` or the command is `qs -p plugin/tests/smoke/main.qml`. P2-02's `plugin/tests/store/main.qml` has the same problem. Sprout's reference script writes a `shell.qml`.
>
> 2. **The never-resolving magnet never appears in the model, so add, pause, resume and remove all fail.**
>    - The vendored librqbit finishes resolving metadata before it creates the torrent (`vendor/rqbit/librqbit/src/session.rs:1118-1138`).
>    - Today's `add` waits up to 90 s for that and writes the ledger record only afterwards (`src/engine.rs:905-950`).
>    - List rows come from `Record` (A.10).
>    - §3.4.8 and P1-09 promise only that the *hash* comes back at once. No P1 or P2 item makes a row visible before metadata arrives; P3-06 (staged adds) and P7-09 (deferred adds) do, later.
>    - Result: "wait for it in the model" runs into the 60 s timeout, and `remove` finds nothing to remove.
>    - Even if the row existed, the §9.3 default rule (PLAN.md:2954-2955) says a mutation on a `metadata` torrent answers `invalid_state`, so `pause` would be refused. Read literally, the same rule also refuses `start` on a paused torrent.
>    - Sprout's design works only because qBittorrent lists a magnet before it has metadata.
>    - Options: have P1-06/P1-09 create a `metadata` row from the magnet's hash and name, or switch the fixture to a `.torrent` with no peers saved in the temporary directory, or use P1-07's local magnet with a seeding peer.
>
> 3. **The acceptance check reads a field that does not exist.** DL-59 removed `Summary.daemon.clients` (PLAN.md:5648). Appendix F's `status` builds its `daemon` object from `hello`, `transfer.info` and `torrents.list` only (PLAN.md:6412).
>    - The only connection counter is `app.info.clients.{cli,tui,dashboard,bar,agent,test}`, counted since the daemon started (A.4, PLAN.md:5801). It is reachable as `fbittorrent api app.info` (PLAN.md:6446).
>    - The check has to compare `clients.bar`, not a total, because running the check itself adds a `cli` connection.
>    - It is meaningless if the owner's daemon is not running, and wrong if it restarts between the two readings (the counters reset).
>    - Not stated: whether `qs-smoke.sh` runs this check itself or a reviewer does it by hand.
>
> 4. **The smoke test would make exactly the connection the acceptance check forbids.** Under P2-01 and §4.4 rules 1 and 3, `Service.qml` points its socket at `Quickshell.env("XDG_RUNTIME_DIR")/fbittorrent/api.sock` and sends `hello` as soon as it is created. That means it connects to the owner's daemon before `useRuntimeDir(tmp)` can be called.
>    - One fix: start `qs` with the temporary `XDG_RUNTIME_DIR`. Then `WAYLAND_DISPLAY` has to be an absolute path, because the Wayland socket lives in the real runtime directory.
>    - The other fix: give `Service.qml` an initial property or environment override for the runtime directory.
>    - Neither is in the item.
>
> 5. **The `reconnect_after_reload` case contradicts itself** (P2-01, PLAN.md:3870-3872: "touches the plugin directory and asserts `state()` shows a new instance within 1 s").
>    - `instance` is the daemon's id (§3.2.5, §4.17). A plugin reload does not restart the daemon, so it never changes.
>    - `plugin/oliver.torrents/` is the live plugin; it is symlinked from `~/.config/omarchy/plugins/oliver.torrents`. If the live shell reloads, its service reconnects to the owner's daemon. That breaks the acceptance check and the item's "without touching the running shell".
>      - The plan says repo edits do reload the live shell (§4.15). But Omarchy's watcher is `inotifywait -m -r` (`/usr/share/omarchy/shell/services/PluginRegistry.qml:716-727`), and its man page says recursion does not follow symlinks. So it is unknown whether a touch reaches the live shell at all.
>    - In the separate test instance, Quickshell's own file watcher is on; Omarchy turns it off only for the live shell (`QS_DISABLE_FILE_WATCHER=1` in `omarchy-launch-shell`). It is unknown whether that watcher notices a `Service.qml` loaded from outside the test folder, especially through `Qt.createComponent`.
>    - A reload also restarts the QML test driver. Keeping the step state needs `PersistentProperties` or an outside driver (`qs ipc -p <path> call oliver.torrents state`).
>    - Setting `QS_NO_RELOAD_POPUP=1` would stop Quickshell's reload popup from appearing on screen.
>    - The case name differs too: `reconnect` in Tests, `reconnect_after_reload` in Do and in P2-01.
>
> 6. **The `qs.Commons` import is required, not an optional experiment.**
>    - P2-01's `Service.qml` uses `Util.execArgv` (PLAN.md:1838, 1848, 3857). That is the `qs.Commons` `Util` singleton (`/usr/share/omarchy/shell/Commons/Util.qml:77`; qmldir `module qs.Commons`). If the import fails in the separate instance, `Service.qml` does not load and the whole smoke test fails. The §4.16 fallback only covers the visual components.
>    - `QML_IMPORT_PATH=/usr/share/omarchy/shell` probably cannot work. An import path resolves `qs.Commons` to `<path>/qs/Commons/qmldir`, which does not exist. Quickshell resolves `qs.*` from the config root, so a symlink `smoke/Commons -> /usr/share/omarchy/shell/Commons` (and the same for `Ui`) is more likely to work. This is unverified, as the item asks.
>    - The import probe should be a separate QML file, so a failure is recorded without killing the smoke test.
>
> ## B. Facts I had to look up (found)
>
> - **`serve` flags** `--idle-exit-secs N` and `--no-notify` exist: §3.2.1 (PLAN.md:628-640).
> - **Temporary XDG variables**: P1-07 lists five under one `mkdtemp` (`XDG_RUNTIME_DIR`, `_DATA_`, `_CONFIG_`, `_STATE_`, `_CACHE_HOME`). The runtime directory must be absolute, owned by the user, mode 0700 and not a symlink (§3.2.5). `native.py:44-59` is the working model.
> - **Log line format** (`FBTQS-STEP {json}`, final lines, paths `target/test-logs/<suite>/<case>.jsonl`): §9.1 and E.1. **`qs-log.py`'s job**: P0-15 (around PLAN.md:3290). Its command line (suite name, timeout, how a case that never reports is treated) is not defined anywhere; it has to be read from P0-15's implementation.
> - **Service actions and their arguments** (`remove(hash, deleteFiles)`, the ref format, `add(source, stage)`): §4.4 and P2-01. `pause` maps to `torrents.stop`, `resume` to `torrents.start`, remove to `torrents.remove {delete_files: false}` → `{state: done, undo_until?}` (A.5).
> - **The `FBT_TEST_HOOKS=1` guard**: `useRuntimeDir` refuses to run without it (§4.17, P2-01), and the item never says to set it in the test's environment. The sources disagree on `state()`: §4.17 says unguarded, P2-01 says guarded.
> - **`torrents` is only subscribed while the popup or dashboard is open** (§4.4 rule 9, P2-01). The smoke test has to set the Service's "popup open" state to get rows at all. P2-01 does not name that property or function.
> - **The host injects `manifest`** into services (`/usr/share/omarchy/shell/shell.qml:944-950`). The smoke test has to supply it for `hello`'s `client.version`, or the Service has to cope without it.
> - **The `FBT_TEST_QS` block in `all.sh`** comes from P0-06 and P2-02. Adding the `qs-smoke.sh` line is implied by Accept but missing from Do. Today's `all.sh` has no QS block.
> - **Sprout's reference script** (OR §8 item 9) has a local copy at `/tmp/claude-1000/-home-oliver-Projects-odcpw-fbittorrent/d7aee677-4c3c-40be-a662-cab58b5a412d/scratchpad/research/cuciz/repo/tools/test_quickshell.sh`. It uses `shell.qml`, `2>&1 | grep OTQS-` and a hard-timeout `Timer`.
>
> ## C. Facts I would have to guess (not in the plan)
>
> - **The disposable daemon's `config.toml`.** The default `peer_port` is 6882 (`src/config.rs:50`), and production binds `0.0.0.0:peer_port` (`src/engine.rs:426-429`), which collides with the owner's running daemon. It needs a free port, save path and `notify = false`, the way `native.py` writes them. Also: with no `network_interface`, the daemon would ask the public DHT about the random hash outside the VPN, which breaks §9.1's "local only" fixture rule.
> - **Which binary**: `target/release/fbittorrent` as `all.sh` builds it, or an argument or environment variable.
> - **How the temp path and the hash reach the QML**: an environment variable read with `Quickshell.env`, or `sed` substitution as Sprout does.
> - **Waiting for daemon readiness, and cleanup**: killing the daemon (it has a 600 s idle exit) and removing the temp directory in a trap.
> - **Success per step**: which model fields count as paused or resumed, and the time budget per step inside the 60 s.
> - **Output stream and format of `qs`.** The live shell's log shows lines like `DEBUG qml: …` with ANSI colours, and Sprout merges stderr into stdout. P0-15 says "stdout line", so the parser should use `--no-color` or `NO_COLOR` and match the marker mid-line.
> - **Suite name and log paths**, for example `target/test-logs/qs-smoke/`, and where the daemon's log goes.
> - **What `useRuntimeDir(path)` expects**: an `XDG_RUNTIME_DIR`-style root or the `fbittorrent/` directory itself.
> - **Which `IpcHandler` holds the hooks.** P2-03 keeps a popup handler with the same target, and Quickshell allows one handler per target.
> - **Socket path length**: it must stay under 108 bytes, so the temp directory should be short.
> - **What `plugin/tests/README.md` contains** and in what format.

Decided and fixed in P2-06 (every point): `shell.qml` for both test roots (P2-02 too); a magnet
add creates a `metadata` row at once (P1-09, a correction the UIs' "Fetching metadata" row needed
anyway) and the edge-state rule of the preamble now says which mutations are ledger intents
(`stop` on a `metadata` torrent works); the acceptance uses `fbittorrent api app.info`'s
`clients.bar`, run by the script itself with the stated skip conditions; `qs` starts with the
temporary `XDG_RUNTIME_DIR` and an absolute `WAYLAND_DISPLAY`, so the first `hello` goes to the
disposable daemon; the `reconnect` case restarts the disposable daemon (the daemon's `instance`
changes) and the plugin-reload case moves to the live-shell harness as
`qs-dashboard.sh::reload_reconnects`, with `instance` unchanged and the symlink question recorded
in §10.4; `qs.Commons` required through `Commons`/`Ui` symlinks in the test root (§4.16
corrected) with a separate `probe-ui.qml`; `FBT_TEST_HOOKS=1`, `QS_NO_RELOAD_POPUP=1`,
`NO_COLOR=1` set; `state()` unguarded (§4.17 and P2-01 agree); `svc.popupOpen` named (P2-01);
`manifest` supplied; the `all.sh` block; the daemon's config (`network_interface = "lo"`, a free
port, `notify = false`); `FBT_BIN`; `FBT_SMOKE_HASH`; readiness and cleanup; per-step success
fields; `qs-log.py` reads both streams and matches markers mid-line (P0-15); suite name and log
paths; `useRuntimeDir` takes a runtime root (§4.17); the one `IpcHandler` on `BarWidget.qml`
(P2-01); the short temp path; the README's table.

### Self-containment tally

| Batch | Items | As written | After reading the cited sections | Not implementable |
|---|---|---|---|---|
| validator (round 3 review) | P7-03, P7-06, P0-08, P0-15, P1-09, P3-13, P4-02, P6-08 | 0 | 3 | 5 |
| integration, batch 1 (after the rewrite) | P0-02, P3-10, P5-03, P6-01 | 0 | 4 | 0 |
| integration, batch 2 (after the batch-1 fixes) | P1-11, P2-06, P3-07, P6-05 | 0 | 1 | 3 |

Reading: no item passes "as written" under this prompt, and none will while items anchor into
canonical sections by design (the §9.3 preamble says so); the useful signal is the third column.
Batch 1's four were all buildable from the plan; batch 2 drew three items whose problems were
plan-level contradictions or undecided designs (P1-11's dependencies and the contract-1.x shape
rule; P2-06's scenario against a daemon that had no `metadata` rows; P3-07's shared-shape
decision), which the rewrite had inherited and which are now decided in the plan. Every gap the
twelve fresh agents named is closed in PLAN.md; none was left as "the implementer decides". The
transcripts above are the record. Round 4 should draw another four before the beads conversion;
P1-06, P1-08, P3-09 and P4-04 (the L items with the most clauses) are the ones most likely to
hide a decision.

### Script outputs (final state)

`r3/dagcheck.py` (the round-2 reviewer's script with two regexes widened to `M[0-6]a?` for the
new milestone; the output format is unchanged), after the last edit:

```
items in §9.3: 109 (active 107); rows in §9.4: 107
no cycle; 107 nodes, 21 levels
orphans (no consumer): []
critical path to M1: 16 items, weight 31 (S=1,M=2,L=4): M1 <- M1a <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-03
critical path to M2: 18 items, weight 37 (S=1,M=2,L=4): M2 <- P3-16 <- P3-18 <- P3-07 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-03
critical path to M3: 19 items, weight 45 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-03
critical path to M4: 18 items, weight 41 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-03
sizes: Counter({'M': 44, 'S': 42, 'L': 13})
```

§9.4's adjacency table, level table, critical path and weighted paths were written by
`r3/regen94.py` from §9.3 (the dagcheck parser plus a writer), so §9.3 and §9.4 agree by
construction; the script will be committed as `scripts/dagcheck.py`'s sibling in P0-06.

`mockwidth.py` (unchanged from round 2):

```
L2309 [OK] '5.3 Module split': no declared width, max 111, box widths []
L2336 [OK] '5.4 One action table': no declared width, max 83, box widths []
L2455 [OK] 'Torrents at 80×24 (narrow):': declared 80, max 80, box widths [80]
L2471 [OK] 'Torrents at 120×30 (medium):': declared 120, max 119, box widths [116]
L2491 [OK] 'Details modal at 80×24 (Enter on the narrow layout):': declared 80, max 80, box widths [80]
L6329 [OK] "Popup, idle with a ready-to-watch row (owner's theme):": no declared width, max 54, box widths [54]
L6347 [OK] 'Popup, waiting for the VPN, with a confirm strip open:': no declared width, max 54, box widths [54]
L6365 [OK] 'Popup, daemon not running (with `showWhenStopped`):': no declared width, max 54, box widths [54]
L6377 [OK] 'Dashboard, Search view with the poster grid (empty query, `v` grid):': no declared width, max 113, box widths [113]
L6397 [OK] 'Dashboard, Search view (rows):': no declared width, max 113, box widths [113]
L6413 [OK] 'Dashboard at 900 px (chip-row tier, §4.3.3: SIZE, PROGRESS, ↓, ETA, PE': no declared width, max 90, box widths [90]
L6434 [OK] 'Dashboard, remove dialog (`ConfirmDialog`, `selectedIndex: 0`):': no declared width, max 90, box widths []
L6451 [OK] 'Dashboard, add sheet:': no declared width, max 67, box widths []
L6467 [OK] 'TUI Search at 80×24 (narrow), card hidden:': declared 80, max 80, box widths [80]
L6484 [OK] 'TUI Shows at 80×24 (narrow), one pane with breadcrumb:': declared 80, max 80, box widths [80]
15 blocks measured, 0 need fixing
```

Counts: PLAN.md 6,688 lines; 101 IDs (99 active items, P7-07 and P7-08 superseded) and 8
milestones (M0, M1a, M1 to M6); sizes S 42, M 44, L 13 (P1-11, P3-07 and P5-03 grew during the
sampling fixes; nothing was split).

### Validation loop (planning-workflow)

- Self-containment: the two batches above; every named gap closed in PLAN.md.
- Dependency graph: generated from §9.3 after every edit; no cycle, no orphan; the M1 path is 31
  S-units (down from 35) and has two branches of equal weight (§9.2).
- Justification: the six Weak/Missing paragraphs the validator sampled now carry their why; the
  round-3 decisions are DL-53 to DL-65 with their sections.
- Steady state: not reached. This round's diff is the largest so far (every §9.3 item rewritten)
  and the second sampling batch still found three plan-level decisions to make. One more review
  round plus a sampling batch of four (P1-06, P1-08, P3-09, P4-04 suggested) before beads.

### Still not grounded (carried in §10.4)

Round 2's list, plus: whether Omarchy's `inotifywait -m -r` watcher reloads a symlinked plugin
folder (P2-01 records it; if not, `--plugin=copy` is the development default); whether an
isolated `qs -p` sees `qs.Commons`/`qs.Ui` through symlinks in its config root (P2-06's
`probe-ui.qml`); the Asupersync licence rider's bearing on a public fork (Q3, the owner's call);
the fork's `state.rs` behaviour change (`with_independent_module_sequences`), which P0-02 now
documents in `FBITTORRENT_PATCH.md` but nobody has justified in writing.

## Round 4 (integration), 2026-10-18

Input: `reviews/round-4-fresh-read.md` (1,422 lines; one independent cold read of the round-3
`PLAN.md`: 24 defects A1 to A24, 6 machinery cuts B1 to B6, 7 improvements C1 to C7, a 22-step
walk of the M1a demo (D) with four gaps, four self-containment probes (E.1 to E.4) with eight
derived hunks, 33 re-verified citations and 8 recomputed numbers (F), four owner questions (G1 to
G4), and a verifier run over its 73 hunks (H: 0 misses)). Brief: integrate in place; only
`PLAN.md`, `review-log.md`, `README.md`; IDs stable; cuts of machinery only; B4 only after Q1 is
answered; disagreements recorded here rather than skipped; then re-run `dagcheck.py` and
`mockwidth.py`, regenerate §9.4, fix stale numbers, refresh §0 within its length.

Method: `r3/apply_review.py` applied 69 of the 73 hunks by content (exact-once matching on the
removed and context lines; B4's four hunks skipped by index); `r4/followons.py` applied 46
follow-on edits as exact-once pairs; `r4/regen94.py` (round 3's generator plus a tie enumerator
and `textwrap`) rewrote §9.4; `r3/dagcheck.py` and `review/mockwidth.py` ran before and after.
Every source citation a hunk introduced was checked on this machine before it was kept
(`src/main.rs:391,416-430,1296-1300`, `plugin/oliver.torrents/Model.js:101-116`,
`src/engine.rs:154,517-546,577-583`, `Ui/PanelKeyCatcher.qml:47,59-60,93-95`,
`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:201,910`).

### Verdict per review item

| Item | Verdict | What was done | Where |
|---|---|---|---|
| A1 M1a "26 S-units" vs 31 | agree, number superseded | Hunk applied; then A23's edge put P0-02 (S) on the path, so the weight is **32**, not 31, in §9.2, §9.4, R18 and §0 | §9.2, §9.4, §10.1 |
| A2 preamble vs §3.2.4 on `Starting`; `start`/`set_files` carve-outs | agree | Both hunks applied: an operation during `Starting` waits up to 60 s for the session; `start` on a completed stop-on-complete record is `invalid_state`; `set_files` with a non-empty `include` restores a dormant torrent | §9.3 preamble, P1-16 |
| A3 `install.sh` is P2-07's, after M1a | agree | Three hunks: P1-04 creates the script (build, `mv`, `setup --magnet`), P2-07 extends it, §8.6 says so; §9.7 step (1) names P1-04 | P1-04, P2-07, §8.6, §9.7 |
| A4 v0.1 popup never hides under DL-22; "compat socket" | agree | Five hunks: `Service.qml:15` tests `status.running !== false` (P0-07 Do and Accept); P1-11 keeps every 1.x running field; §0 and the §9.7 table say "through the new CLI"; §3.2.13 explains the M1a weeks | P0-07, P1-11, §0.5, §3.2.13, §9.7 |
| A5 who posts "Added" | agree | Hunk applied: the daemon, only for `origin: handler`; the CLI's plain toast on a TTY, never for the handler. G4 became Q33 | P1-13, §10.3 |
| A6 `subs: 8` too low | agree | Four hunks: 16 in the example, §3.4.10, P1-06 and A.6 (with A12's `since_seq`) | §3.4.3, §3.4.10, P1-06, A.6 |
| A7 bulk `X` cannot reach the dashboard | agree | Two hunks: window `Shortcut` in Appendix B; §4.12 states the catcher's folding and `tabRequested(±1)` (verified `PanelKeyCatcher.qml:59-60`) | Appendix B, §4.12 |
| A8 sort persistence | agree | Hunk applied (`dashboard.json`, clickable headers) | §4.3.2 |
| A9 Torznab caps and key | agree | Hunk applied (`api_key_file` only, 4 MiB, 30 s) | §7.3 |
| A10 details modal first tab | agree | Hunk applied (Overview first, `]` reaches Files) | §5.6 |
| A11 icon vanishes while reconnecting | agree, extended | Hunk applied; `reconnecting` defined in §4.4 rule 10 and added to P2-03's visibility expression, which the hunk did not touch | §4.4, §4.5, P2-03 |
| A12 `since_seq` and `oldest_seq` | agree | Two hunks (A.7, A.8) plus the A.6 argument in A6's hunk | A.6, A.7, A.8 |
| A13 request-less error frame | agree | Two hunks (A.1 rule, A.8 `error` event); `errors.ndjson` now carries the sequence (P1-05) | A.1, A.8, P1-05 |
| A14 wrong notify; no ledger counter | agree, extended | Two hunks applied; P1-19 now owns `Ledger.generation: AtomicU64` plus a `Condvar` and a test for it, since P1-08's hunk delegated to P1-19 without P1-19 saying so | §3.4.7, P1-08, P1-19 |
| A15 coalesced set not closed | agree | Hunk applied | §3.4.7 |
| A16 1 MiB cap vs growable results | agree, extended | Hunk applied; the lint rule it names is now in P2-05's rule list | §3.4.1, P2-05 |
| A17 "replaced by `cp`" | agree | Hunk applied | §3.2.12 |
| A18 `agents:972` path | agree | Hunk applied | §4.2.2 |
| A19 `dashboard` kind never sent, `agent` undefined | agree, extended | Three hunks applied; P1-11 sends `kind: "agent"` under `--json` | §3.4.3, A.2, A.4, P1-11 |
| A20 tab-bar state text at 80 columns | agree | Hunk applied; §5.7 row 17 says the same so the defect table and the item agree | P4-07, §5.7 |
| A21 pane title rows | agree | Hunk applied | §4.2.2 |
| A22 width tiers in `Style.space` units | agree | Hunk applied | §4.3.3 |
| A23 P0-02 before P0-01 | agree, reworded | Hunk applied; the kept clause "(the only owner action before P0-04)" is parsed by `dagcheck.py` as an edge P0-04 → P0-02 and produced a cycle, so it reads "before the landing commit of C3" | P0-02 |
| A24 §9.4 path "as the script prints it" | agree | `regen94.py` now enumerates every path of maximal weight and names the tie (three P0 prefixes, two post-P1-08 branches) | §9.4 |
| B1 `serve --test-proto` | agree | Four hunks; P1-05's `hello.ndjson` is told to carry the `proto: 2` frame the banner tests need | §3.2.1, P1-03, P1-05, P1-06, Appendix F |
| B2 `--detach`, double fork, `daemon.log`, `last-daemon.json`, `detached` | agree | Six hunks; follow-ons in R13, P1-04 Tests and P1-11's `agent_contract.py` wording ("foreground child", not "detached") | §3.2.4, §3.2.7, §3.2.14, A.2, P1-04, §10.1 |
| B3 timer-wheel runner retargeting | agree | Three hunks; §8.1's C5 title and contents and R19 updated as the review's follow-on note asked | P0-02, §8.1, §10.1 |
| B4 `last-ui` and `explicit` policies | **deferred** (as briefed) | Four hunks not applied. §3.2.10 carries a note, Q1 points at Q31, DL-02 and DL-68 record the pending cut; P1-12 implements all three until Q1 and Q31 are answered | §3.2.10, §10.2, §10.3 |
| B5 `ping` | agree | Two hunks | A.4, P1-06 |
| B6 per-torrent speed rings | agree | Two hunks | P6-03, §6.3 |
| C1 hide the key-hint row at rest | agree, as an assumed answer | Two hunks applied; the §4.5 and Appendix C popup mockups are captioned "keyboard mode"; G3 became Q32 with "hidden at rest" as the default (§0.3 A9) | §4.2.3, §4.5, P2-03, Appendix C, §10.3 |
| C2 sentinels in contract-1.x fields | agree | Hunk applied | P1-11 |
| C3 one unit rule | agree, went further | Hunk applied, then its clause "which several mockups in this plan still print" was replaced: the nine prose strings and the two Appendix C mockups now print GiB/MiB/TiB (box lines re-padded; `mockwidth.py` 0 need fixing). A plan rule its own mockups break is a contradiction waiting for a reviewer | §4.2.3, §4.3.4, §4.11, §6.3, P5-07, P6-04, P6-11, Appendix C |
| C4 `LICENSE`/`NOTICE` | agree | Hunk applied; P0-03's Accept names the two files | P0-03 |
| C5 Q3's third clause | agree | Hunk applied; G1 is also its own question (Q30) because "do you accept this reading for the whole project" is a different question from "where does the patch live" | §10.3 |
| C6 the dashboard looks like an app; say so | agree (no hunk in the review) | One bullet added at the top of §4.6 | §4.6 |
| C7 TUI at 80 columns readable | noted, nothing to change | A20 covers the one finding | §5.6 |
| D the 22-step M1a walk | agree | Gaps at steps 2, 9, 11, 15 and 20 are A3, A2, A5, A4; step 3's "when does `setup` run" is answered by A3's script | §9.7 |
| E.1 P1-06 probe (3 hunks) | agree, two details corrected | Head and Depends hunks applied; the tail hunk applied then corrected: `serde_json::Map` with `preserve_order` does not reject duplicate keys (it keeps the last like any map), so the server uses a `Visitor` over `MapAccess`; a `\r\n`-terminated line is valid JSON (`\r` is whitespace) and is accepted rather than refused, with the `garbage_table` row changed to match; `src/main.rs` has no `Interfaces` arm, the listing is `network_app::interfaces()` at `:391`. P1-19 is a direct dependency rather than a parenthetical, because the parenthetical would have been parsed as an edge anyway | P1-06 |
| E.2 P1-08 probe (1 hunk) | agree | Applied (`resources.py --daemon` spelled out, `resources.json` schema, "on change" topics ignore `interval_ms`, engine-restart behaviour, the slow-client log line) | P1-08 |
| E.3 P3-09 probe (2 hunks) | agree, extended | Applied; the `scroll(dy)` hook the hunk "adds to §4.17 here" is now a §4.17 row; `table-200.json` is listed with the fixture states in Appendix E | P3-09, §4.17, Appendix E |
| E.4 P4-04 probe (1 hunk) | agree | Applied (tabs per item, `[`/`]` stop at the ends, modal placement, column widths, blank versus `—`, `include_str!`, `layout::Rects`, goldens generated from the P0-16 fixture) | P4-04 |
| F.1 citations | agree | `src/agent.rs:118-120` → `:121`; `Style.qml:236-245` → `237-245`; the agents path (A18); the "4 s plus 4 s" drain overstatement (#9) fixed to the shared 4 s bound; D10 broadened to the ledger (0644) as the note under the table says | §3.2.2, §3.4.2, §4.2.1, §4.2.2, §1 D10 |
| F.2 numeric claims | agree | Nothing to change; the critical-path weight moved to 32 for the reason under A1 | §9.4 |
| G1 to G4 | agree | Q30 (rider reading), Q31 (B4, attached to Q1), Q32 (key-hint row), Q33 (TTY toast), each with the default the plan runs on; §0.3 A9 lists the assumed answers; R19 names Q30 | §0.3, §10.1, §10.3 |
| H hunk verification | agree | 73 hunks matched exact-once before application here too (`apply_review.py --dry`: 73 match, 0 miss) | — |

### Consequential edits not in any hunk

Header (round 4); §0 rewritten within its length (235 of 240 lines: §0.5 phases → milestones →
what each demo shows, §0.6 the plan in numbers and the open questions by number, §0.7 review
history replacing the three per-round sections; §0.1, §0.2, §0.4 unchanged, §0.3 gains A9);
§3.2.10 note on the deferred B4; §3.2.13 (the v0.1 widget during M1a); §4.4 rule 10
(`reconnecting`); §4.6 first bullet (C6); §4.17 `scroll(dy)`; §5.7 row 17; §8.1 C5; §9.2 (32
S-units, the P0 prefix on the path, reflowed); §9.7 M1a step (1); P0-03 Accept; P1-04 Tests;
P1-05 fixtures (`proto: 2`, the `error`+`bye` sequence); P1-06 corrections; P1-11 (`agent` kind,
"foreground child"); P1-19 (`Ledger.generation`, `Condvar`, test); P2-03 (`reconnecting`); P2-05
lint rule; R13, R18, R19; DL-02 note; DL-66 to DL-69; Q1 cross-reference; Q30 to Q33; Appendix C
captions and GiB mockups; Appendix E fixture row; DL-57 and DL-66 point at §0.5 (the M1a/M1 text
moved there).

Deviations from the review, all recorded above: A1's number (32, not 31); A23's and E.1's
parenthetical IDs reworded or made explicit so the generated graph has no spurious edge; E.1's
serde and `\r\n` mechanics; C3 applied to the mockups too; C1 applied as a default under Q32
rather than waiting; C5 plus a separate Q30; B4 deferred.

### Script outputs (final state)

`r3/dagcheck.py` (the reviewer's script with the `M1a` regexes of round 3):

```
items in §9.3: 109 (active 107); rows in §9.4: 107
no cycle; 107 nodes, 22 levels
orphans (no consumer): []
critical path to M1: 17 items, weight 32 (S=1,M=2,L=4): M1 <- M1a <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M2: 19 items, weight 38 (S=1,M=2,L=4): M2 <- P3-16 <- P3-18 <- P3-07 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M3: 20 items, weight 46 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M4: 19 items, weight 42 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
sizes: Counter({'M': 44, 'S': 42, 'L': 13})
```

Before the two Depends rewordings (A23's "before P0-04", E.1's "through P1-05, P1-19's …") the
same script reported `CYCLE among: […]` and a spurious direct edge; both are gone.

`r4/regen94.py` (writes §9.4 from §9.3; now also lists every path of maximal weight):

```
regenerated: 107 nodes (99 items and 8 milestones), 22 levels, orphans []
M1a: 17 nodes (15 items), weight 32: M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M1: 18 nodes (15 items), weight 32: M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M2: 20 nodes (18 items), weight 38: M2 ← P3-16 ← P3-18 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M3: 21 nodes (19 items), weight 46: M3 ← P4-07 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M4: 20 nodes (18 items), weight 42: M4 ← P5-02 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M5: 21 nodes (19 items), weight 48: M5 ← P6-01 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M6: 22 nodes (20 items), weight 50: M6 ← P7-04 ← P6-01 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
```

The two scripts print different members of the same tie (dagcheck's `max` picks `P0-09 ← P0-15`,
regen's dependency order picks `P0-05 ← P0-06`); §9.4 now lists all three P0 prefixes and both
post-P1-08 branches, so the plan no longer claims one of them is "the" path.

`review/mockwidth.py` (unchanged from round 2; run after the GiB re-padding and the captions):

```
L2361 [OK] '5.3 Module split': no declared width, max 111, box widths []
L2388 [OK] '5.4 One action table': no declared width, max 83, box widths []
L2507 [OK] 'Torrents at 80×24 (narrow):': declared 80, max 80, box widths [80]
L2523 [OK] 'Torrents at 120×30 (medium):': declared 120, max 119, box widths [116]
L2543 [OK] 'Details modal at 80×24 (Enter on the narrow layout):': declared 80, max 80, box widths [80]
L6536 [OK] "Popup, idle with a ready-to-watch row (owner's theme; keyboard mode, s": no declared width, max 54, box widths [54]
L6554 [OK] 'Popup, waiting for the VPN, with a confirm strip open (keyboard mode):': no declared width, max 54, box widths [54]
L6572 [OK] 'Popup, daemon not running (with `showWhenStopped`):': no declared width, max 54, box widths [54]
L6584 [OK] 'Dashboard, Search view with the poster grid (empty query, `v` grid):': no declared width, max 113, box widths [113]
L6604 [OK] 'Dashboard, Search view (rows):': no declared width, max 113, box widths [113]
L6620 [OK] 'Dashboard at 900 px (chip-row tier, §4.3.3: SIZE, PROGRESS, ↓, ETA, PE': no declared width, max 90, box widths [90]
L6641 [OK] 'Dashboard, remove dialog (`ConfirmDialog`, `selectedIndex: 0`):': no declared width, max 90, box widths []
L6658 [OK] 'Dashboard, add sheet:': no declared width, max 67, box widths []
L6674 [OK] 'TUI Search at 80×24 (narrow), card hidden:': declared 80, max 80, box widths [80]
L6691 [OK] 'TUI Shows at 80×24 (narrow), one pane with breadcrumb:': declared 80, max 80, box widths [80]
15 blocks measured, 0 need fixing
```

Counts: PLAN.md 6,891 lines (6,688 after round 3; +203); 101 IDs (99 active items, P7-07 and P7-08 superseded) and 8 milestones; no ID added
or removed; sizes S 42, M 44, L 13 (unchanged); critical path 32 S-units to M1a and M1 (31 in
round 3; P0-02 joined the path); 69 decisions (DL-66 to DL-69 new); 19 risks (R13, R18, R19
reworded); 31 owner questions (Q30 to Q33 new).

### Validation loop (planning-workflow)

- Self-containment: the review's four probes (P1-06, P1-08, P3-09, P4-04) found 0 of 4
  implementable as written and 2 of 4 buildable from the cited anchors; every real gap is now in
  the item or in the canonical section it cites (E hunks plus A12 to A16). Twelve items have been
  probed over rounds 3 and 4; the pattern is stable: the item text plus its anchors suffices, the
  item text alone never does, which the §9.3 preamble says is by design.
- Dependency graph: regenerated after every edit; no cycle, no orphan; two parenthetical IDs that
  the parser would have read as edges were removed, and the generator now prints ties.
- Justification: the round's decisions are DL-66 to DL-69 with their sections; the deferred cut
  (B4) and the assumed answers (A9) are visible in §0.3, §3.2.10, §10.2 and §10.3.
- Steady state: **reached**, in the reviewer's judgment and in this integrator's. Round 4's
  structural findings were gaps (an ordering edge, five protocol holes, one install script), not
  redesigns; no section was restructured except §0; the diff is a third of round 3's. What remains
  open is the owner's (Q1/Q31, Q3/Q30, Q32, Q33) and changes defaults, not architecture. Next step:
  conversion to beads, with Q3 and Q30 asked first.

### Still not grounded (carried in §10.4)

Round 3's list, less the two items this round settled by reading the fork (`finished_notify` is
the completion signal; `state_change_notify` fires before the `Error` assignment at `:333`), plus:
whether the Asupersync rider's "available to" clause bears on agents reading the vendored tree and
building the binary (Q30; the plan draws no legal conclusion and proceeds under the owner's
default until told otherwise).

## Round 5 (verification and prose), 2026-10-18

Input: the round-4 `PLAN.md` (6,891 lines), declared at steady state by its author and by the
round-4 reviewer. Brief: a final verification and copy-editing pass; polish-level edits only
(typos, grammar, stale or broken cross-references, stale counts, em-dashes in prose, slop
vocabulary); no change to technical meaning, numbers, decisions or structure; §9.4 not edited by
hand; mockup widths unchanged; structural findings reported, not fixed. Only `PLAN.md`,
`README.md` and this file were touched (`git status` confirms nothing under `src/`, `plugin/`,
`vendor/` or `Cargo.*` moved).

Method: the whole plan read once, section by section, with §0, §3.2, §3.4, §9.3's preamble,
Appendix A and Appendix F read against each other for the round-4 integration points (the `error`
event, `since_seq`, `subs: 16`, the coalesced set, the 1 MiB line cap, `finished_notify` and
`Ledger.generation`, the client kinds, the bulk `X`, the `--test-proto`/`--detach`/`ping` cuts, the
contract-1.x sentinels, GiB/MiB, `LICENSE`/`NOTICE`, the Torznab caps, P0-02 before P0-04); a new
script `review/idcheck.py` (session scratchpad) resolves every `P*`, `M*`, `DL-*`, `R*`, `Q*`,
`W*`, `D*` and `A*` token in prose against its definition and every `§x.y.z`, `Appendix X` and
`A.n`/`E.n` reference against the headings; `review/mockwidth.py` and `r3/dagcheck.py` ran before
and after the edits; a word list (robust, seamless, leverage, polish, delve, pivotal, "serves as",
"here's why", trailing "-ing" commentary, and so on) was grepped over the prose. Edits were
applied by exact-once string matching with a backup at `review/PLAN.before-r5.md`.

### Counts

- `PLAN.md`: 13 hunks, 48 changed lines, 6,891 → 6,898 lines. By kind: 4 cross-reference fixes
  (one wrong section number, two stray review-hunk ids an implementer would read as Appendix A
  references, one "names" that should be "describes"); 5 stale statements aligned with the
  canonical text they contradicted; 2 prose edits (one rhetorical question recast, one "simply"
  removed); 2 bookkeeping lines (the header's round note and a §0.7 bullet for this round).
- Typos and grammar: none found. Slop vocabulary: one hit (`simply`), removed; the two "marks a"
  are literal uses ("marks a claim", "marks a completed torrent") and stay.
- Em-dashes: 0 replaced. The 20 lines that still carry `—` outside code blocks are 4 quoted UI
  placeholder strings (`"—"` as the unknown-value glyph, §4.2.1, §4.2.3, §4.3.2, P2-08, P4-04) and
  16 table cells meaning "none" (9 in the generated §9.4, 5 in A.7's Key column, 2 in E.2's Needs
  column). They are literal content, not prose punctuation, and the A.7/E.2 cells were left as
  they are so the hand-written tables keep the convention the generated §9.4 uses.
- `README.md`: 2 bullets extended (PLAN.md and review-log.md) to name this round; every file and
  folder in `docs/plan-2026-10-18/` is indexed and "Read PLAN.md first" is stated.
- §9.4: not edited; `dagcheck.py` reports no disagreement between §9.3 and §9.4 after the edits.

### Corrections (before → after)

| Where | Before | After | Why |
|---|---|---|---|
| Preamble (line 25) | "§10.5 lists them all" | "§10.4 lists them all" | §10.4 is "Unverified claims, and the item that verifies each"; §10.5 is the process note |
| §3.2.11 bullet 1 | "systemd's default start limit (5 starts in 10 s) stops a crash loop" | "the unit's start limit (20 starts in 60 s, §3.2.2) stops a crash loop" | §3.2.2's unit sets `StartLimitIntervalSec=60`, `StartLimitBurst=20` (DL-45) and §3.2.11's own third bullet says "the unit allows 20 starts per minute" |
| §3.8 notification table, row "added", Actions | "`undo` (remove, keep files), `window` (open dashboard)" | the same, plus "the daemon posts these only for `origin: handler`, and the CLI's TTY toast is plain, without actions (P1-13, Q33)" | round 4's A5 changed P1-13, Q33 and DL-66 but not this table, which read as if the TTY toast carried actions |
| §6.3 row "Added notification with Undo", Daemon column | "after `torrents.add` from the handler or CLI" | "after `torrents.add` with `origin: "handler"` (the CLI's TTY toast is plain, without Undo, P1-13)" | same A5 propagation miss |
| §4.18 row "Table anatomy" | "`Color.muted` for unknown values" (borrowed from omaqbt) | "one dim colour for unknown values (omaqbt's `Color.muted` itself is not adopted, §4.2.1)" | §4.2.1 "Unknown marker" says `Color.muted` was not adopted |
| §10.5 | "This is round 3. Per the planning workflow it should go through at least one more review round …" | "Four review rounds are done (§0.7) and round 4 judged the plan at steady state … The next step is to convert §9 into beads …" (same validation-loop sentence, round 4's sampling added) | contradicted §0.7's steady-state verdict; the round number was two rounds stale |
| P1-07 Do | "P1-03 the first daemon cases (A10), P1-06 the NDJSON client helper" | "P1-03 the first daemon cases, P1-06 the NDJSON client helper" | "A10" is a round-3 review hunk id, not Appendix A.10 (field provenance) |
| E.2 row `qs-store.sh` | "(every `plugin/tests/` script lives outside the symlinked plugin folder, A13)" | "(…, §4.1)" | "A13" is a review hunk id; §4.1 is where the rule lives |
| P4-04 Do | "the one `state_glyph_width_one` test, which §5.5.3 names" | "… which §5.5.3 describes" | §5.5.3 describes the width check but names only `format::tests::states` |
| §0.1, after the four points | "Is there a grounded reason to drop the TUI (option "Quickshell only")? The decision matrix puts it one point behind …" | "The grounded case for dropping the TUI (option "Quickshell only") is thin: the decision matrix puts it one point behind …" | rhetorical question recast as a statement; content unchanged |
| §3.8 Notifier | "simply never acts" | "never acts" | filler |
| Header; §0.7 | round note; new bullet "Round 5 (verification and prose) …" | | bookkeeping |

Not changed, on purpose: `Q7`/`Q8` appear once, in §10.3, as the questions round 1 removed (the
checker lists them as retired, not unresolved); DL-23's "Superseded by DL-31" points at a live
decision; P7-07 and P7-08 carry "Superseded (round n): removed as ceremony" with no pointer,
which is the plan's stated convention for removed items.

### Round-4 regression check (§0, §3.2, §3.4, §9.3 preamble, Appendix A, Appendix F)

Consistent across every location checked: the request-less `error` event (A.1, A.8, P1-05, P1-06);
`since_seq`/`oldest_seq` (A.6, A.7, A.8, P1-08, P1-18, DL-41, DL-67); `subs: 16` (§3.4.3, §3.4.10,
P1-06, A.6); the closed coalesced set (§3.4.7, P1-08); the 1 MiB line cap with `torrents.list` and
`torrents.files` uncapped and banned in `plugin/` (§3.4.1, §4.4 rule 5, P2-05, DL-67);
`finished_notify` and `Ledger.generation` plus `Condvar` (§3.4.7, P1-08, P1-19); client kinds
`cli|agent|tui|bar|test` with no `dashboard` kind (§3.4.3, §4.4 rule 8, A.2, A.4, P1-11); the bulk
`X` as a window `Shortcut` (§4.12, Appendix B); `serve` flags `--idle-exit-secs` and `--no-notify`
only (§3.2.1, P1-03, Appendix F) and the `proto: 2` fixture frame (§3.2.1, P1-05); no `--detach`,
`daemon.log` or `last-daemon.json` (§3.2.14, A.2 `launcher`, DL-68); no `ping` (A.4, P1-06); the
contract-1.x sentinels (P1-11, DL-69, Appendix F `list`); GiB/MiB in prose and mockups with
decimal MB/s for rates (§4.2.3, §5.6, Appendix C); `LICENSE` and `NOTICE` in P0-03 (DL-69); Torznab
4 MiB and 30 s (§6.3, §7.3, P7-01, DL-69); P0-02 after P0-01 (P0-02 Depends, §8.1, §9.4, R19).
The two statements that still disagreed were the §3.8 and §6.3 "Added" rows above (A5 propagation).

### §0 against §9.4 and §10

99 items (13 L, 44 M, 42 S), 8 phases, 8 milestones, 107 nodes, 22 levels, no cycle, no orphan,
P7-07 and P7-08 superseded; critical path 32 S-units to M1a and M1 with the two 9-unit branches
(P1-12 → P1-17 → P1-11 → P1-15 = 2+1+4+2; P2-01 → P2-02 → P2-03 → P2-05 = 2+2+4+1); M2 38, M3
46, M4 42, M5 48, M6 50; per-phase item counts 16/19/8/18/11/7/13/7 = 99; DL-01 to DL-69; R1 to
R19 (19 rows); 31 questions (Q1 to Q6, Q9 to Q33). Every one of these in §0.5 and §0.6 matches
§9.2, §9.4, §9.7 and §10.1 to §10.3.

### Script outputs (final state)

`review/idcheck.py` (new this round):

```
definitions: A 9, D 18, DL 69, M 8, P 101, Q 31, R 19, W 8
retired Q7: mentioned as removed at lines [6079]
retired Q8: mentioned as removed at lines [6079]
superseded-by pointer: superseded by …"
superseded-by pointer: Superseded by DL-31 (round 3): one ladder for both surfaces
0 unresolved references; 265 distinct IDs referenced; 133 distinct section refs
```

`review/mockwidth.py` (unchanged script; run after the edits):

```
L2367 [OK] '5.3 Module split': no declared width, max 111, box widths []
L2394 [OK] '5.4 One action table': no declared width, max 83, box widths []
L2513 [OK] 'Torrents at 80×24 (narrow):': declared 80, max 80, box widths [80]
L2529 [OK] 'Torrents at 120×30 (medium):': declared 120, max 119, box widths [116]
L2549 [OK] 'Details modal at 80×24 (Enter on the narrow layout):': declared 80, max 80, box widths [80]
L6543 [OK] "Popup, idle with a ready-to-watch row (owner's theme; keyboard mode, s": no declared width, max 54, box widths [54]
L6561 [OK] 'Popup, waiting for the VPN, with a confirm strip open (keyboard mode):': no declared width, max 54, box widths [54]
L6579 [OK] 'Popup, daemon not running (with `showWhenStopped`):': no declared width, max 54, box widths [54]
L6591 [OK] 'Dashboard, Search view with the poster grid (empty query, `v` grid):': no declared width, max 113, box widths [113]
L6611 [OK] 'Dashboard, Search view (rows):': no declared width, max 113, box widths [113]
L6627 [OK] 'Dashboard at 900 px (chip-row tier, §4.3.3: SIZE, PROGRESS, ↓, ETA, PE': no declared width, max 90, box widths [90]
L6648 [OK] 'Dashboard, remove dialog (`ConfirmDialog`, `selectedIndex: 0`):': no declared width, max 90, box widths []
L6665 [OK] 'Dashboard, add sheet:': no declared width, max 67, box widths []
L6681 [OK] 'TUI Search at 80×24 (narrow), card hidden:': declared 80, max 80, box widths [80]
L6698 [OK] 'TUI Shows at 80×24 (narrow), one pane with breadcrumb:': declared 80, max 80, box widths [80]
15 blocks measured, 0 need fixing
```

`r3/dagcheck.py` (identical to round 4's output):

```
items in §9.3: 109 (active 107); rows in §9.4: 107
no cycle; 107 nodes, 22 levels
orphans (no consumer): []
critical path to M1: 17 items, weight 32 (S=1,M=2,L=4): M1 <- M1a <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M2: 19 items, weight 38 (S=1,M=2,L=4): M2 <- P3-16 <- P3-18 <- P3-07 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M3: 20 items, weight 46 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M4: 19 items, weight 42 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
sizes: Counter({'M': 44, 'S': 42, 'L': 13})
```

### Structural findings

None that need another review round. One wording question for the owner to settle when the beads
are created, because it decides what the owner must do before P0-04 and the plan says it two ways:
§0.6 says "Q3 and Q30 … must be answered before P0-04 commits C3", R19 says P0-02 "waits on the
owner twice", while P0-02's Depends and DL-58 call Q3 "the only owner action before the landing
commit of C3" and §0.3 A9 lets the plan proceed under Q30's default. Either Q30 gates C3 (then
P0-02 and DL-58 should say so) or it does not (then §0.6 should say "Q3 gates; Q30 is asked at the
same time"). This round did not change either text, since which reading holds is the owner's call.

### Validation loop (planning-workflow)

- Self-containment: unchanged from round 4 (twelve items probed over rounds 3 and 4); this round
  added no item text beyond the three one-phrase alignments above.
- Dependency graph: `dagcheck.py` before and after the edits is identical; §9.4 untouched.
- Justification: the five statements this round aligned each cite the section that justifies the
  canonical version (DL-45, DL-66/Q33, §4.2.1, §0.7).
- Steady state: **confirmed**. The round-4 → round-5 diff is 48 lines of cross-references, stale
  sentences and wording, with no decision, number or structure changed; that is the "typo-level
  polish" the planning workflow names as the stopping condition. The plan is ready for conversion
  to beads (99 items and 8 milestones with the §9.4 edges), with Q3 and Q30 asked first, as §0.6
  says.

## Round 6 (owner answers), 2026-10-18

Input: the owner's answers, verbatim, relayed by the coordinator: (1) on Q3 and Q30, "who cares,
continue"; (2) on Q1, "I don't want running processes. When we close the toolbar widget, it stops,
and is done." Brief: fold them into `PLAN.md` in place (only `PLAN.md`, `review-log.md`,
`README.md`; IDs stable; nothing under `src/`, `plugin/`, `vendor/`, `Cargo.*`; no git state
commands; `dagcheck.py`, `regen94.py` and `mockwidth.py` clean). Encoding, as briefed: Q30
accepted and recorded (the plan still draws no legal conclusion); Q3's default accepted (public
fork, full tree as the fallback) with an explicit owner checkpoint in P0-02 before anything is
pushed and a fallback path that runs without asking again; Q1 as one stop rule (lit icon = running
process; the power button, `down` and `Q` stop everything; closing windows never does; the idle net
stays at 300 s, `0` disables it; `last-ui` removed, `explicit` superseded, the `stop_policy` enum
and its UI branches deleted); Q2 answered with it (hidden when stopped); Q31 answered with it.
Q4 to Q29 and Q32, Q33 unchanged.

Method: `r6/owner_answers.py` applied 58 exact-once edits (55 string pairs and row rewrites keyed
on the row prefix, two block rewrites for §3.2.10 and P1-12, the DL-70 row), then 7 follow-up pairs; `r4/regen94.py` and `r3/dagcheck.py` ran after
(no edge changed, so §9.4 is byte-identical); `review/mockwidth.py` ran last. Backups:
`r6/PLAN.before-r6.md`, `r6/review-log.before-r6.md`, `r6/README.before-r6.md`.

### The stop rule as now written

§3.2.10 "Stop rule and idle exit (owner decision A1)": "The owner's rule ("I don't want running
processes. When we close the toolbar widget, it stops, and is done."): the daemon runs while the
bar icon is lit and stops when the owner stops it, from the bar icon's power button (§4.5), from
`fbittorrent down` or from `Q` in the TUI, each sending `app.shutdown {reason: "user"}`; the popup
and the TUI confirm first when transfers are active. Nothing else stops it: closing the TUI or the
dashboard window, dismissing the popup and losing the bar connection never stop a transfer. The
idle net below exits the daemon by itself when nothing is left to do, so a forgotten daemon does
not run for days. There is no `stop_policy` setting (the former `last-ui` and `explicit` policies
are gone, DL-70); the one knob is `idle_exit_secs`."

§0.4, the user-visible sentence: "The torrent icon in the bar is lit while fbittorrent runs; click
its power button and it stops, and is done."

One judgment call, recorded here: `fbittorrent down` does not prompt (typing the command is the
deliberate act, and agents call it); the confirm when transfers are active belongs to the popup's
power button and the TUI's `Q`, as the plan already specified, never to the daemon.

### Every changed location

| Where | Change |
|---|---|
| Header | Round 6; rounds 4 and 5 named as preceding |
| §0.3 A1 | Rewritten around the owner's words: one stop rule, lit icon = running, power button/`down`/`Q`, windows never stop transfers, the bar never keeps the daemon alive, idle net 300 s with `0` off, no `stop_policy` (Q1, Q2, Q31, DL-70) |
| §0.3 A9 | Q30 recorded as answered ("who cares, continue"); Q32 and Q33 remain the two assumed defaults |
| §0.4 | The "Closing the TUI…" row replaced by the owner-recognisable sentence plus the `down`/`Q` and idle-net clauses |
| §0.5 | P2 → M1 row names the power button; the demo paragraph ends with the power button or the idle net |
| §0.6 | DL-01 to DL-70; questions: 31 asked, five answered (Q1, Q2, Q3, Q30, Q31), 26 open, none gates work |
| §0.7 | Round 6 bullet |
| §2.2 principle 5 | "stops when the owner clicks the icon's power button (or runs `down`, or presses `Q`)" |
| §2.6 | `q` never asks (closing the TUI never stops a transfer); the one-time notice names the power button; "the idle net counts it as interactive" |
| §3.2.7 | Supervisor box "idle net"; the 5 s loop evaluates "the busy table"; the parked note says "busy table" |
| §3.2.10 | Retitled; the owner's-rule paragraph; the busy table loses its `last-ui` column; bullets "The idle net" (`0` disables) and "The owner's stop"; the `last-ui`/`explicit` bullets and the round-4 deferral note removed |
| §3.3 | `src/daemon/policy.rs`: "busy table evaluation and the idle countdown" |
| §3.4.3 | `hello` example `"policy":{"idle_exit_secs":300}` |
| §4.2.4 | Glyph row "power (stop the daemon, A1)" |
| §4.5 | Icon table: the stopped-recently tooltip is reason-aware (`bye.reason`); the visibility paragraph records Q2 as answered; the **Quit** bullet is now **Stop**: the power icon `󰐥`, `app.shutdown {reason: "user"}` at once with nothing active, the inline strip "Stop fbittorrent? 2 downloads pause until it starts again." [Cancel] [Stop] with Cancel focused when transfers are active, the dim 10 s and the icon leaving the bar |
| §5.7, P4-07 | F6 "gains save path, seed default, idle exit" (was "stop policy") |
| §8.2 | "Decision (the owner, 2026-10-18, Q3): the public fork. Option B is the runnable fallback if the repository is not created" |
| P0-02 | Do: Q3/Q30 answered; **Checkpoint before anything is pushed** (owner creates the repository or says "fallback"; asked once, in one message, proceeds on either answer); **Option B, the runnable fallback** spelled out (C0, `path =` unchanged, tests in place, runner files deleted, no archive, receipt, offline build acceptance). Depends: the checkpoint replaces "blocked on Q3" |
| P1-12 | Retitled "Stop rule and idle exit"; the one key `idle_exit_secs` with `0`; no `stop_policy`, `last-ui` or `explicit`; the owner's stop is P1-03's drain, never gated; Accept and Tests: `last_ui`, `explicit_never` → `disabled_never_exits`, `disabled_shutdown_drains`; unit `zero_disables` |
| P1-15 | `Q` sends `app.shutdown {reason: "user"}`; `q` never asks; "the idle net counts it as interactive" |
| P2-03 | Do: `Q` and the power icon as the owner's off switch with the §4.5 strip; Accept: the strip with Cancel focused, Stop drains, `running: false`, icon gone within 10 s; Tests: W-016 |
| P3-05 | Live settings list without `stop_policy` |
| P3-15 | `Dropdown` for `peer_encryption`, `handler` (no `stop_policy`) |
| §9.7 | M1a demo step (7): `fbittorrent down` stops at once, `up` resumes; M1 demo new step 9 (the power icon, the strip, Enter cancels, Stop drains, the icon leaves, `up` resumes), steps 9 and 10 renumbered 10 and 11 |
| §10.1 | R5 mitigation names the owner's rule and the power button; R19 narrowed again ("mostly closed round 6", low likelihood): one owner act, the checkpoint |
| §10.2 | DL-02 superseded by DL-70; DL-13 confirmed (Q2); DL-58 and DL-68 updated; **DL-70** new (the owner's words and the encoding of Q1, Q2, Q3, Q30, Q31) |
| §10.3 | Intro names the five answered questions; Q1, Q2, Q3, Q30, Q31 rows marked **answered 2026-10-18** with the owner's words and the encoding; the question text kept |
| §10.5 | "On 2026-10-18 the owner answered the questions that gated work … nothing blocks the next step" |
| A.2 | `policy: {idle_exit_secs: u64}` (0 means the idle net is off) |
| A.9 | `Settings` without `stop_policy`; `idle_exit_secs` documents `0` |
| Appendix B | "Stop fbittorrent (the daemon and everything in it, A1)" row; the popup's power icon `󰐥` |
| Appendix D | "Interactive client … under the idle net"; "Stop policy" → "Stop rule" |
| Appendix F | `settings` without `--stop-policy`; `down` row: `app.shutdown {reason: "user"}`, no prompt |

Not changed, on purpose: `showWhenStopped` stays as an off-by-default setting (a user-facing
option, not machinery; the owner's rule holds by default and DL-13 says so); the glance pause (an
open popup pauses the countdown only while it is open, §3.2.10, so the widget cannot keep the
daemon alive once it closes); the `bye.reason` enum (`shutdown` already covers the owner's stop);
Q4 to Q29, Q32, Q33; every item ID and every §9.4 edge.

### Script outputs (final state)

`r3/dagcheck.py`:

```
items in §9.3: 109 (active 107); rows in §9.4: 107
no cycle; 107 nodes, 22 levels
orphans (no consumer): []
critical path to M1: 17 items, weight 32 (S=1,M=2,L=4): M1 <- M1a <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M2: 19 items, weight 38 (S=1,M=2,L=4): M2 <- P3-16 <- P3-18 <- P3-07 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M3: 20 items, weight 46 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
critical path to M4: 19 items, weight 42 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-11 <- P1-17 <- P1-12 <- P1-08 <- P1-06 <- P1-05 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-02 <- P0-01
sizes: Counter({'M': 44, 'S': 42, 'L': 13})
```

`r4/regen94.py` (§9.4 regenerated; no edge changed, so the section is byte-identical to round 5):

```
regenerated: 107 nodes (99 items and 8 milestones), 22 levels, orphans []
M1a: 17 nodes (15 items), weight 32: M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M1: 18 nodes (15 items), weight 32: M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
```

`review/mockwidth.py`: `15 blocks measured, 0 need fixing` (no mockup touched; the §3.2.7 box
line keeps its width, 81 cells, after "stop policy" became "idle net").

Counts: PLAN.md 6,898 → 6,954 lines (+56); 101 IDs and 8 milestones unchanged;
sizes S 42, M 44, L 13; critical path 32; 70 decisions (DL-70 new, DL-02 superseded); 19 risks;
31 questions asked, 5 answered, 26 open; §0 is 250 lines (241 after round 5; A1 grew by two lines
and §0.7 gained the round-6 bullet).

### Validation loop (planning-workflow)

- The answers changed defaults and deleted an enum; no architecture moved. The one new mechanism
  (the popup's power button as the owner's off switch) already existed as "Quit"; it gained its
  confirm text, its `app.shutdown {reason: "user"}` call and a test (W-016).
- Steady state holds. Nothing the owner has to answer blocks work: P0-02 asks once whether the
  repository exists or the fallback applies and proceeds either way. Next step: conversion to beads.
